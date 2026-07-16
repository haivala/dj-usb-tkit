//! File path utilities: sanitization, artwork/analysis export, pruning, manifest paths.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use super::super::SETTING_EXPORT_OWNED_FILES_PREFIX;
use super::super::usb_helpers::sanitize_text;
use super::super::usb_utils::{canonicalize_playlist_name, resolve_usb_side_path};
use super::super::usb_vendor_compat::{
    USB_ANALYSIS_PREFIX, USB_ARTWORK_DIR, USB_ARTWORK_PREFIX, USB_CONTENTS_PREFIX,
    USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR,
};
use super::{ExportManifest, ExportTrackData};
use crate::edb::{open_edb_rw, table_exists};
use crate::error::{BackendError, BackendResult};
use crate::metadata::{MAX_GRAPHEME_CLUSTER_CHARS, cap_grapheme_clusters, cap_script_diversity};
use crate::pdb_reader::parse_pdb;

pub fn sanitize_filename_component(value: &str) -> String {
    let normalized = sanitize_text(value)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let collapsed = normalized
        .split('_')
        .filter(|p| !p.trim().is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if collapsed.is_empty() {
        "untitled".to_string()
    } else {
        collapsed
    }
}

pub fn sanitize_contents_component(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "Unknown".to_string();
    }
    let replaced = trimmed
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            _ => c,
        })
        .collect::<String>();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    // Bound stacked combining marks ("zalgo" text) and how many unrelated
    // scripts a single name can mix, so on-disk file/folder names — and the
    // ANLZ PPTH chunk built from them — stay within what CDJ text rendering
    // can handle without hanging.
    let depth_capped = cap_grapheme_clusters(&collapsed, MAX_GRAPHEME_CLUSTER_CHARS);
    let capped = cap_script_diversity(&depth_capped).into_owned();
    if capped.is_empty() {
        "Unknown".to_string()
    } else {
        capped
    }
}

const CONTENT_COMPONENT_MAX_LEN: usize = 48;
pub const CONTENT_FILENAME_MAX_LEN: usize = 48;

pub fn truncate_component(value: &str, max_len: usize) -> String {
    let mut out = value.chars().take(max_len).collect::<String>();
    while out.ends_with(' ') || out.ends_with('.') {
        out.pop();
    }
    if out.is_empty() {
        "Unknown".to_string()
    } else {
        out
    }
}

pub fn limit_contents_file_name(file_name: &str, max_len: usize) -> String {
    if file_name.chars().count() <= max_len {
        return file_name.to_string();
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() => (s, Some(e)),
        _ => (file_name, None),
    };

    let ext_part = ext.map(|e| format!(".{e}")).unwrap_or_default();
    let ext_len = ext_part.chars().count();
    let reserve_ext = ext_len > 0 && ext_len + 1 < max_len;
    let reserved = if reserve_ext { ext_len } else { 0 };
    let keep = max_len.saturating_sub(reserved);
    let mut trimmed_stem = stem.chars().take(keep).collect::<String>();
    while trimmed_stem.ends_with(' ') || trimmed_stem.ends_with('.') {
        trimmed_stem.pop();
    }
    if trimmed_stem.is_empty() {
        return file_name.chars().take(max_len).collect::<String>();
    }
    if reserve_ext {
        format!("{trimmed_stem}{ext_part}")
    } else {
        trimmed_stem
    }
}

pub fn copy_if_different(source: &Path, target: &Path) -> BackendResult<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let should_copy = match (std::fs::metadata(source), std::fs::metadata(target)) {
        (Ok(src), Ok(dst)) => src.len() != dst.len(),
        (Ok(_), Err(_)) => true,
        (Err(err), _) => return Err(BackendError::Io(err)),
    };
    if should_copy {
        std::fs::copy(source, target)?;
    }
    Ok(())
}

