//! USB utility functions: path resolution, export DB access, master DB access.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use walkdir::WalkDir;

use crate::edb::open_edb_from_usb_root;
use crate::error::{BackendError, BackendResult};
use crate::pdb_reader::parse_pdb;

use super::WAVEFORM_PREVIEW_BINS;
use super::usb_vendor_compat::{
    DEFAULT_USB_EDB_KEY, MASTER_DB_ENV_KEY, USB_ANALYSIS_DIR, USB_CONTENTS_DIR, USB_ROOT_ENV_KEY,
    USB_VENDOR_DB_DIR, USB_VENDOR_DB_DIR_LOWER, USB_VENDOR_ROOT_DIR, USB_VENDOR_ROOT_DIR_LOWER,
    USB_VENDOR_ROOT_PREFIX, desktop_master_db_rel_path, vendor_db_dir, vendor_pdb_path,
};

pub(crate) fn artwork_path_to_data_url(path: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())?;
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    };
    let bytes = std::fs::read(p).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let encoded = BASE64_STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

pub(crate) fn canonicalize_or_self(path: std::path::PathBuf) -> std::path::PathBuf {
    std::fs::canonicalize(&path).unwrap_or_else(|_| normalize_path_components(&path))
}

/// Resolve `.` and `..` components without touching the filesystem.
/// Used as fallback when `std::fs::canonicalize` fails (path doesn't exist).
fn normalize_path_components(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

pub(crate) fn normalize_usb_root_path(path: std::path::PathBuf) -> std::path::PathBuf {
    let lower_name = |p: &std::path::Path| {
        p.file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default()
    };

    let name = lower_name(&path);
    if name == USB_CONTENTS_DIR.to_ascii_lowercase() || name == USB_VENDOR_ROOT_DIR_LOWER {
        if let Some(parent) = path.parent() {
            return parent.to_path_buf();
        }
    }

    if name == USB_VENDOR_DB_DIR_LOWER {
        if let Some(parent) = path.parent() {
            if lower_name(parent) == USB_VENDOR_ROOT_DIR_LOWER {
                if let Some(root) = parent.parent() {
                    return root.to_path_buf();
                }
            }
        }
    }

    path
}

pub(crate) fn resolve_usb_root(requested_root: Option<&str>) -> BackendResult<std::path::PathBuf> {
    if let Some(requested_root) = requested_root {
        let trimmed = requested_root.trim();
        if !trimmed.is_empty() {
            let candidate = std::path::PathBuf::from(trimmed);
            if candidate.exists() {
                return Ok(normalize_usb_root_path(canonicalize_or_self(candidate)));
            }
            if candidate.is_relative() {
                let cwd = std::env::current_dir()?;
                let relative_candidates = [
                    cwd.join(trimmed),
                    cwd.join("..").join(trimmed),
                    cwd.join("../..").join(trimmed),
                    cwd.join("../../..").join(trimmed),
                ];
                for alt in relative_candidates {
                    if alt.exists() {
                        return Ok(normalize_usb_root_path(canonicalize_or_self(alt)));
                    }
                }
            }
            return Err(BackendError::NotFound(format!(
                "requested usbRoot does not exist: {}",
                candidate.display()
            )));
        }
    }

    if let Ok(override_path) = std::env::var(USB_ROOT_ENV_KEY) {
        let trimmed = override_path.trim();
        if !trimmed.is_empty() {
            let candidate = std::path::PathBuf::from(trimmed);
            if candidate.exists() {
                return Ok(normalize_usb_root_path(canonicalize_or_self(candidate)));
            }
            return Err(BackendError::NotFound(format!(
                "{USB_ROOT_ENV_KEY} is set but path does not exist: {}",
                candidate.display()
            )));
        }
    }

    let cwd = std::env::current_dir()?;
    let candidates = [
        cwd.join("USB"),
        cwd.join("../USB"),
        cwd.join("../../USB"),
        cwd.join("../../../USB"),
    ];
    for candidate in candidates {
        if candidate.exists() {
            return Ok(normalize_usb_root_path(canonicalize_or_self(candidate)));
        }
    }

    Err(BackendError::NotFound(
        "USB root not found; set usbRoot in request, DJUSBTKIT_USB_ROOT, or create project USB directory".to_string(),
    ))
}

pub(crate) fn resolve_usb_side_path(usb_root: &std::path::Path, raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.replace('\\', "/");
    if normalized.is_empty() {
        return None;
    }

    if normalized.starts_with("file://") {
        return Some(normalized);
    }

    let is_windows_abs = normalized.len() > 2
        && normalized.as_bytes().get(1) == Some(&b':')
        && normalized.as_bytes().get(2) == Some(&b'/');
    if is_windows_abs {
        return Some(normalized);
    }

    // Resolve relative paths under usb_root, then verify containment
    let canon_root = canonicalize_or_self(usb_root.to_path_buf());

    let looks_usb_relative = normalized.starts_with(USB_VENDOR_ROOT_PREFIX)
        || normalized.starts_with(format!("/{USB_CONTENTS_DIR}/").as_str())
        || normalized.starts_with(format!("{USB_VENDOR_ROOT_DIR}/").as_str())
        || normalized.starts_with(format!("{USB_CONTENTS_DIR}/").as_str());

    if looks_usb_relative {
        let rel = normalized.trim_start_matches('/');
        let resolved = canonicalize_or_self(usb_root.join(rel));
        if !resolved.starts_with(&canon_root) {
            return None; // path traversal attempt
        }
        return Some(resolved.to_string_lossy().to_string());
    }

    if std::path::Path::new(&normalized).is_absolute() {
        return Some(
            canonicalize_or_self(std::path::PathBuf::from(&normalized))
                .to_string_lossy()
                .to_string(),
        );
    }

    let resolved = canonicalize_or_self(usb_root.join(&normalized));
    if !resolved.starts_with(&canon_root) {
        return None; // path traversal attempt
    }
    Some(resolved.to_string_lossy().to_string())
}

pub(crate) fn has_write_access(root: &std::path::Path) -> bool {
    let probe_dir = root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
    let target = if probe_dir.is_dir() {
        probe_dir
    } else {
        root.to_path_buf()
    };

    // Keep validation lightweight: avoid create/write probes on potentially slow/corrupt USB media.
    std::fs::metadata(&target)
        .map(|m| !m.permissions().readonly())
        .unwrap_or(false)
}

pub(crate) fn load_waveform_preview_from_analysis_path(path: &str) -> Option<Vec<u8>> {
    let base = std::path::PathBuf::from(path);
    let mut candidates = Vec::<std::path::PathBuf>::new();
    let ext = base
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_default();

    if ext == "EXT" {
        candidates.push(base.clone());
        candidates.push(base.with_extension("2EX"));
        candidates.push(base.with_extension("DAT"));
    } else if ext == "2EX" {
        candidates.push(base.clone());
        candidates.push(base.with_extension("EXT"));
        candidates.push(base.with_extension("DAT"));
    } else if ext == "DAT" {
        candidates.push(base.with_extension("EXT"));
        candidates.push(base.with_extension("2EX"));
        candidates.push(base.clone());
    } else {
        candidates.push(base.with_extension("EXT"));
        candidates.push(base.with_extension("2EX"));
        candidates.push(base.with_extension("DAT"));
        candidates.push(base.clone());
    }

    for candidate in candidates {
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        if let Some(peaks) = extract_waveform_preview_from_anlz_bytes(&bytes, WAVEFORM_PREVIEW_BINS)
        {
            if !peaks.is_empty() {
                return Some(peaks);
            }
        }
    }
    None
}

/// Extract the raw PWV4 payload from a desktop library ANLZ `.EXT` file (no conversion).
/// Returns the 1200 × 6 byte payload, or None if the file/chunk is absent.
pub(crate) fn read_pwv4_from_anlz(dat_path: &str) -> Option<Vec<u8>> {
    let base = std::path::PathBuf::from(dat_path);
    let ext_path = if base
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ext"))
    {
        base
    } else {
        base.with_extension("EXT")
    };
    let bytes = std::fs::read(&ext_path).ok()?;
    let container = find_anlz_chunk_payload(&bytes, "PMAI").unwrap_or(&bytes);
    find_anlz_chunk_payload(container, "PWV4").map(|p| p.to_vec())
}

pub(crate) fn extract_waveform_preview_from_anlz_bytes(
    bytes: &[u8],
    bins: usize,
) -> Option<Vec<u8>> {
    if bytes.len() < 16 {
        return None;
    }

    let container = find_anlz_chunk_payload(bytes, "PMAI").unwrap_or(bytes);
    // Prefer single-byte-per-level formats (PWV3, PWAV, PWV2) because
    // downsample_waveform_payload extracts amplitude via `b & 0x1F` per byte.
    // Multi-byte formats (PWV5=2B, PWV4=6B per level) need dedicated decoders
    // and would produce garbage with byte-level extraction.
    let preferred_tags = ["PWV3", "PWAV", "PWV2"];
    for tag in preferred_tags {
        if let Some(payload) = find_anlz_chunk_payload(container, tag) {
            let peaks = downsample_waveform_payload(payload, bins.max(1));
            if !peaks.is_empty() {
                return Some(peaks);
            }
        }
    }
    None
}

pub(crate) fn find_anlz_chunk_payload<'a>(bytes: &'a [u8], wanted_tag: &str) -> Option<&'a [u8]> {
    let mut offset = 0usize;
    while offset + 12 <= bytes.len() {
        let tag_bytes = &bytes[offset..offset + 4];
        let tag_ok = tag_bytes
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if !tag_ok {
            offset += 1;
            continue;
        }

        let Ok(tag) = std::str::from_utf8(tag_bytes) else {
            offset += 1;
            continue;
        };

        let header_len = u32::from_be_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let total_len = u32::from_be_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
        ]) as usize;

        if header_len < 12 || total_len < header_len || offset + total_len > bytes.len() {
            offset += 1;
            continue;
        }

        if tag == wanted_tag {
            let start = offset + header_len;
            let end = offset + total_len;
            if start < end {
                return Some(&bytes[start..end]);
            }
        }

        offset += total_len;
    }
    None
}

pub(crate) fn downsample_waveform_payload(payload: &[u8], bins: usize) -> Vec<u8> {
    if payload.is_empty() || bins == 0 {
        return Vec::new();
    }

    let mut peaks = Vec::with_capacity(bins);
    let n = payload.len();
    for i in 0..bins {
        let start = i * n / bins;
        let mut end = ((i + 1) * n / bins).max(start + 1);
        if end > n {
            end = n;
        }
        let mut peak = 0u8;
        let mut sum = 0u32;
        let mut count = 0u32;
        for &b in &payload[start..end] {
            // Extract amplitude from lower 5 bits.  PWAV packs as ((7<<5)|v),
            // PWV3 as ((6<<5)|v), PWV2 stores raw 0-31 — all use b & 0x1F.
            let v = b & 0x1F;
            if v > peak {
                peak = v;
            }
            sum += u32::from(v);
            count += 1;
        }
        let avg = if count > 0 {
            sum as f32 / count as f32
        } else {
            0.0
        };
        // Blend average energy with peak to avoid a "brick wall" look.
        let level = (avg * 0.75) + (f32::from(peak) * 0.25);
        let percent = ((level * 100.0) / 31.0).round().clamp(0.0, 100.0) as u8;
        peaks.push(percent);
    }
    peaks
}