/// Copies a WAV file to `target`, normalizing a WAVE_FORMAT_EXTENSIBLE header
/// to standard PCM/IEEE-float first if needed - some Pioneer CDJs reject
/// WAVE_FORMAT_EXTENSIBLE WAVs outright. The rewrite only touches the `fmt `
/// chunk (see `wav_format::rewrite_extensible_to_pcm`), so sample data is
/// never re-encoded. Falls back to a plain `copy_if_different` for anything
/// that isn't a safely-convertible extensible WAV (unreadable header, or an
/// extensible subformat other than PCM/float, which stays a hard warning).
pub fn copy_wav_normalized_if_needed(source: &Path, target: &Path) -> BackendResult<()> {
    let info = match crate::wav_format::parse_wav_fmt(source)? {
        Some(info) => info,
        None => return copy_if_different(source, target),
    };
    if crate::wav_format::classify(&info) != Some(crate::wav_format::WavFormatIssue::ExtensiblePcm)
    {
        return copy_if_different(source, target);
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // The rewrite shrinks the fmt chunk to a standard 16-byte body; use that
    // to compute the expected output size and skip re-writing an up-to-date
    // target, mirroring copy_if_different's skip-if-same-size check.
    let old_fmt_chunk_len = 8 + u64::from(info.fmt_chunk_size) + u64::from(info.fmt_chunk_size % 2);
    let new_fmt_chunk_len = 8 + 16u64;
    let delta = old_fmt_chunk_len.saturating_sub(new_fmt_chunk_len);
    let expected_len = std::fs::metadata(source)?.len().saturating_sub(delta);
    let should_write = match std::fs::metadata(target) {
        Ok(dst) => dst.len() != expected_len,
        Err(_) => true,
    };
    if should_write {
        crate::wav_format::rewrite_extensible_to_pcm(source, target)?;
    }
    Ok(())
}

pub fn stable_u32_hash(input: &str) -> u32 {
    let mut hash: u32 = 0x811C9DC5;
    for byte in input.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Player artwork sizes: 80x80 thumbnail + 240x240 medium, both square JPEG.
const PLAYER_ART_SMALL: u32 = 80;
const PLAYER_ART_MEDIUM: u32 = 240;

/// Returns (small_path, medium_path) for player artwork.
/// Small = `a{id}.jpg` (80x80), medium = `a{id}_m.jpg` (240x240).
pub fn canonical_artwork_target_paths(usb_root: &Path, track_id: &str) -> (PathBuf, PathBuf) {
    let numeric = track_id.trim().parse::<u32>().ok();
    let art_num = if let Some(n) = numeric {
        n
    } else {
        let hash = stable_u32_hash(track_id);
        hash % 100000
    };
    let bucket = "00001".to_string();
    let art_id = format!("b{art_num}");
    let dir = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_ARTWORK_DIR)
        .join(bucket);
    (
        dir.join(format!("{art_id}.jpg")),
        dir.join(format!("{art_id}_m.jpg")),
    )
}

/// Legacy single-path wrapper (used by tests and manifest path references).
#[cfg(test)]
pub fn canonical_artwork_target_path(
    usb_root: &Path,
    track_id: &str,
    _source_path: &str,
) -> PathBuf {
    canonical_artwork_target_paths(usb_root, track_id).0
}

/// Resize source artwork to player-compatible square JPEGs (80x80 and 240x240).
/// Center-crops to 1:1 aspect ratio before resizing.
/// Returns the small path on success.
pub fn export_artwork_for_player(
    source_path: &str,
    usb_root: &Path,
    track_id: &str,
    warnings: &mut Vec<String>,
) -> BackendResult<Option<String>> {
    let source = PathBuf::from(source_path);
    if !source.is_file() {
        warnings.push(format!("artwork missing: {}", source.display()));
        return Ok(None);
    }

    let img = match image::open(&source) {
        Ok(img) => img,
        Err(err) => {
            warnings.push(format!(
                "artwork decode failed for {}: {err}",
                source.display()
            ));
            return Ok(None);
        }
    };

    let (paths_small, paths_medium) = canonical_artwork_target_paths(usb_root, track_id);
    if let Some(parent) = paths_small.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Center-crop to square
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let x_offset = (w - side) / 2;
    let y_offset = (h - side) / 2;
    let cropped = img.crop_imm(x_offset, y_offset, side, side);

    // Resize to both player sizes and save as JPEG
    for (target_path, size) in [
        (&paths_small, PLAYER_ART_SMALL),
        (&paths_medium, PLAYER_ART_MEDIUM),
    ] {
        let resized = cropped.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
        let rgb = resized.to_rgb8();
        let mut buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85);
        if let Err(err) = rgb.write_with_encoder(encoder) {
            warnings.push(format!(
                "artwork encode failed for {}: {err}",
                target_path.display()
            ));
            continue;
        }
        std::fs::write(target_path, buf.into_inner())?;
    }

    // Return the small path (PDB/export DB references this one)
    Ok(Some(paths_small.to_string_lossy().to_string()))
}

// Re-export analysis helpers so they're available via export_helpers::*
pub use super::super::anlz::canonical_analysis_bundle_paths;
use super::super::anlz::ensure_ppth_chunk;

fn write_bytes_if_different(target: &Path, bytes: &[u8]) -> BackendResult<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let should_write = match std::fs::read(target) {
        Ok(existing) => existing != bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => return Err(BackendError::Io(err)),
    };
    if should_write {
        std::fs::write(target, bytes)?;
    }
    Ok(())
}

fn write_anlz_with_export_path(
    source: &Path,
    target: &Path,
    track_path: &str,
) -> BackendResult<()> {
    let source_bytes = std::fs::read(source)?;
    let bytes = ensure_ppth_chunk(&source_bytes, track_path);
    write_bytes_if_different(target, &bytes)
}

pub fn ensure_analysis_bundle_ppth(
    usb_root: &Path,
    analysis_path: &str,
    track_path: &str,
) -> BackendResult<()> {
    let dat_abs = resolve_usb_side_path(usb_root, analysis_path).ok_or_else(|| {
        BackendError::Validation(format!("invalid USB analysis path: {analysis_path}"))
    })?;
    let dat_path = PathBuf::from(dat_abs);
    let ext_path = dat_path.with_extension("EXT");
    let twoex_path = dat_path.with_extension("2EX");
    for path in [&dat_path, &ext_path, &twoex_path] {
        let bytes = std::fs::read(path)?;
        let with_ppth = ensure_ppth_chunk(&bytes, track_path);
        write_bytes_if_different(path, &with_ppth)?;
    }
    Ok(())
}

pub fn export_analysis_bundle_for_track(
    track: &ExportTrackData,
    usb_root: &Path,
    track_path: &str,
    warnings: &mut Vec<String>,
) -> BackendResult<Option<String>> {
    let (dat_path, ext_path, twoex_path) = canonical_analysis_bundle_paths(usb_root, track_path);

    let Some(local_dat_str) = track.waveform_peaks_path.as_deref() else {
        warnings.push(format!(
            "analysis bundle missing for track {} (no DAT path)",
            track.id
        ));
        return Ok(None);
    };

    let local_dat = Path::new(local_dat_str);
    let local_ext = local_dat.with_extension("EXT");
    let local_twoex = local_dat.with_extension("2EX");
    if !local_dat.is_file() || !local_ext.is_file() || !local_twoex.is_file() {
        warnings.push(format!(
            "analysis bundle missing for track {}: {}",
            track.id,
            local_dat.display()
        ));
        return Ok(None);
    }

    if let Some(parent) = dat_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_anlz_with_export_path(local_dat, &dat_path, track_path)?;
    write_anlz_with_export_path(&local_ext, &ext_path, track_path)?;
    write_anlz_with_export_path(&local_twoex, &twoex_path, track_path)?;
    Ok(to_usb_relative_path(usb_root, &dat_path.to_string_lossy())
        .or_else(|| Some(dat_path.to_string_lossy().to_string())))
}

pub fn exported_media_target_path(
    media_root: &Path,
    source: &Path,
    artist: &str,
    album: Option<&str>,
    title: &str,
    extension: &str,
) -> PathBuf {
    let parts = source
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if let Some(contents_index) = parts
        .iter()
        .position(|p| p.eq_ignore_ascii_case("Contents"))
    {
        let rel_parts = parts
            .into_iter()
            .skip(contents_index + 1)
            .collect::<Vec<_>>();
        if !rel_parts.is_empty() {
            return rel_parts
                .into_iter()
                .fold(media_root.to_path_buf(), |acc, part| acc.join(part));
        }
    }

    let raw_source_file_name = source
        .file_name()
        .and_then(|s| s.to_str())
        .map(sanitize_contents_component)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}.{}", sanitize_filename_component(title), extension));
    let artist_clean = sanitize_contents_component(artist);
    let album_clean = sanitize_contents_component(album.unwrap_or("UnknownAlbum"));
    let artist_dir = truncate_component(&artist_clean, CONTENT_COMPONENT_MAX_LEN);
    let album_dir = truncate_component(&album_clean, CONTENT_COMPONENT_MAX_LEN);
    let source_file_name =
        limit_contents_file_name(&raw_source_file_name, CONTENT_FILENAME_MAX_LEN);
    media_root
        .join(artist_dir)
        .join(album_dir)
        .join(source_file_name)
}