pub(crate) fn scan_anlz_warnings(usb_root: &std::path::Path) -> Vec<String> {
    let anlz_root = usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR);
    if !anlz_root.exists() {
        return Vec::new();
    }

    let mut warnings = Vec::new();
    for entry in WalkDir::new(&anlz_root) {
        match entry {
            Ok(e) => {
                // Surface malformed/anomalous entries under USBANLZ as diagnostics warnings.
                if is_malformed_anlz_entry_name(e.path()) {
                    warnings.push(format!(
                        "analysis entry malformed: {}",
                        sanitize_warning_path(e.path())
                    ));
                }

                if !e.file_type().is_file() {
                    continue;
                }
                let ext = e
                    .path()
                    .extension()
                    .and_then(|v| v.to_str())
                    .map(|s| s.to_ascii_uppercase())
                    .unwrap_or_default();
                if is_known_anlz_metadata_file(e.path()) {
                    continue;
                }
                // Only consider real ANLZ bundle members. Ignore stray/corrupt
                // non-bundle filenames under USBANLZ (often control-char garbage).
                if ext != "DAT" && ext != "EXT" && ext != "2EX" {
                    if let Ok(md) = e.metadata() {
                        if md.len() == 0 {
                            warnings.push(format!(
                                "analysis malformed entry is empty: {}",
                                sanitize_warning_path(e.path())
                            ));
                        }
                    }
                    warnings.push(format!(
                        "analysis entry malformed: {}",
                        sanitize_warning_path(e.path())
                    ));
                    continue;
                }
                match e.metadata() {
                    Ok(md) => {
                        if md.len() == 0 {
                            warnings.push(format!(
                                "analysis file appears empty: {}",
                                sanitize_warning_path(e.path())
                            ));
                        }
                    }
                    Err(err) => warnings.push(format!(
                        "failed to read analysis file metadata {}: {err}",
                        sanitize_warning_path(e.path())
                    )),
                }
            }
            Err(err) => warnings.push(format!("failed to walk USBANLZ analysis directory: {err}")),
        }
    }
    warnings
}

fn is_known_anlz_metadata_file(path: &std::path::Path) -> bool {
    let parts = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>();
    let Some(p_idx) = parts.iter().position(|p| p.eq_ignore_ascii_case("USBANLZ")) else {
        return false;
    };
    let tail = &parts[p_idx + 1..];
    if tail.len() != 1 {
        return false;
    }
    tail[0].eq_ignore_ascii_case("USBMNG.DAT")
}

fn is_malformed_anlz_entry_name(path: &std::path::Path) -> bool {
    // Expected shapes:
    //   .../USBANLZ/PXXX/<8HEX>/ANLZ0000.(DAT|EXT|2EX)
    //   .../USBANLZ/PXXX/<8HEX>   (directory)
    //   .../USBANLZ/PXXX          (directory)
    // Flag obvious anomalies: control chars, non-ASCII, or unknown names inside P* buckets.
    let parts = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>();
    let Some(p_idx) = parts.iter().position(|p| p.eq_ignore_ascii_case("USBANLZ")) else {
        return false;
    };
    if parts.len() <= p_idx + 1 {
        return false;
    }
    let tail = &parts[p_idx + 1..];
    if tail.is_empty() {
        return false;
    }

    // Any non-ASCII/control in USBANLZ subtree is suspicious.
    if tail
        .iter()
        .any(|segment| segment.chars().any(|ch| ch.is_control() || !ch.is_ascii()))
    {
        return true;
    }

    if tail.len() == 1 {
        let p = tail[0];
        if p.eq_ignore_ascii_case("USBMNG.DAT") {
            return false;
        }
        return !(p.len() == 4
            && p.starts_with('P')
            && p[1..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    if tail.len() == 2 {
        let p = tail[0];
        let leaf = tail[1];
        let p_ok =
            p.len() == 4 && p.starts_with('P') && p[1..].chars().all(|c| c.is_ascii_hexdigit());
        let leaf_ok = leaf.len() == 8 && leaf.chars().all(|c| c.is_ascii_hexdigit());
        return !(p_ok && leaf_ok);
    }

    if tail.len() >= 3 {
        let p = tail[0];
        let hash = tail[1];
        let file = tail[2];
        let p_ok =
            p.len() == 4 && p.starts_with('P') && p[1..].chars().all(|c| c.is_ascii_hexdigit());
        let hash_ok = hash.len() == 8 && hash.chars().all(|c| c.is_ascii_hexdigit());
        let file_ok = if let Some((stem, ext)) = file.rsplit_once('.') {
            stem.len() == 8
                && stem[..4].eq_ignore_ascii_case("ANLZ")
                && stem[4..].chars().all(|c| c.is_ascii_digit())
                && (ext.eq_ignore_ascii_case("DAT")
                    || ext.eq_ignore_ascii_case("EXT")
                    || ext.eq_ignore_ascii_case("2EX"))
        } else {
            false
        };
        return !(p_ok && hash_ok && file_ok);
    }

    false
}

pub(crate) fn sanitize_warning_path(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .chars()
        .flat_map(|c| {
            if c.is_control() || c == '\u{fffd}' {
                format!("\\x{:02X}", c as u32).chars().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect::<String>()
}

pub(crate) fn analysis_bundle_exists(usb_root: &Path, anlz_path: &str) -> bool {
    let Some(dat_abs) = resolve_usb_side_path(usb_root, anlz_path) else {
        return false;
    };
    let dat = PathBuf::from(dat_abs);
    if !dat.is_file() {
        return false;
    }
    let ext = dat.with_extension("EXT");
    let twoex = dat.with_extension("2EX");
    ext.is_file() && twoex.is_file()
}

pub(crate) fn load_existing_analysis_paths_by_content_path(
    usb_root: &Path,
    warnings: &mut Vec<String>,
) -> HashMap<String, String> {
    let Some(conn) = open_edb_from_usb_root(usb_root, warnings) else {
        return HashMap::new();
    };
    let mut stmt = match conn.prepare(
        r#"
        SELECT path, analysisDataFilePath
        FROM content
        WHERE path IS NOT NULL
          AND analysisDataFilePath IS NOT NULL
          AND analysisDataFilePath != ''
        "#,
    ) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    let rows = match stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
        Ok(r) => r,
        Err(_) => return HashMap::new(),
    };

    let mut out = HashMap::<String, String>::new();
    for row in rows {
        let Ok((path, anlz)) = row else { continue };
        let p = path.trim();
        let a = anlz.trim();
        if p.is_empty() || a.is_empty() {
            continue;
        }
        out.insert(canonicalize_playlist_name(p), a.to_string());
    }
    out
}

pub(crate) fn load_existing_analysis_paths_by_pdb_track_path(
    usb_root: &Path,
) -> HashMap<String, String> {
    let pdb_path = usb_root
        .join(USB_VENDOR_ROOT_DIR)
        .join(USB_VENDOR_DB_DIR)
        .join("export.pdb");
    let Ok(parsed) = parse_pdb(&pdb_path) else {
        return HashMap::new();
    };
    let mut out = HashMap::<String, String>::new();
    for track in parsed.tracks {
        let path = track.track_file_path.trim();
        let anlz = track.anlz_path.trim();
        if path.is_empty() || anlz.is_empty() {
            continue;
        }
        out.insert(canonicalize_playlist_name(path), anlz.to_string());
    }
    out
}

pub(crate) fn collect_contents_audio_files(usb_root: &std::path::Path) -> Vec<String> {
    let contents_root = usb_root.join(USB_CONTENTS_DIR);
    if !contents_root.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(&contents_root) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase());
        let is_audio = matches!(
            ext.as_deref(),
            Some("mp3")
                | Some("flac")
                | Some("wav")
                | Some("aif")
                | Some("aiff")
                | Some("m4a")
                | Some("aac")
                | Some("ogg")
        );
        if !is_audio {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(usb_root)
            .ok()
            .map(|p| format!("/{}", p.to_string_lossy()))
            .unwrap_or_else(|| entry.path().to_string_lossy().to_string());
        out.push(rel);
    }
    out
}

pub(crate) fn parse_history_numeric_id(id: &str) -> u32 {
    id.rsplit('-')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(u32::MAX)
}

pub(crate) fn canonicalize_playlist_name(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
}

pub(crate) fn repair_utf8_mojibake(value: &str) -> String {
    fn suspicious_score(value: &str) -> usize {
        value
            .chars()
            .filter(|c| {
                matches!(*c, 'Ã' | 'Â' | '¤' | '�') || ('\u{0080}'..='\u{009f}').contains(c)
            })
            .count()
    }

    fn latin1_bytes(value: &str) -> Option<Vec<u8>> {
        let mut bytes = Vec::with_capacity(value.len());
        for ch in value.chars() {
            let code = ch as u32;
            if code > 0xff {
                return None;
            }
            bytes.push(code as u8);
        }
        Some(bytes)
    }

    let mut current = value.to_string();
    for _ in 0..2 {
        let Some(bytes) = latin1_bytes(&current) else {
            break;
        };
        let Ok(decoded) = String::from_utf8(bytes) else {
            break;
        };
        if suspicious_score(&decoded) < suspicious_score(&current) {
            current = decoded;
        } else {
            break;
        }
    }
    current
}

// ── Initialize empty USB ─────────────────────────────

pub fn initialize_usb(usb_root: &str) -> BackendResult<crate::models::InitializeUsbData> {
    let root = Path::new(usb_root);
    if !root.is_dir() {
        return Err(BackendError::Internal(format!(
            "USB root does not exist: {}",
            usb_root
        )));
    }

    let dirs_to_create = [
        root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR),
        root.join(USB_CONTENTS_DIR),
    ];

    let mut created = Vec::new();
    for dir in &dirs_to_create {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
            created.push(dir.to_string_lossy().to_string());
        }
    }

    // Create full-shape PDB baseline if missing.
    let pdb_file = vendor_pdb_path(root);
    if !pdb_file.exists() {
        std::fs::write(&pdb_file, build_fullshape_pdb_bytes())?;
        created.push(pdb_file.to_string_lossy().to_string());
    }

    // Create full-shape encrypted eDB when missing.
    let edb_file = vendor_db_dir(root).join("exportLibrary.db");
    if !edb_file.exists() {
        initialize_fullshape_edb(&edb_file)?;
        created.push(edb_file.to_string_lossy().to_string());
    }

    Ok(crate::models::InitializeUsbData {
        path: usb_root.to_string(),
        created_dirs: created,
    })
}

fn initialize_fullshape_edb(db_path: &Path) -> BackendResult<()> {
    let conn = rusqlite::Connection::open(db_path)?;
    conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))?;
    conn.execute_batch(EDB_SCHEMA)?;
    seed_edb_defaults(&conn)?;
    Ok(())
}