pub fn to_usb_relative_path(usb_root: &Path, absolute_or_raw: &str) -> Option<String> {
    let path = PathBuf::from(absolute_or_raw);
    let abs = if path.is_absolute() {
        path
    } else {
        usb_root.join(path)
    };
    let rel = abs.strip_prefix(usb_root).ok()?;
    Some(format!("/{}", rel.to_string_lossy().replace('\\', "/")))
}

pub fn export_owned_files_setting_key(usb_root: &Path, playlist_id: &str) -> String {
    let root_key = canonicalize_playlist_name(&usb_root.to_string_lossy());
    format!("{SETTING_EXPORT_OWNED_FILES_PREFIX}:{root_key}:{playlist_id}")
}

fn insert_if_owned_path(owned: &mut HashSet<String>, usb_root: &Path, path: &str) {
    if let Some(normalized) = normalize_owned_export_path(usb_root, path) {
        owned.insert(normalized);
    }
}

fn insert_with_medium_variant(owned: &mut HashSet<String>, normalized: String) {
    owned.insert(normalized.clone());
    let medium = normalized
        .replace(".jpg", "_m.jpg")
        .replace(".png", "_m.png");
    if medium != normalized {
        owned.insert(medium);
    }
}

fn protect_if_stale(
    protected: &mut HashSet<String>,
    stale_normalized: &HashSet<String>,
    normalized: String,
) {
    if stale_normalized.contains(&normalized) {
        protected.insert(normalized);
    }
}

fn protect_artwork_variants_if_stale(
    protected: &mut HashSet<String>,
    stale_normalized: &HashSet<String>,
    normalized: String,
) {
    protect_if_stale(protected, stale_normalized, normalized.clone());
    let medium = normalized
        .replace(".jpg", "_m.jpg")
        .replace(".png", "_m.png");
    protect_if_stale(protected, stale_normalized, medium);
}

fn protect_analysis_variants_if_stale(
    protected: &mut HashSet<String>,
    stale_normalized: &HashSet<String>,
    usb_root: &Path,
    analysis_path: &str,
) {
    for variant in analysis_bundle_path_variants(analysis_path) {
        if let Some(normalized) = normalize_owned_export_path(usb_root, &variant) {
            protect_if_stale(protected, stale_normalized, normalized);
        }
    }
}

fn as_usb_relative_path(trimmed: &str) -> String {
    if trimmed.starts_with('/') {
        trimmed.replace('\\', "/")
    } else {
        format!("/{}", trimmed.replace('\\', "/"))
    }
}

pub fn collect_manifest_owned_paths(usb_root: &Path, manifest: &ExportManifest) -> HashSet<String> {
    let mut owned = HashSet::<String>::new();
    for track in &manifest.tracks {
        if track.owns_exported_media {
            insert_if_owned_path(&mut owned, usb_root, &track.exported_path);
        }
        if track.owns_artwork
            && let Some(path) = track.artwork_path.as_deref()
            && let Some(normalized) = normalize_owned_export_path(usb_root, path)
        {
            // Also claim the _m (medium) variant
            insert_with_medium_variant(&mut owned, normalized);
        }
        if track.owns_waveform
            && let Some(path) = track.waveform_path.as_deref()
        {
            for bundle_path in analysis_bundle_path_variants(path) {
                if let Some(normalized) = normalize_owned_export_path(usb_root, &bundle_path) {
                    owned.insert(normalized);
                }
            }
        }
    }
    owned
}

pub fn filter_prunable_stale_paths_for_playlist(
    usb_root: &Path,
    playlist_name: &str,
    stale_paths: &[String],
    warnings: &mut Vec<String>,
) -> BackendResult<Vec<String>> {
    let stale_normalized = stale_paths
        .iter()
        .filter_map(|path| normalize_owned_export_path(usb_root, path))
        .collect::<HashSet<_>>();
    if stale_normalized.is_empty() {
        return Ok(Vec::new());
    }

    let mut protected = HashSet::<String>::new();
    let wanted_name = canonicalize_playlist_name(playlist_name);

    let mut unlock_warnings = Vec::<String>::new();
    if let Some(conn) = open_edb_rw(usb_root, &mut unlock_warnings) {
        if table_exists(&conn, "playlist")
            && table_exists(&conn, "playlist_content")
            && table_exists(&conn, "content")
        {
            let mut current_playlist_ids = HashSet::<i64>::new();
            let mut stmt = conn.prepare("SELECT playlist_id, name, attribute FROM playlist")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2).unwrap_or(0),
                ))
            })?;
            for row in rows {
                let (playlist_id, name, attribute) = row?;
                if attribute == 0 && canonicalize_playlist_name(&name) == wanted_name {
                    current_playlist_ids.insert(playlist_id);
                }
            }

            let sql = if table_exists(&conn, "image") {
                r#"
                SELECT pc.playlist_id, c.path, c.analysisDataFilePath, img.path
                FROM playlist_content pc
                JOIN content c ON c.content_id = pc.content_id
                LEFT JOIN image img ON img.image_id = c.image_id
                "#
            } else {
                r#"
                SELECT pc.playlist_id, c.path, c.analysisDataFilePath, NULL AS image_path
                FROM playlist_content pc
                JOIN content c ON c.content_id = pc.content_id
                "#
            };
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            for row in rows {
                let (playlist_id, content_path, analysis_path, image_path) = row?;
                if current_playlist_ids.contains(&playlist_id) {
                    continue;
                }
                if let Some(path) = content_path
                    .as_deref()
                    .and_then(|p| normalize_owned_export_path(usb_root, p))
                {
                    protect_if_stale(&mut protected, &stale_normalized, path);
                }
                if let Some(path) = analysis_path.as_deref() {
                    protect_analysis_variants_if_stale(
                        &mut protected,
                        &stale_normalized,
                        usb_root,
                        path,
                    );
                }
                if let Some(path) = image_path
                    .as_deref()
                    .and_then(|p| normalize_owned_export_path(usb_root, p))
                {
                    protect_artwork_variants_if_stale(&mut protected, &stale_normalized, path);
                }
            }
        }
    } else {
        warnings.extend(unlock_warnings);
    }

    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    if pdb_path.is_file() {
        let parsed = parse_pdb(&pdb_path)?;
        let current_playlist_ids = parsed
            .playlist_tree
            .iter()
            .filter(|row| {
                !row.row_is_folder && canonicalize_playlist_name(&row.name) == wanted_name
            })
            .map(|row| row.id)
            .collect::<HashSet<_>>();
        let track_by_id = parsed
            .tracks
            .iter()
            .map(|track| (track.id, track))
            .collect::<HashMap<_, _>>();

        for entry in &parsed.playlist_entries {
            if current_playlist_ids.contains(&entry.playlist_id) {
                continue;
            }
            let Some(track) = track_by_id.get(&entry.track_id).copied() else {
                continue;
            };
            if let Some(path) = normalize_owned_export_path(usb_root, &track.track_file_path) {
                protect_if_stale(&mut protected, &stale_normalized, path);
            }
            protect_analysis_variants_if_stale(
                &mut protected,
                &stale_normalized,
                usb_root,
                &track.anlz_path,
            );
            if let Some(art_path) = parsed.artworks.get(&track.artwork_id)
                && let Some(normalized) = normalize_owned_export_path(usb_root, art_path)
            {
                protect_artwork_variants_if_stale(&mut protected, &stale_normalized, normalized);
            }
        }
    }

    let mut prunable = stale_paths
        .iter()
        .filter_map(|path| {
            let normalized = normalize_owned_export_path(usb_root, path)?;
            if protected.contains(&normalized) {
                None
            } else {
                Some(path.clone())
            }
        })
        .collect::<Vec<_>>();
    prunable.sort();
    prunable.dedup();

    if !protected.is_empty() {
        warnings.push(format!(
            "prune stale protected {} shared file(s) still referenced by other USB playlists",
            protected.len()
        ));
    }

    Ok(prunable)
}