const EDB_SCHEMA: &str = r#"
CREATE TABLE album(album_id integer primary key, name varchar, artist_id integer, image_id integer, isComplation integer, nameForSearch varchar);
CREATE TABLE artist(artist_id integer primary key, name varchar, nameForSearch varchar);
CREATE TABLE category(category_id integer primary key, menuItem_id integer, sequenceNo integer, isVisible integer);
CREATE TABLE color(color_id integer primary key, name varchar);
CREATE TABLE content(content_id integer primary key, title varchar, titleForSearch varchar, subtitle varchar, bpmx100 integer, length integer, trackNo integer, discNo integer, artist_id_artist integer, artist_id_remixer integer, artist_id_originalArtist integer, artist_id_composer integer, artist_id_lyricist integer, album_id integer, genre_id integer, label_id integer, key_id integer, color_id integer, image_id integer, djComment varchar, rating integer, releaseYear integer, releaseDate varchar, dateCreated varchar, dateAdded varchar, path varchar, fileName varchar, fileSize integer, fileType integer, bitrate integer, bitDepth integer, samplingRate integer, isrc varchar, djPlayCount integer, isHotCueAutoLoadOn integer, isKuvoDeliverStatusOn integer, kuvoDeliveryComment varchar, masterDbId integer, masterContentId integer, analysisDataFilePath varchar, analysedBits integer, contentLink integer, hasModified integer, cueUpdateCount integer, analysisDataUpdateCount integer, informationUpdateCount integer);
CREATE TABLE cue(cue_id integer primary key, content_id integer, kind integer, colorTableIndex integer, cueComment varchar, isActiveLoop integer, beatLoopNumerator integer, beatLoopDenominator integer, inUsec integer, outUsec integer, in150FramePerSec integer, out150FramePerSec integer, inMpegFrameNumber integer, outMpegFrameNumber integer, inMpegAbs integer, outMpegAbs integer, inDecodingStartFramePosition integer, outDecodingStartFramePosition integer, inFileOffsetInBlock integer, OutFileOffsetInBlock integer, inNumberOfSampleInBlock integer, outNumberOfSampleInBlock integer);
CREATE TABLE genre(genre_id integer primary key, name varchar);
CREATE TABLE history(history_id integer primary key, sequenceNo integer, name varchar, attribute integer, history_id_parent integer);
CREATE TABLE history_content(history_id integer, content_id integer, sequenceNo integer);
CREATE TABLE hotCueBankList(hotCueBankList_id integer primary key, sequenceNo integer, name varchar, image_id integer, attribute integer, hotCueBankList_id_parent integer);
CREATE TABLE hotCueBankList_cue(hotCueBankList_id integer, cue_id integer, sequenceNo integer);
CREATE TABLE image(image_id integer primary key, path varchar);
CREATE TABLE key(key_id integer primary key, name varchar);
CREATE TABLE label(label_id integer primary key, name varchar);
CREATE TABLE menuItem(menuItem_id integer primary key, kind integer, name varchar);
CREATE TABLE myTag(myTag_id integer primary key, sequenceNo integer, name varchar, attribute integer, myTag_id_parent integer);
CREATE TABLE myTag_content(myTag_id integer, content_id integer);
CREATE TABLE playlist(playlist_id integer primary key, sequenceNo integer, name varchar, image_id integer, attribute integer, playlist_id_parent integer);
CREATE TABLE playlist_content(playlist_id integer, content_id integer, sequenceNo integer);
CREATE TABLE property(deviceName varchar, dbVersion varchar, numberOfContents integer, createdDate varchar, backGroundColorType integer, myTagMasterDBID integer);
CREATE TABLE recommendedLike(content_id_1 integer, content_id_2 integer, rating integer, createdDate integer);
CREATE TABLE sort(sort_id integer primary key, menuItem_id integer, sequenceNo integer, isVisible integer, isSelectedAsSubColumn integer);
CREATE INDEX index_hotCueBankList_cue_hotCueBankList_id on hotCueBankList_cue(hotCueBankList_id);
CREATE INDEX index_myTag_content_content_id on myTag_content(content_id);
CREATE INDEX index_myTag_content_myTag_id on myTag_content(myTag_id);
CREATE INDEX index_playlist_content_playlist_id on playlist_content(playlist_id);
"#;