#[derive(Debug, Clone, Copy)]
pub struct PruneResult {
    pub removed: usize,
    pub missing: usize,
    pub skipped: usize,
}

pub fn prune_stale_export_owned_files(
    usb_root: &Path,
    stale_paths: &[String],
    warnings: &mut Vec<String>,
) -> BackendResult<PruneResult> {
    let mut removed = 0usize;
    let mut missing = 0usize;
    let mut skipped = 0usize;
    let mut ordered = stale_paths.to_vec();
    ordered.sort();

    for stale in ordered {
        let Some(normalized) = normalize_owned_export_path(usb_root, &stale) else {
            skipped += 1;
            warnings.push(format!("prune stale skipped invalid path: {stale}"));
            continue;
        };
        if !is_safe_export_owned_path(&normalized) {
            skipped += 1;
            warnings.push(format!("prune stale skipped unsafe path: {normalized}"));
            continue;
        }
        let Some(abs) = resolve_usb_side_path(usb_root, &normalized) else {
            skipped += 1;
            warnings.push(format!("prune stale skipped unresolved path: {normalized}"));
            continue;
        };
        let abs_path = PathBuf::from(&abs);
        if !abs_path.starts_with(usb_root) {
            skipped += 1;
            warnings.push(format!(
                "prune stale skipped outside usb root: {normalized}"
            ));
            continue;
        }
        if !abs_path.exists() {
            missing += 1;
            continue;
        }
        if abs_path.is_file() {
            std::fs::remove_file(&abs_path)?;
            removed += 1;
            continue;
        }
        skipped += 1;
        warnings.push(format!(
            "prune stale skipped non-file path: {}",
            abs_path.display()
        ));
    }

    Ok(PruneResult {
        removed,
        missing,
        skipped,
    })
}

pub fn normalize_owned_export_path(usb_root: &Path, path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    if Path::new(trimmed)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    if let Some(abs) = resolve_usb_side_path(usb_root, trimmed) {
        return to_usb_relative_path(usb_root, &abs)
            .or_else(|| Some(as_usb_relative_path(trimmed)));
    }
    None
}

pub fn is_safe_export_owned_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if Path::new(&normalized)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    normalized.starts_with(USB_CONTENTS_PREFIX)
        || normalized.starts_with(USB_ARTWORK_PREFIX)
        || normalized.starts_with(USB_ANALYSIS_PREFIX)
}

pub fn analysis_bundle_path_variants(path: &str) -> Vec<String> {
    let normalized = path.replace('\\', "/");
    let Some((stem, ext)) = normalized.rsplit_once('.') else {
        return vec![normalized];
    };
    let upper = ext.to_ascii_uppercase();
    if upper == "DAT" || upper == "EXT" || upper == "2EX" {
        return vec![
            format!("{stem}.DAT"),
            format!("{stem}.EXT"),
            format!("{stem}.2EX"),
        ];
    }
    vec![normalized]
}

#[cfg(test)]
mod tests {
    use super::normalize_owned_export_path;
    use tempfile::tempdir;

    #[test]
    fn normalize_owned_export_path_rejects_parent_traversal() {
        let temp = tempdir().expect("tempdir");
        assert_eq!(
            normalize_owned_export_path(temp.path(), "../secret.mp3"),
            None
        );
        assert_eq!(
            normalize_owned_export_path(temp.path(), "/Contents/Artist/../secret.mp3"),
            None
        );
    }

    #[test]
    fn normalize_owned_export_path_keeps_usb_relative_normalization_stable() {
        let temp = tempdir().expect("tempdir");
        let track_abs = temp.path().join("Contents/Artist/Album/track.mp3");
        std::fs::create_dir_all(track_abs.parent().expect("track parent")).expect("mkdirs");
        std::fs::write(&track_abs, b"audio").expect("write track");

        let from_relative =
            normalize_owned_export_path(temp.path(), "/Contents/Artist/Album/track.mp3")
                .expect("normalize relative");
        let from_absolute = normalize_owned_export_path(temp.path(), &track_abs.to_string_lossy())
            .expect("normalize absolute");

        assert_eq!(from_relative, "/Contents/Artist/Album/track.mp3");
        assert_eq!(from_absolute, from_relative);
    }
}