fn seed_edb_defaults(conn: &rusqlite::Connection) -> BackendResult<()> {
    // property
    conn.execute(
        "INSERT INTO property (deviceName, dbVersion, numberOfContents, createdDate, backGroundColorType, myTagMasterDBID) VALUES ('', '1000', 0, date('now'), 0, 969967066)",
        [],
    )?;

    // color (8 standard player colors)
    let colors = [
        (1, "Pink"),
        (2, "Red"),
        (3, "Orange"),
        (4, "Yellow"),
        (5, "Green"),
        (6, "Aqua"),
        (7, "Blue"),
        (8, "Purple"),
    ];
    for (id, name) in &colors {
        conn.execute(
            "INSERT INTO color (color_id, name) VALUES (?1, ?2)",
            rusqlite::params![id, name],
        )?;
    }

    // menuItem/category/sort defaults must match the working reference export baseline.
    // Freshly initialized USBs rely on these tables before any additive export can
    // preserve reference rows from an existing device database.
    let menu_items: [(i32, i32, &str); 27] = [
        (1, 128, "\u{FFFA}GENRE\u{FFFB}"),
        (2, 129, "\u{FFFA}ARTIST\u{FFFB}"),
        (3, 130, "\u{FFFA}ALBUM\u{FFFB}"),
        (4, 131, "\u{FFFA}TRACK\u{FFFB}"),
        (5, 133, "\u{FFFA}BPM\u{FFFB}"),
        (6, 134, "\u{FFFA}RATING\u{FFFB}"),
        (7, 135, "\u{FFFA}YEAR\u{FFFB}"),
        (8, 136, "\u{FFFA}REMIXER\u{FFFB}"),
        (9, 137, "\u{FFFA}LABEL\u{FFFB}"),
        (10, 138, "\u{FFFA}ORIGINAL ARTIST\u{FFFB}"),
        (11, 139, "\u{FFFA}KEY\u{FFFB}"),
        (12, 141, "\u{FFFA}CUE\u{FFFB}"),
        (13, 142, "\u{FFFA}COLOR\u{FFFB}"),
        (14, 146, "\u{FFFA}TIME\u{FFFB}"),
        (15, 147, "\u{FFFA}BITRATE\u{FFFB}"),
        (16, 148, "\u{FFFA}FILE NAME\u{FFFB}"),
        (17, 132, "\u{FFFA}PLAYLIST\u{FFFB}"),
        (18, 152, "\u{FFFA}HOT CUE BANK\u{FFFB}"),
        (19, 149, "\u{FFFA}HISTORY\u{FFFB}"),
        (20, 145, "\u{FFFA}SEARCH\u{FFFB}"),
        (21, 150, "\u{FFFA}COMMENTS\u{FFFB}"),
        (22, 140, "\u{FFFA}DATE ADDED\u{FFFB}"),
        (23, 151, "\u{FFFA}DJ PLAY COUNT\u{FFFB}"),
        (24, 144, "\u{FFFA}FOLDER\u{FFFB}"),
        (25, 161, "\u{FFFA}DEFAULT\u{FFFB}"),
        (26, 162, "\u{FFFA}ALPHABET\u{FFFB}"),
        (27, 170, "\u{FFFA}MATCHING\u{FFFB}"),
    ];
    for (id, kind, name) in &menu_items {
        conn.execute(
            "INSERT INTO menuItem (menuItem_id, kind, name) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, kind, name],
        )?;
    }

    // category seed mirrors PDB t16 (the master menu source) and matches the
    // Default visible set: ARTIST, ALBUM, TRACK, KEY, PLAYLIST, HISTORY,
    // SEARCH, MATCHING, FOLDER, DATE ADDED. eDB.menuItem keeps the full 27
    // catalog so the Menu Editor can promote a hidden kind back to visible
    // (which patches PDB t16 + eDB.category atomically). The 5 menuItems
    // observed without a category row in vendor exports (CUE, COMMENTS, DJ PLAY COUNT,
    // DEFAULT, ALPHABET) are intentionally omitted here to keep the eDB
    // shape byte-comparable to vendor exports.
    let category_rows: [(i32, i32, i32, i32); 22] = [
        (1, 1, 0, 0),    // GENRE
        (2, 2, 1, 1),    // ARTIST
        (3, 3, 2, 1),    // ALBUM
        (4, 4, 3, 1),    // TRACK
        (5, 17, 5, 1),   // PLAYLIST
        (6, 5, 0, 0),    // BPM
        (7, 6, 0, 0),    // RATING
        (8, 7, 0, 0),    // YEAR
        (9, 8, 0, 0),    // REMIXER
        (10, 9, 0, 0),   // LABEL
        (11, 10, 0, 0),  // ORIGINAL ARTIST
        (12, 11, 4, 1),  // KEY
        (15, 13, 0, 0),  // COLOR
        (17, 24, 9, 1),  // FOLDER
        (18, 20, 7, 1),  // SEARCH
        (19, 14, 0, 0),  // TIME
        (20, 15, 0, 0),  // BITRATE
        (21, 16, 0, 0),  // FILE NAME
        (22, 19, 6, 1),  // HISTORY
        (23, 18, 0, 0),  // HOT CUE BANK
        (26, 27, 8, 1),  // MATCHING
        (27, 22, 10, 1), // DATE ADDED
    ];
    for (id, mi, seq, vis) in &category_rows {
        conn.execute(
            "INSERT INTO category (category_id, menuItem_id, sequenceNo, isVisible) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, mi, seq, vis],
        )?;
    }

    // sort (17 rows)
    let sorts: [(i32, i32, i32, i32, i32); 17] = [
        (0, 25, 1, 1, 0),
        (1, 26, 2, 1, 0),
        (2, 2, 3, 1, 0),
        (3, 3, 4, 1, 0),
        (4, 5, 5, 1, 0),
        (5, 6, 6, 1, 0),
        (6, 1, 0, 0, 0),
        (7, 21, 0, 0, 0),
        (8, 14, 0, 0, 0),
        (9, 8, 0, 0, 0),
        (10, 9, 0, 0, 0),
        (11, 10, 0, 0, 0),
        (12, 11, 7, 1, 0),
        (13, 15, 0, 0, 0),
        (15, 13, 0, 0, 0),
        (16, 23, 0, 0, 0),
        (17, 22, 0, 0, 0),
    ];
    for (id, mi, seq, vis, sel) in &sorts {
        conn.execute("INSERT INTO sort (sort_id, menuItem_id, sequenceNo, isVisible, isSelectedAsSubColumn) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![id, mi, seq, vis, sel])?;
    }

    // myTag (28 rows — 4 top-level groups + children)
    let my_tags: [(i64, i32, &str, i32, i64); 28] = [
        (1, 1, "Genre", 1, 0),
        (2, 2, "Components", 1, 0),
        (3, 3, "Situation", 1, 0),
        (4, 4, "Untitled Column", 1, 0),
        (302295111, 1, "Acid House", 0, 1),
        (550969514, 2, "Deep House", 0, 1),
        (674668498, 3, "Techno", 0, 1),
        (886498498, 4, "Nu Disco", 0, 1),
        (986312498, 5, "Electro House", 0, 1),
        (1001102498, 6, "Bass Music", 0, 1),
        (1133242498, 7, "Trap", 0, 1),
        (268245111, 1, "Synth", 0, 2),
        (468969514, 2, "Vocal", 0, 2),
        (574668498, 3, "Beat", 0, 2),
        (786498498, 4, "Sub Bass", 0, 2),
        (886312498, 5, "Percussion", 0, 2),
        (901102498, 6, "Piano", 0, 2),
        (1033242498, 7, "Dark", 0, 2),
        (1133242499, 8, "Upper", 0, 2),
        (202295111, 1, "Main Floor", 0, 3),
        (350969514, 2, "Second Floor", 0, 3),
        (474668498, 3, "Lounge", 0, 3),
        (686498498, 4, "Mid Night", 0, 3),
        (786312498, 5, "Morning", 0, 3),
        (801102498, 6, "Build up", 0, 3),
        (933242498, 7, "Peak Time", 0, 3),
        (1033242499, 8, "Build down", 0, 3),
        (168245111, 1, "My Comment", 0, 4),
    ];
    for (id, seq, name, attr, parent) in &my_tags {
        conn.execute("INSERT INTO myTag (myTag_id, sequenceNo, name, attribute, myTag_id_parent) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![id, seq, name, attr, parent])?;
    }

    Ok(())
}

fn build_fullshape_pdb_bytes() -> Vec<u8> {
    use crate::pdb_writer::{PdbData, standard_colors, standard_columns_raw, write_pdb_to_bytes};
    let data = PdbData {
        colors: standard_colors(),
        columns_raw_rows: standard_columns_raw(),
        ..PdbData::empty()
    };
    let mut bytes = write_pdb_to_bytes(&data).expect("build empty PDB");
    seed_rb_initialized_history_shape(&mut bytes);
    // The fresh writer starts seq_counter at 100, leaving seqdb=102 in the template.
    // Reset seqdb to 34 — template max_seqpage is now 31 (from tt=19 seq=31, matching
    // reference exporter template), so seqdb=34 satisfies seqdb > max_seqpage. The additive writer
    // will assign new pages seq=32+ on first export, matching reference export range.
    if let Some(sl) = bytes.get_mut(0x14..0x18) {
        sl.copy_from_slice(&34u32.to_le_bytes());
    }
    bytes
}

fn seed_rb_initialized_history_shape(bytes: &mut [u8]) {
    use crate::utils::{set_table_ptr_fields, write_u8_at, write_u16_le_at, write_u32_le_at};
    const PAGE_SIZE: usize = 4096;
    if bytes.len() < PAGE_SIZE * 40 {
        return;
    }

    fn write_data_page(
        bytes: &mut [u8],
        page_idx: u32,
        table_type: u32,
        next_page: u32,
        seq: u32,
        u3: u8,
        rows: &[&[u8]],
        rowpf_words: &[u16],
        tranrf_words: &[u16],
    ) {
        let base = page_idx as usize * PAGE_SIZE;
        let Some(page) = bytes.get_mut(base..base + PAGE_SIZE) else {
            return;
        };
        page.fill(0);

        let nrs = rows.len();
        let mut payload = Vec::<u8>::new();
        let mut offsets = Vec::<u16>::with_capacity(nrs);
        for row in rows {
            offsets.push(payload.len() as u16);
            payload.extend_from_slice(row);
        }
        let used_s = payload.len();
        let groups = nrs.div_ceil(16);
        let footer_size = groups * 4 + nrs * 2;
        let free_s = PAGE_SIZE
            .saturating_sub(40)
            .saturating_sub(used_s)
            .saturating_sub(footer_size);

        write_u32_le_at(page, 0x04, page_idx);
        write_u32_le_at(page, 0x08, table_type);
        write_u32_le_at(page, 0x0c, next_page);
        write_u32_le_at(page, 0x2c, next_page);
        write_u32_le_at(page, 0x10, seq);
        page[0x18] = nrs as u8;
        page[0x19] = u3;
        page[0x1a] = if groups >= 2 { 2 } else { 0 };
        page[0x1b] = 36;
        write_u16_le_at(page, 0x1c, free_s as u16);
        write_u16_le_at(page, 0x1e, used_s as u16);
        write_u16_le_at(page, 0x20, nrs as u16);
        write_u16_le_at(page, 0x22, 0);

        page[40..40 + used_s].copy_from_slice(&payload);

        let mut m = PAGE_SIZE;
        for i in 0..nrs {
            if i % 16 == 0 {
                let group = i / 16;
                m -= 2;
                write_u16_le_at(page, m, *tranrf_words.get(group).unwrap_or(&0));
                m -= 2;
                write_u16_le_at(page, m, *rowpf_words.get(group).unwrap_or(&0));
            }
            m -= 2;
            write_u16_le_at(page, m, offsets[i]);
        }
    }

    // Row order and u4 values match reference exporter history_playlist_rows constants.
    let t17_rows: [[u8; 8]; 22] = [
        [0x0f, 0x00, 0x14, 0x00, 0x06, 0x01, 0x00, 0x00], // u1=15
        [0x10, 0x00, 0x15, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=16
        [0x12, 0x00, 0x17, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=18
        [0x08, 0x00, 0x09, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=8
        [0x09, 0x00, 0x0a, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=9
        [0x0a, 0x00, 0x0b, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=10
        [0x0d, 0x00, 0x0f, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=13
        [0x0e, 0x00, 0x13, 0x00, 0x04, 0x01, 0x00, 0x00], // u1=14
        [0x01, 0x00, 0x01, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=1
        [0x05, 0x00, 0x06, 0x00, 0x05, 0x01, 0x00, 0x00], // u1=5
        [0x06, 0x00, 0x07, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=6
        [0x07, 0x00, 0x08, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=7
        [0x02, 0x00, 0x02, 0x00, 0x02, 0x00, 0x01, 0x00], // u1=2,  u5=1
        [0x03, 0x00, 0x03, 0x00, 0x03, 0x00, 0x02, 0x00], // u1=3,  u5=2
        [0x04, 0x00, 0x04, 0x00, 0x01, 0x00, 0x03, 0x00], // u1=4,  u5=3
        [0x0b, 0x00, 0x0c, 0x00, 0x63, 0x00, 0x04, 0x00], // u1=11, u5=4
        [0x11, 0x00, 0x05, 0x00, 0x63, 0x00, 0x05, 0x00], // u1=17, u5=5
        [0x13, 0x00, 0x16, 0x00, 0x63, 0x00, 0x06, 0x00], // u1=19, u5=6
        [0x14, 0x00, 0x12, 0x00, 0x63, 0x00, 0x07, 0x00], // u1=20, u5=7
        [0x1b, 0x00, 0x1a, 0x00, 0x63, 0x02, 0x08, 0x00], // u1=27, u5=8, u4=2
        [0x18, 0x00, 0x11, 0x00, 0x63, 0x00, 0x09, 0x00], // u1=24, u5=9
        [0x16, 0x00, 0x1b, 0x00, 0x63, 0x05, 0x0a, 0x00], // u1=22, u5=10, u4=5
    ];
    let t18_rows: [[u8; 8]; 17] = [
        [0x01, 0x00, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x15, 0x00, 0x07, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x0e, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x08, 0x00, 0x09, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x09, 0x00, 0x0a, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x0a, 0x00, 0x0b, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x0f, 0x00, 0x0d, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x0d, 0x00, 0x0f, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x17, 0x00, 0x10, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x16, 0x00, 0x11, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x19, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00],
        [0x1a, 0x00, 0x01, 0x00, 0x00, 0x02, 0x00, 0x00],
        [0x02, 0x00, 0x02, 0x00, 0x00, 0x03, 0x00, 0x00],
        [0x03, 0x00, 0x03, 0x00, 0x00, 0x04, 0x00, 0x00],
        [0x05, 0x00, 0x04, 0x00, 0x00, 0x05, 0x00, 0x00],
        [0x06, 0x00, 0x05, 0x00, 0x00, 0x06, 0x00, 0x00],
        [0x0b, 0x00, 0x0c, 0x00, 0x00, 0x07, 0x00, 0x00],
    ];
    let t19_row: [u8; 40] = [
        0x80, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x17, 0x32, 0x30,
        0x32, 0x36, 0x2d, 0x30, 0x33, 0x2d, 0x30, 0x36, 0x19, 0x1e, 0x0b, 0x31, 0x30, 0x30, 0x30,
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    let t17_refs = t17_rows.iter().map(|r| r.as_slice()).collect::<Vec<_>>();
    let t18_refs = t18_rows.iter().map(|r| r.as_slice()).collect::<Vec<_>>();
    write_data_page(
        bytes,
        36,
        17,
        44,
        4,
        0xc0,
        &t17_refs,
        &[0xffff, 0x003f],
        &[0xffff, 0x003f],
    );
    write_data_page(
        bytes,
        38,
        18,
        45,
        5,
        0x20,
        &t18_refs,
        &[0xffff, 0x0001],
        &[0xffff, 0x0001],
    );
    write_data_page(
        bytes,
        40,
        19,
        41,
        31,
        0x20,
        &[&t19_row],
        &[0x0001],
        &[0x0001],
    );

    // Repoint empty-candidate chains to reference export shape.
    set_table_ptr_fields(bytes, 6, 42, 13, 14);
    set_table_ptr_fields(bytes, 16, 43, 33, 34);
    set_table_ptr_fields(bytes, 17, 44, 35, 36);
    set_table_ptr_fields(bytes, 18, 45, 37, 38);
    set_table_ptr_fields(bytes, 19, 41, 39, 40);

    // Reference baseline increments seq/next on populated colors/columns pages.
    write_u32_le_at(bytes, 14 * PAGE_SIZE + 0x0c, 42);
    write_u32_le_at(bytes, 14 * PAGE_SIZE + 0x10, 2);

    write_u32_le_at(bytes, 34 * PAGE_SIZE + 0x0c, 43);
    write_u32_le_at(bytes, 34 * PAGE_SIZE + 0x10, 3);

    // Match observed reference flags on these two pages.
    write_u8_at(bytes, 14 * PAGE_SIZE + 0x19, 0x00);
    write_u8_at(bytes, 34 * PAGE_SIZE + 0x19, 0x60);

    // Keep file-level next-unallocated-page pointer consistent with highest virtual EC.
    write_u32_le_at(bytes, 0x0c, 46);
}

// ── External library master.db autodetection ────────────────

pub fn detect_external_master_db() -> crate::models::DetectExternalMasterDbData {
    let candidates = external_master_db_candidates();

    for candidate in &candidates {
        if candidate.is_file() {
            return crate::models::DetectExternalMasterDbData {
                found: true,
                path: Some(candidate.to_string_lossy().to_string()),
            };
        }
    }

    crate::models::DetectExternalMasterDbData {
        found: false,
        path: None,
    }
}

pub(crate) fn external_master_db_candidates() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let mut candidates = Vec::new();

    // Check env var override first
    if let Ok(env_path) = std::env::var(MASTER_DB_ENV_KEY) {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    // HOME-based paths (macOS + Linux)
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        // macOS: desktop library uses "Pioneer DJ" (with space) as the application directory
        candidates.push(
            home.join("Library/Application Support")
                .join("Pioneer DJ/rekordbox/master.db"),
        );
        // macOS fallback: older installs may use "Pioneer"
        candidates.push(
            home.join("Library/Application Support")
                .join(desktop_master_db_rel_path()),
        );
        // Linux
        candidates.push(home.join(".local/share").join(desktop_master_db_rel_path()));
    }

    // Windows APPDATA
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(PathBuf::from(appdata).join(desktop_master_db_rel_path()));
    }

    // Windows USERPROFILE fallback
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        candidates.push(
            PathBuf::from(userprofile)
                .join("AppData/Roaming")
                .join(desktop_master_db_rel_path()),
        );
    }

    candidates
}

#[cfg(test)]
mod diag_tests {
    use super::*;
    use crate::models::{DiagStatus, ExportToUsbOptions};
    use crate::pdb_reader::{ParsedPdb, PdbPlaylistEntryRow, PdbPlaylistTreeRow, PdbTrackRow};
    use crate::service::analysis::resolve_analysis_worker_count;
    use crate::service::diagnostics::{diagnose_contents_integrity, diagnose_playlist_resolution};
    use crate::service::export_helpers::{
        CONTENT_FILENAME_MAX_LEN, ExportManifest, ExportManifestTrack, ExportPlaylistData,
        exported_media_target_path, write_edb_playlist,
    };
    use crate::service::now;
    use crate::service::usb_helpers::{parse_history_name_numeric_id, parse_history_slot_id};
    use rusqlite::params;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;
    use uuid::Uuid;

    fn env_var_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn diag_status_worst_of_all_pass() {
        let result = DiagStatus::worst_of(&[&DiagStatus::Pass, &DiagStatus::Pass]);
        assert!(matches!(result, DiagStatus::Pass));
    }

    #[test]
    fn diag_status_worst_of_includes_warn() {
        let result = DiagStatus::worst_of(&[&DiagStatus::Pass, &DiagStatus::Warn]);
        assert!(matches!(result, DiagStatus::Warn));
    }

    #[test]
    fn diag_status_worst_of_includes_fail() {
        let result =
            DiagStatus::worst_of(&[&DiagStatus::Pass, &DiagStatus::Warn, &DiagStatus::Fail]);
        assert!(matches!(result, DiagStatus::Fail));
    }

    #[test]
    fn analysis_worker_count_uses_cpu_limit() {
        assert_eq!(resolve_analysis_worker_count(8, 4), 4);
        assert_eq!(resolve_analysis_worker_count(2, 16), 2);
    }

    #[test]
    fn analysis_worker_count_never_returns_zero() {
        assert_eq!(resolve_analysis_worker_count(0, 0), 1);
        assert_eq!(resolve_analysis_worker_count(0, 7), 1);
        assert_eq!(resolve_analysis_worker_count(7, 0), 1);
    }

    #[test]
    fn contents_integrity_exact_match() {
        let section = diagnose_contents_integrity(100, 100);
        assert!(matches!(section.status, DiagStatus::Pass));
        assert_eq!(section.checks.len(), 3);
        let counts = section.counts.expect("contents counts");
        assert_eq!(counts.contents_count, 100);
        assert_eq!(counts.indexed_count, 100);
        assert_eq!(counts.mismatch_count, 0);
    }

    #[test]
    fn contents_integrity_small_mismatch_warns() {
        let section = diagnose_contents_integrity(103, 100);
        assert!(matches!(section.status, DiagStatus::Warn));
        let counts = section.counts.expect("contents counts");
        assert_eq!(counts.contents_count, 103);
        assert_eq!(counts.indexed_count, 100);
        assert_eq!(counts.mismatch_count, 3);
    }

    #[test]
    fn contents_integrity_large_unindexed_audio_mismatch_warns() {
        let section = diagnose_contents_integrity(110, 100);
        assert!(matches!(section.status, DiagStatus::Warn));
        let counts = section.counts.expect("contents counts");
        assert_eq!(counts.contents_count, 110);
        assert_eq!(counts.indexed_count, 100);
        assert_eq!(counts.mismatch_count, 10);
    }

    #[test]
    fn contents_integrity_missing_files_fails() {
        let section = diagnose_contents_integrity(90, 100);
        assert!(matches!(section.status, DiagStatus::Fail));
        let counts = section.counts.expect("contents counts");
        assert_eq!(counts.contents_count, 90);
        assert_eq!(counts.indexed_count, 100);
        assert_eq!(counts.mismatch_count, -10);
    }

    #[test]
    fn scan_anlz_warnings_ignores_usbmng_dat() {
        let temp = tempfile::tempdir().expect("tempdir");
        let anlz_root = temp.path().join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR);
        std::fs::create_dir_all(&anlz_root).expect("create USBANLZ");
        std::fs::write(anlz_root.join("USBMNG.DAT"), b"stub").expect("write USBMNG.DAT");

        let warnings = scan_anlz_warnings(temp.path());

        assert!(
            warnings
                .iter()
                .all(|w| !w.contains("USBMNG.DAT") && !w.contains("analysis entry malformed"))
        );
    }

    #[test]
    fn scan_anlz_warnings_accepts_numbered_anlz_bundle_members() {
        let temp = tempfile::tempdir().expect("tempdir");
        let anlz_dir = temp
            .path()
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_ANALYSIS_DIR)
            .join("P04F")
            .join("00019DDD");
        std::fs::create_dir_all(&anlz_dir).expect("create ANLZ dir");
        std::fs::write(anlz_dir.join("ANLZ0001.2EX"), b"stub").expect("write numbered 2EX");

        let warnings = scan_anlz_warnings(temp.path());

        assert!(
            warnings
                .iter()
                .all(|w| !w.contains("ANLZ0001.2EX") && !w.contains("analysis entry malformed")),
            "numbered ANLZ bundle members should not be treated as malformed"
        );
    }

    #[test]
    fn try_read_playlists_from_edb_merges_duplicate_same_name_playlists() {
        let tmp = tempdir().unwrap();
        let vendor_db = tmp.path().join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        std::fs::create_dir_all(&vendor_db).unwrap();

        let db_path = vendor_db.join("exportLibrary.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE artist (artist_id INTEGER PRIMARY KEY, name TEXT);
            CREATE TABLE album (album_id INTEGER PRIMARY KEY, artist_id INTEGER, name TEXT);
            CREATE TABLE "key" (key_id INTEGER PRIMARY KEY, scaleName TEXT, seq INTEGER, name TEXT);
            CREATE TABLE image (image_id INTEGER PRIMARY KEY, path TEXT);
            CREATE TABLE playlist (playlist_id INTEGER PRIMARY KEY, name TEXT, attribute INTEGER, sequenceNo INTEGER);
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              title TEXT,
              artist_id_artist INTEGER,
              album_id INTEGER,
              key_id INTEGER,
              image_id INTEGER,
              path TEXT,
              analysisDataFilePath TEXT,
              bpmx100 INTEGER,
              length INTEGER
            );
            CREATE TABLE playlist_content (playlist_id INTEGER, content_id INTEGER, sequenceNo INTEGER);
            INSERT INTO artist (artist_id, name) VALUES (1, 'Artist');
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES
              (1, 'Testi', 0, 1),
              (2, 'Testi', 0, 2);
            INSERT INTO content (content_id, title, artist_id_artist, path, bpmx100, length) VALUES
              (11, 'Old A', 1, '/Contents/A.mp3', 12000, 180),
              (12, 'Old B', 1, '/Contents/B.mp3', 12100, 181),
              (13, 'New C', 1, '/Contents/C.mp3', 12200, 182);
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES
              (1, 11, 1),
              (1, 12, 2),
              (2, 13, 1);
            "#,
        )
        .unwrap();
        drop(conn);

        std::fs::create_dir_all(tmp.path().join("Contents")).unwrap();
        std::fs::write(tmp.path().join("Contents/A.mp3"), b"a").unwrap();
        std::fs::write(tmp.path().join("Contents/B.mp3"), b"b").unwrap();
        std::fs::write(tmp.path().join("Contents/C.mp3"), b"c").unwrap();

        let mut warnings = Vec::new();
        let playlists =
            crate::edb::try_read_playlists_with_metadata_from_edb(tmp.path(), &mut warnings)
                .unwrap();
        let testi = playlists.get("Testi").expect("merged playlist");
        assert_eq!(
            testi.playlist_id, 1,
            "same-name merge should keep the earliest playlist id"
        );
        assert_eq!(
            testi.sort_order, 1,
            "same-name merge should keep the earliest playlist sort order"
        );
        assert_eq!(
            testi.tracks.len(),
            3,
            "same-name export DB playlists should merge"
        );
    }

    fn make_track(id: u32) -> PdbTrackRow {
        PdbTrackRow {
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            id,
            artist_id: 0,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
            bitrate_kbps: None,
            track_number: id,
            tempo_x100: 12800,
            release_year: None,
            bit_depth: None,
            duration_seconds: None,
            file_type: None,
            isrc: None,
            date_added: None,
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info: None,
            autoload_hotcues: None,
            title: format!("Track {id}"),
            anlz_path: String::new(),
            track_file_path: String::new(),
        }
    }

    fn make_entry(playlist_id: u32, track_id: u32) -> PdbPlaylistEntryRow {
        PdbPlaylistEntryRow {
            entry_index: 0,
            track_id,
            playlist_id,
        }
    }

    fn make_tree_leaf(id: u32, name: &str) -> PdbPlaylistTreeRow {
        PdbPlaylistTreeRow {
            id,
            parent_id: 0,
            sort_order: id,
            row_is_folder: false,
            name: name.to_string(),
        }
    }

    #[test]
    fn normalize_usb_root_path_from_subfolders() {
        let root = std::path::PathBuf::from("/tmp/USB");
        assert_eq!(normalize_usb_root_path(root.join(USB_CONTENTS_DIR)), root);
        assert_eq!(
            normalize_usb_root_path(root.join(USB_VENDOR_ROOT_DIR)),
            root
        );
        assert_eq!(
            normalize_usb_root_path(root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)),
            root
        );
    }

    #[test]
    fn playlist_resolution_all_resolved() {
        let mut parsed = ParsedPdb::default();
        parsed.tracks = vec![make_track(1), make_track(2), make_track(3)];
        parsed.playlist_tree = vec![make_tree_leaf(10, "MyPlaylist")];
        parsed.playlist_entries = vec![make_entry(10, 1), make_entry(10, 2), make_entry(10, 3)];

        let (section, details) = diagnose_playlist_resolution(Some(&parsed));
        assert!(matches!(section.status, DiagStatus::Pass));
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].resolved_entries, 3);
        assert_eq!(details[0].total_entries, 3);
        assert!((details[0].resolution_rate - 1.0).abs() < 0.001);
    }

    #[test]
    fn playlist_resolution_partial_warns() {
        let mut parsed = ParsedPdb::default();
        parsed.tracks = vec![make_track(1), make_track(2)];
        parsed.playlist_tree = vec![make_tree_leaf(10, "Partial")];
        // 10 entries, only 2 track IDs exist (1, 2), rest are orphaned (3..10)
        parsed.playlist_entries = (1..=10).map(|i| make_entry(10, i)).collect();

        let (section, details) = diagnose_playlist_resolution(Some(&parsed));
        // 2/10 = 20% < 80% threshold → FAIL
        assert!(matches!(section.status, DiagStatus::Fail));
        assert_eq!(details[0].resolved_entries, 2);
        assert_eq!(details[0].total_entries, 10);
    }

    #[test]
    fn playlist_resolution_borderline_warn() {
        let mut parsed = ParsedPdb::default();
        // 9 tracks, 10 entries → 90% resolution → WARN (not PASS, < 100%)
        parsed.tracks = (1..=9).map(make_track).collect();
        parsed.playlist_tree = vec![make_tree_leaf(10, "Almost")];
        parsed.playlist_entries = (1..=10).map(|i| make_entry(10, i)).collect();

        let (section, details) = diagnose_playlist_resolution(Some(&parsed));
        assert!(matches!(section.status, DiagStatus::Warn));
        assert_eq!(details[0].resolved_entries, 9);
    }

    #[test]
    fn playlist_resolution_empty_playlist_passes() {
        let mut parsed = ParsedPdb::default();
        parsed.tracks = vec![make_track(1)];
        parsed.playlist_tree = vec![make_tree_leaf(10, "Empty")];
        // No entries for playlist 10

        let (section, details) = diagnose_playlist_resolution(Some(&parsed));
        assert!(matches!(section.status, DiagStatus::Pass));
        assert_eq!(details[0].total_entries, 0);
        assert_eq!(details[0].resolved_entries, 0);
        assert!((details[0].resolution_rate - 1.0).abs() < 0.001);
    }

    #[test]
    fn playlist_resolution_no_pdb_fails() {
        let (section, details) = diagnose_playlist_resolution(None);
        assert!(matches!(section.status, DiagStatus::Fail));
        assert!(details.is_empty());
    }

    #[test]
    fn playlist_resolution_multiple_playlists() {
        let mut parsed = ParsedPdb::default();
        parsed.tracks = vec![make_track(1), make_track(2), make_track(3)];
        parsed.playlist_tree = vec![make_tree_leaf(10, "Full"), make_tree_leaf(20, "Half")];
        parsed.playlist_entries = vec![
            make_entry(10, 1),
            make_entry(10, 2),
            make_entry(20, 3),
            make_entry(20, 999), // orphaned
        ];

        let (section, details) = diagnose_playlist_resolution(Some(&parsed));
        assert_eq!(details.len(), 2);
        // "Full" is 100% → PASS
        assert!(matches!(details[0].status, DiagStatus::Pass));
        // "Half" is 50% → FAIL (< 80%)
        assert!(matches!(details[1].status, DiagStatus::Fail));
        // Overall should be FAIL
        assert!(matches!(section.status, DiagStatus::Fail));
    }

    #[test]
    fn parse_history_slot_id_extracts_numeric_value() {
        assert_eq!(parse_history_slot_id("001"), Some(1));
        assert_eq!(parse_history_slot_id("history 042"), Some(42));
        assert_eq!(parse_history_slot_id("no-digits"), None);
    }

    #[test]
    fn parse_history_name_numeric_id_only_accepts_history_names() {
        assert_eq!(parse_history_name_numeric_id("HISTORY 007"), Some(7));
        assert_eq!(parse_history_name_numeric_id("history 7"), Some(7));
        assert_eq!(parse_history_name_numeric_id("Session 007"), None);
    }

    #[test]
    fn detect_external_master_db_env_override_nonexistent() {
        let _guard = env_var_lock().lock().expect("env var lock");
        // Point to a path that doesn't exist — should return found: false
        unsafe {
            std::env::set_var(
                "DJUSBTKIT_MASTER_DB_PATH",
                "/tmp/__nonexistent_external_master_db_test__",
            )
        };
        let result = detect_external_master_db();
        unsafe { std::env::remove_var("DJUSBTKIT_MASTER_DB_PATH") };
        assert!(!result.found);
        assert!(result.path.is_none());
    }

    #[test]
    fn detect_external_master_db_env_override_existing_file() {
        let _guard = env_var_lock().lock().expect("env var lock");
        let tmp = std::env::temp_dir().join("__test_external_master.db");
        std::fs::write(&tmp, b"fake").unwrap();
        unsafe { std::env::set_var("DJUSBTKIT_MASTER_DB_PATH", tmp.to_str().unwrap()) };
        let result = detect_external_master_db();
        unsafe { std::env::remove_var("DJUSBTKIT_MASTER_DB_PATH") };
        std::fs::remove_file(&tmp).ok();
        assert!(result.found);
        assert_eq!(result.path.unwrap(), tmp.to_string_lossy().to_string());
    }

    #[test]
    fn external_master_db_candidates_includes_env_override() {
        let _guard = env_var_lock().lock().expect("env var lock");
        unsafe { std::env::set_var("DJUSBTKIT_MASTER_DB_PATH", "/custom/path/master.db") };
        let candidates = external_master_db_candidates();
        unsafe { std::env::remove_var("DJUSBTKIT_MASTER_DB_PATH") };
        assert!(
            candidates
                .iter()
                .any(|p| p.to_str() == Some("/custom/path/master.db"))
        );
    }

    #[test]
    fn initialize_usb_creates_structure() {
        let tmp = std::env::temp_dir().join("__test_init_usb__");
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp).ok();
        }
        std::fs::create_dir_all(&tmp).unwrap();

        let result = initialize_usb(tmp.to_str().unwrap()).unwrap();
        assert_eq!(result.path, tmp.to_str().unwrap());
        assert!(!result.created_dirs.is_empty());

        // Verify structure
        assert!(
            tmp.join(USB_VENDOR_ROOT_DIR)
                .join(USB_VENDOR_DB_DIR)
                .is_dir()
        );
        assert!(tmp.join(USB_CONTENTS_DIR).is_dir());
        assert!(
            tmp.join(USB_VENDOR_ROOT_DIR)
                .join(USB_VENDOR_DB_DIR)
                .join("export.pdb")
                .is_file()
        );
        let parsed = parse_pdb(
            &tmp.join(USB_VENDOR_ROOT_DIR)
                .join(USB_VENDOR_DB_DIR)
                .join("export.pdb"),
        )
        .expect("parse initialized PDB");
        assert!(
            parsed.tracks.is_empty()
                && parsed.playlist_tree.is_empty()
                && parsed.playlist_entries.is_empty(),
            "initialized PDB should be valid and empty"
        );

        // Running initialize again should be idempotent (no error)
        let result2 = initialize_usb(tmp.to_str().unwrap()).unwrap();
        assert!(
            result2.created_dirs.is_empty(),
            "second run creates nothing"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn initialize_usb_seeds_reference_menu_defaults() {
        let tmp = tempfile::tempdir().expect("tempdir");
        initialize_usb(tmp.path().to_str().expect("usb path")).expect("initialize usb");

        let db_path = tmp
            .path()
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db");
        let conn = rusqlite::Connection::open(db_path).expect("open exportLibrary.db");
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
            .expect("apply SQLCipher key");

        let db_version: String = conn
            .query_row("SELECT dbVersion FROM property LIMIT 1", [], |row| {
                row.get(0)
            })
            .expect("property row");
        assert_eq!(db_version, "1000");

        let menu_rows = conn
            .prepare("SELECT menuItem_id, kind, name FROM menuItem ORDER BY menuItem_id")
            .expect("prepare menuItem")
            .query_map([], |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("query menuItem")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect menuItem");
        assert_eq!(
            menu_rows.iter().find(|(id, _, _)| *id == 17),
            Some(&(17, 132, "\u{FFFA}PLAYLIST\u{FFFB}".to_string()))
        );
        assert_eq!(
            menu_rows.iter().find(|(id, _, _)| *id == 5),
            Some(&(5, 133, "\u{FFFA}BPM\u{FFFB}".to_string()))
        );

        let category_rows = conn
            .prepare("SELECT category_id, menuItem_id, sequenceNo, isVisible FROM category ORDER BY category_id")
            .expect("prepare category")
            .query_map([], |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, i32>(3)?,
                ))
            })
            .expect("query category")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect category");
        // Init must produce the default visible set: 10 visible
        // (ARTIST/ALBUM/TRACK/KEY/PLAYLIST/HISTORY/SEARCH/MATCHING/FOLDER/
        // DATE ADDED) and 12 hidden, mirrored on the PDB side so older and
        // newer players see the same menu out of the box.
        assert_eq!(
            category_rows.len(),
            22,
            "expected 22 category rows in default layout"
        );
        let visible = category_rows
            .iter()
            .filter(|(_, _, _, vis)| *vis == 1)
            .count();
        assert_eq!(visible, 10, "default has 10 visible category rows");
        // Spot-check key rows.
        assert_eq!(
            category_rows.iter().find(|(id, _, _, _)| *id == 5),
            Some(&(5, 17, 5, 1)),
            "PLAYLIST visible at seq 5"
        );
        assert_eq!(
            category_rows.iter().find(|(id, _, _, _)| *id == 27),
            Some(&(27, 22, 10, 1)),
            "DATE ADDED visible at seq 10"
        );
        assert_eq!(
            category_rows.iter().find(|(id, _, _, _)| *id == 1),
            Some(&(1, 1, 0, 0)),
            "GENRE hidden by default"
        );

        let sort_rows = conn
            .prepare("SELECT sort_id, menuItem_id, sequenceNo, isVisible, isSelectedAsSubColumn FROM sort ORDER BY sort_id")
            .expect("prepare sort")
            .query_map([], |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, i32>(3)?,
                    row.get::<_, i32>(4)?,
                ))
            })
            .expect("query sort")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect sort");
        assert_eq!(
            sort_rows.iter().find(|(id, _, _, _, _)| *id == 0),
            Some(&(0, 25, 1, 1, 0))
        );
        assert_eq!(
            sort_rows.iter().find(|(id, _, _, _, _)| *id == 12),
            Some(&(12, 11, 7, 1, 0))
        );
        assert_eq!(
            sort_rows.iter().find(|(id, _, _, _, _)| *id == 17),
            Some(&(17, 22, 0, 0, 0))
        );
    }

    #[test]
    fn write_edb_sets_per_track_key_and_analysis_fields() {
        let tmp = std::env::temp_dir().join(format!("__test_edb_fields_{}__", Uuid::now_v7()));
        std::fs::create_dir_all(tmp.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
        let db_path = tmp
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db");
        let template_anlz = format!(
            "/{}/{}/P000/TEMPLATE/ANLZ0000.DAT",
            USB_VENDOR_ROOT_DIR, USB_ANALYSIS_DIR
        );
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            &format!(
                r#"
            CREATE TABLE playlist (
              playlist_id INTEGER PRIMARY KEY,
              name TEXT,
              attribute INTEGER,
              sequenceNo INTEGER
            );
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              title TEXT,
              path TEXT,
              analysisDataFilePath TEXT,
              bpmx100 INTEGER,
              key_id INTEGER,
              image_id INTEGER
            );
            CREATE TABLE playlist_content (
              playlist_id INTEGER,
              content_id INTEGER,
              sequenceNo INTEGER
            );
            CREATE TABLE "key" (
              key_id INTEGER PRIMARY KEY,
              name TEXT
            );
            INSERT INTO "key" (key_id, name) VALUES (1, '8A'), (2, '9A');
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (1, 'Template', 0, 1);
            INSERT INTO content (content_id, title, path, analysisDataFilePath, bpmx100, key_id, image_id)
              VALUES (1, 'Template', '/Contents/TEMPLATE.mp3', '{template_anlz}', 12800, 2, 99);
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (1, 1, 1);
            "#,
            ),
        )
        .unwrap();
        drop(conn);

        let playlist = ExportPlaylistData {
            id: "local-test".to_string(),
            name: "Field Test".to_string(),
            tracks: Vec::new(),
        };
        let manifest = ExportManifest {
            version: 1,
            generated_at: now(),
            playlist_id: "local-test".to_string(),
            playlist_name: "Field Test".to_string(),
            usb_root: tmp.to_string_lossy().to_string(),
            options: ExportToUsbOptions {
                include_artwork: false,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![ExportManifestTrack {
                id: "track-1".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 1,
                track_number: Some(1),
                title: "Alpha".to_string(),
                artist: "Artist".to_string(),
                album: None,
                bpm: Some(123.45),
                key: Some("8A".to_string()),
                source_path: "/tmp/source.mp3".to_string(),
                exported_path: "/Contents/Artist/UnknownAlbum/source.mp3".to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: true,
                owns_waveform: true,
                artwork_path: None,
                waveform_path: None,
                duration_ms: None,
            }],
        };

        let result = write_edb_playlist(&tmp, &playlist, &manifest, true);
        assert!(result.is_ok(), "db write failed: {result:?}");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let row = conn
            .query_row(
                r#"
                SELECT analysisDataFilePath, key_id, bpmx100, image_id
                FROM content
                WHERE path = ?1
                LIMIT 1
                "#,
                params!["/Contents/Artist/UnknownAlbum/source.mp3"],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .unwrap();
        assert!(
            row.0.is_none(),
            "analysisDataFilePath should be NULL when waveform is not exported"
        );
        assert_eq!(row.1, Some(1), "key_id should map from key name");
        assert_eq!(row.2, Some(12345), "bpmx100 should map from BPM");
        assert!(
            row.3.is_none(),
            "image_id should be NULL when artwork is not exported"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn write_edb_updates_existing_content_analysis_path() {
        let tmp =
            std::env::temp_dir().join(format!("__test_edb_update_existing_{}__", Uuid::now_v7()));
        std::fs::create_dir_all(tmp.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)).unwrap();
        let db_path = tmp
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db");
        let old_anlz = format!(
            "/{}/{}/P000/OLD/ANLZ0000.DAT",
            USB_VENDOR_ROOT_DIR, USB_ANALYSIS_DIR
        );
        let new_anlz = format!(
            "/{}/{}/P111/NEW/ANLZ0000.DAT",
            USB_VENDOR_ROOT_DIR, USB_ANALYSIS_DIR
        );
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            &format!(
                r#"
            CREATE TABLE playlist (
              playlist_id INTEGER PRIMARY KEY,
              name TEXT,
              attribute INTEGER,
              sequenceNo INTEGER
            );
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              title TEXT,
              path TEXT,
              analysisDataFilePath TEXT
            );
            CREATE TABLE playlist_content (
              playlist_id INTEGER,
              content_id INTEGER,
              sequenceNo INTEGER
            );
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (1, 'Template', 0, 1);
            INSERT INTO content (content_id, title, path, analysisDataFilePath)
              VALUES (10, 'Old', '/Contents/Artist One/Album One/Artist One - Album One - 03 Track One.mp3', '{old_anlz}');
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (1, 10, 1);
            "#,
            ),
        )
        .unwrap();
        drop(conn);

        let playlist = ExportPlaylistData {
            id: "local-test".to_string(),
            name: "Field Test".to_string(),
            tracks: Vec::new(),
        };
        let manifest = ExportManifest {
            version: 1,
            generated_at: now(),
            playlist_id: "local-test".to_string(),
            playlist_name: "Field Test".to_string(),
            usb_root: tmp.to_string_lossy().to_string(),
            options: ExportToUsbOptions {
                include_artwork: false,
                include_analysis: true,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: 1,
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: vec![ExportManifestTrack {
                id: "track-1".to_string(),
                master_db_id: None,
                master_content_id: None,
                content_link: None,
                position: 1,
                track_number: Some(3),
                title: "Track One".to_string(),
                artist: "Artist One".to_string(),
                album: Some("Album One".to_string()),
                bpm: None,
                key: None,
                source_path: "/tmp/source.mp3".to_string(),
                exported_path:
                    "/Contents/Artist One/Album One/Artist One - Album One - 03 Track One.mp3"
                        .to_string(),
                file_modified_at: None,
                file_size_bytes: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                title_for_search: None,
                kuvo_delivery_comment: None,
                dj_play_count: None,
                rating: None,
                color_id: None,
                artist_id_lyricist: None,
                artist_id_original_artist: None,
                artist_id_remixer: None,
                artist_id_composer: None,
                genre_id: None,
                genre: None,
                label_id: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                file_type: None,
                owns_exported_media: true,
                owns_artwork: true,
                owns_waveform: true,
                artwork_path: None,
                waveform_path: Some(new_anlz.clone()),
                duration_ms: None,
            }],
        };

        let result = write_edb_playlist(&tmp, &playlist, &manifest, true);
        assert!(result.is_ok(), "db write failed: {result:?}");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let stored = conn
            .query_row(
                "SELECT analysisDataFilePath FROM content WHERE content_id = 10",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap();
        assert_eq!(
            stored.as_deref(),
            Some(new_anlz.as_str()),
            "existing content row should get refreshed analysisDataFilePath"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn content_name_limits_match_vendor_style_caps() {
        let artist =
            "Very Long Artist Name That Should Definitely Be Trimmed For USB Export Compatibility";
        let album = "An Even Longer Album Name That Exceeds Typical External library Content Folder Constraints";
        let file_name =
            "Very Long Track Name That Should Also Be Trimmed To Device Safe Limits.mp3";
        let source = PathBuf::from(format!("/tmp/{file_name}"));
        let target = exported_media_target_path(
            Path::new("/tmp/usb/Contents"),
            &source,
            artist,
            Some(album),
            "Title",
            "mp3",
        );
        let rel = target
            .strip_prefix("/tmp/usb/Contents")
            .expect("strip prefix");
        let parts = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(parts.len(), 3, "expected artist/album/file under Contents");
        assert!(
            parts
                .iter()
                .all(|p| p.chars().count() <= CONTENT_FILENAME_MAX_LEN),
            "all path components should respect 48-char limit: {parts:?}"
        );
    }

    #[test]
    fn load_existing_analysis_paths_reads_content_map_and_detects_bundle() {
        let tmp =
            std::env::temp_dir().join(format!("__test_existing_analysis_map_{}__", Uuid::now_v7()));
        let rb = tmp.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        let anlz_dir = tmp
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_ANALYSIS_DIR)
            .join("P001")
            .join("ABCDEF01");
        std::fs::create_dir_all(&rb).unwrap();
        std::fs::create_dir_all(&anlz_dir).unwrap();
        std::fs::write(anlz_dir.join("ANLZ0000.DAT"), b"dat").unwrap();
        std::fs::write(anlz_dir.join("ANLZ0000.EXT"), b"ext").unwrap();
        std::fs::write(anlz_dir.join("ANLZ0000.2EX"), b"2ex").unwrap();

        let db_path = rb.join("exportLibrary.db");
        let anlz_path = format!(
            "/{}/{}/P001/ABCDEF01/ANLZ0000.DAT",
            USB_VENDOR_ROOT_DIR, USB_ANALYSIS_DIR
        );
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&format!(
            r#"
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              path TEXT,
              analysisDataFilePath TEXT
            );
            INSERT INTO content (content_id, path, analysisDataFilePath)
              VALUES (1, '/Contents/Artist/Album/Track.mp3', '{anlz_path}');
            "#,
        ))
        .unwrap();
        drop(conn);

        let mut warnings = Vec::<String>::new();
        let map = load_existing_analysis_paths_by_content_path(&tmp, &mut warnings);
        let anlz = map
            .get(&canonicalize_playlist_name(
                "/Contents/Artist/Album/Track.mp3",
            ))
            .cloned()
            .expect("analysis path should be mapped");
        assert!(
            analysis_bundle_exists(&tmp, &anlz),
            "bundle should be detected"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn artwork_path_to_data_url_encodes_jpeg() {
        let tmp = std::env::temp_dir().join("__test_artwork__.jpg");
        // Minimal JPEG: FFD8 header + FFD9 trailer
        std::fs::write(
            &tmp,
            &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x02, 0x00, 0x00, 0xFF, 0xD9],
        )
        .unwrap();
        let result = artwork_path_to_data_url(tmp.to_str().unwrap());
        assert!(result.is_some(), "should produce data URL for .jpg file");
        let url = result.unwrap();
        assert!(
            url.starts_with("data:image/jpeg;base64,"),
            "should have jpeg mime: {url}"
        );
        assert!(url.len() > 30, "should contain base64 data");
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn artwork_path_to_data_url_encodes_png() {
        let tmp = std::env::temp_dir().join("__test_artwork_png__.png");
        // Minimal PNG-like bytes (just needs non-empty content + .png extension)
        std::fs::write(&tmp, b"\x89PNG\r\n\x1a\n fake png data").unwrap();
        let result = artwork_path_to_data_url(tmp.to_str().unwrap());
        assert!(result.is_some(), "should produce data URL for .png file");
        let url = result.unwrap();
        assert!(
            url.starts_with("data:image/png;base64,"),
            "should have png mime: {url}"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn canonicalize_playlist_name_preserves_unicode_letters() {
        assert_eq!(canonicalize_playlist_name("劇団レコード"), "劇団レコード");
        assert_eq!(canonicalize_playlist_name("夢路歩"), "夢路歩");
        assert_eq!(
            canonicalize_playlist_name("Aural Imbalance"),
            "auralimbalance"
        );
    }

    #[test]
    fn canonicalize_playlist_name_distinguishes_unicode_artist_names() {
        assert_ne!(
            canonicalize_playlist_name("劇団レコード"),
            canonicalize_playlist_name("夢路歩")
        );
        assert_ne!(
            canonicalize_playlist_name("夢路歩"),
            canonicalize_playlist_name("かめりあ")
        );
    }

    #[test]
    fn repair_utf8_mojibake_recovers_double_decoded_utf8() {
        fn latin1_mojibake(value: &str) -> String {
            value
                .as_bytes()
                .iter()
                .map(|byte| char::from(*byte))
                .collect()
        }

        let original = "/Contents/ヒゲドライバー feat. ころねぽち/beatmania IIDX 31 EPOLIS ORIGINAL SOUNDTRACK/02 - ヒゲドライバー feat. ころねぽち - あるビー!.flac";
        let mojibake = latin1_mojibake(&latin1_mojibake(original));
        assert_eq!(repair_utf8_mojibake(&mojibake), original);
    }

    #[test]
    fn artwork_path_to_data_url_returns_none_for_unsupported_extension() {
        let tmp = std::env::temp_dir().join("__test_artwork_txt__.txt");
        std::fs::write(&tmp, b"not an image").unwrap();
        let result = artwork_path_to_data_url(tmp.to_str().unwrap());
        assert!(result.is_none(), "should return None for .txt file");
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn artwork_path_to_data_url_returns_none_for_empty_file() {
        let tmp = std::env::temp_dir().join("__test_artwork_empty__.jpg");
        std::fs::write(&tmp, b"").unwrap();
        let result = artwork_path_to_data_url(tmp.to_str().unwrap());
        assert!(result.is_none(), "should return None for empty file");
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn artwork_path_to_data_url_returns_none_for_missing_file() {
        let result = artwork_path_to_data_url("/tmp/__nonexistent_artwork_test_file__.jpg");
        assert!(result.is_none(), "should return None for missing file");
    }

    #[test]
    fn usb_tracks_from_edb_defer_artwork_data_url_to_lazy_load() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_artwork_db_{}__", Uuid::now_v7()));
        let rb = tmp.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        std::fs::create_dir_all(&rb).unwrap();

        // Create a fake artwork file on the USB
        let artwork_dir = tmp.join(USB_CONTENTS_DIR).join("artwork");
        std::fs::create_dir_all(&artwork_dir).unwrap();
        let artwork_file = artwork_dir.join("cover.jpg");
        std::fs::write(
            &artwork_file,
            &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x02, 0x00, 0x00, 0xFF, 0xD9],
        )
        .unwrap();

        // Create an eDB with proper schema (JOINed tables)
        let db_path = rb.join("exportLibrary.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE playlist (
              playlist_id INTEGER PRIMARY KEY,
              name TEXT,
              attribute INTEGER,
              sequenceNo INTEGER
            );
            CREATE TABLE artist (
              artist_id INTEGER PRIMARY KEY,
              name TEXT
            );
            CREATE TABLE album (
              album_id INTEGER PRIMARY KEY,
              name TEXT
            );
            CREATE TABLE "key" (
              key_id INTEGER PRIMARY KEY,
              name TEXT
            );
            CREATE TABLE image (
              image_id INTEGER PRIMARY KEY,
              path TEXT
            );
            CREATE TABLE content (
              content_id INTEGER PRIMARY KEY,
              title TEXT,
              artist_id_artist INTEGER,
              album_id INTEGER,
              key_id INTEGER,
              image_id INTEGER,
              path TEXT,
              analysisDataFilePath TEXT,
              bpmx100 INTEGER
            );
            CREATE TABLE playlist_content (
              playlist_id INTEGER,
              content_id INTEGER,
              sequenceNo INTEGER
            );
            INSERT INTO artist (artist_id, name) VALUES (1, 'Test Artist');
            INSERT INTO image (image_id, path) VALUES (1, '/Contents/artwork/cover.jpg');
            INSERT INTO playlist (playlist_id, name, attribute, sequenceNo) VALUES (1, 'ArtTest', 0, 1);
            INSERT INTO content (content_id, title, artist_id_artist, image_id, path, bpmx100)
              VALUES (1, 'Track With Art', 1, 1, '/Contents/track.mp3', 12800);
            INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (1, 1, 1);
            "#,
        )
        .unwrap();
        drop(conn);

        // Use the function that reads tracks from export DB
        let mut warnings = Vec::new();
        let playlists = crate::edb::try_read_playlists_with_metadata_from_edb(&tmp, &mut warnings);
        assert!(
            playlists.is_some(),
            "should read playlists from export db, warnings: {warnings:?}"
        );
        let playlists = playlists.unwrap();

        let track = playlists
            .values()
            .flat_map(|playlist| playlist.tracks.iter())
            .find(|t| t.title == "Track With Art");
        assert!(track.is_some(), "should find the test track");
        let track = track.unwrap();

        assert!(
            track.artwork_path.is_some(),
            "USB track artwork_path should be populated when artwork exists"
        );
        let artwork_path = track.artwork_path.as_ref().unwrap();
        assert!(
            artwork_path.ends_with("/Contents/artwork/cover.jpg"),
            "artwork_path should resolve to USB artwork location: {artwork_path}"
        );
        assert!(
            track.artwork_data_url.is_none(),
            "USB import should defer artwork_data_url loading until inspect/hydration"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    // --- resolve_usb_side_path: path traversal protection ---

    #[test]
    fn resolve_usb_side_path_allows_valid_vendor_relative() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_resolve_{}__", Uuid::now_v7()));
        let vendor_root = tmp.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        std::fs::create_dir_all(&vendor_root).unwrap();
        std::fs::write(vendor_root.join("test.db"), b"data").unwrap();

        let vendor_rel = format!("/{USB_VENDOR_ROOT_DIR}/{USB_VENDOR_DB_DIR}/test.db");
        let result = resolve_usb_side_path(&tmp, &vendor_rel);
        assert!(
            result.is_some(),
            "should resolve valid vendor-relative path"
        );
        let resolved = result.unwrap();
        assert!(
            resolved.contains(USB_VENDOR_ROOT_DIR),
            "resolved path should contain vendor root segment: {resolved}"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_usb_side_path_allows_valid_contents_relative() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_resolve_c_{}__", Uuid::now_v7()));
        let contents = tmp.join(USB_CONTENTS_DIR).join("Artist");
        std::fs::create_dir_all(&contents).unwrap();
        std::fs::write(contents.join("track.mp3"), b"audio").unwrap();

        let result = resolve_usb_side_path(&tmp, "/Contents/Artist/track.mp3");
        assert!(
            result.is_some(),
            "should resolve valid Contents-relative path"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_usb_side_path_rejects_traversal_outside_root() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_traversal_{}__", Uuid::now_v7()));
        let vendor_root = tmp.join(USB_VENDOR_ROOT_DIR);
        std::fs::create_dir_all(&vendor_root).unwrap();

        // Attempt to escape via ../
        let traversal = format!("{USB_VENDOR_ROOT_DIR}/../../etc/passwd");
        let result = resolve_usb_side_path(&tmp, &traversal);
        // Should be None (blocked by containment check) since resolved path escapes root
        assert!(
            result.is_none(),
            "path traversal via ../ should be rejected, got: {:?}",
            result
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_usb_side_path_rejects_contents_traversal() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_ct_{}__", Uuid::now_v7()));
        let contents = tmp.join(USB_CONTENTS_DIR);
        std::fs::create_dir_all(&contents).unwrap();

        let result = resolve_usb_side_path(&tmp, "Contents/../../etc/shadow");
        assert!(
            result.is_none(),
            "Contents traversal via ../ should be rejected, got: {:?}",
            result
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_usb_side_path_empty_returns_none() {
        let tmp = std::env::temp_dir();
        assert!(resolve_usb_side_path(&tmp, "").is_none());
        assert!(resolve_usb_side_path(&tmp, "   ").is_none());
    }

    #[test]
    fn resolve_usb_side_path_absolute_outside_root_is_passthrough() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_abs_{}__", Uuid::now_v7()));
        std::fs::create_dir_all(&tmp).unwrap();

        // Absolute paths that don't start with vendor/contents roots are returned as-is
        let result = resolve_usb_side_path(&tmp, "/etc/hostname");
        // This is an absolute path → goes through the is_absolute() branch (passthrough)
        assert!(result.is_some(), "absolute paths are passed through");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_usb_side_path_non_relative_traversal_blocked() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_nrt_{}__", Uuid::now_v7()));
        let vendor_root = tmp.join(USB_VENDOR_ROOT_DIR);
        std::fs::create_dir_all(&vendor_root).unwrap();

        // Non-vendor/contents relative path with traversal
        let result = resolve_usb_side_path(&tmp, "../../etc/passwd");
        assert!(
            result.is_none(),
            "non-USB-relative traversal should be rejected, got: {:?}",
            result
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn pwav_encode_decode_roundtrip_preserves_values() {
        // Simulate PWAV encoding with band=7 (legacy) — decoder extracts low 5 bits regardless of band
        let original_peaks: Vec<u8> = vec![0, 25, 50, 75, 100];
        let levels: Vec<u8> = original_peaks
            .iter()
            .map(|&v| ((u16::from(v) * 31) / 100) as u8)
            .collect();
        let encoded: Vec<u8> = levels.iter().map(|&v| (7u8 << 5) | (v & 0x1F)).collect();

        // Decode with downsample_waveform_payload (same bin count = no resampling)
        let decoded = downsample_waveform_payload(&encoded, encoded.len());

        // Each decoded value should be the 5-bit level scaled back to 0-100
        for (i, &dec) in decoded.iter().enumerate() {
            let expected_level = levels[i];
            let expected_percent = ((f32::from(expected_level) * 100.0) / 31.0)
                .round()
                .clamp(0.0, 100.0) as u8;
            assert_eq!(
                dec, expected_percent,
                "PWAV roundtrip mismatch at index {i}: level={expected_level}, expected={expected_percent}, got={dec}"
            );
        }
    }

    #[test]
    fn pwv3_encode_decode_roundtrip_preserves_values() {
        // PWV3 encoding: peaks_to_levels then pack as ((6<<5) | (v&0x1F))
        let original_peaks: Vec<u8> = vec![10, 30, 60, 90, 100];
        let levels: Vec<u8> = original_peaks
            .iter()
            .map(|&v| ((u16::from(v) * 31) / 100) as u8)
            .collect();
        let encoded: Vec<u8> = levels.iter().map(|&v| (6u8 << 5) | (v & 0x1F)).collect();

        let decoded = downsample_waveform_payload(&encoded, encoded.len());

        for (i, &dec) in decoded.iter().enumerate() {
            let expected_level = levels[i];
            let expected_percent = ((f32::from(expected_level) * 100.0) / 31.0)
                .round()
                .clamp(0.0, 100.0) as u8;
            assert_eq!(
                dec, expected_percent,
                "PWV3 roundtrip mismatch at index {i}: level={expected_level}, expected={expected_percent}, got={dec}"
            );
        }
    }

    #[test]
    fn anlz_dat_file_roundtrip_extracts_correct_preview() {
        use super::super::anlz::{WaveformData, build_anlz_dat_file};

        let peaks: Vec<u8> = (0..100).map(|i| (i * 100 / 99).min(100) as u8).collect();
        let dat_bytes = build_anlz_dat_file(&WaveformData::from_peaks(peaks), "", None, None);

        // extract_waveform_preview_from_anlz_bytes should find PWAV and decode it
        let preview = extract_waveform_preview_from_anlz_bytes(&dat_bytes, 50);
        assert!(preview.is_some(), "should extract preview from DAT file");
        let preview = preview.unwrap();
        assert_eq!(preview.len(), 50);
        // Values should be in 0-100 range and not all zero
        assert!(
            preview.iter().any(|&v| v > 0),
            "preview should have non-zero values"
        );
        assert!(
            preview.iter().all(|&v| v <= 100),
            "all values should be <= 100"
        );
    }
}
