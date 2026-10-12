//! Track identity keys and path normalization shared by the checks.

use super::*;

pub(crate) fn track_identity_key(
    file_path: &str,
    title: &str,
    artist: &str,
    id_fallback: Option<&str>,
) -> String {
    let path = normalize_track_path_for_identity(file_path);
    if !path.is_empty() {
        return format!("path:{path}");
    }

    let meta = build_meta_key(title, artist);
    if meta != "meta:|" {
        return meta;
    }

    if let Some(id) = id_fallback.map(str::trim).filter(|id| !id.is_empty()) {
        return format!("id:{id}");
    }

    "unknown".to_string()
}

/// Build a meta-key string from title and artist (same format as track_identity_key fallback).
pub(crate) fn build_meta_key(title: &str, artist: &str) -> String {
    format!(
        "meta:{}|{}",
        canonicalize_playlist_name(title),
        canonicalize_playlist_name(artist)
    )
}

pub(super) fn table_type_label(table_type: u32) -> &'static str {
    match table_type {
        0 => "track",
        2 => "artist",
        3 => "album",
        _ => "row",
    }
}

pub(super) fn normalize_contents_path(value: &str, lowercase_output: bool) -> String {
    let normalized = repair_utf8_mojibake(value.trim()).replace('\\', "/");
    if normalized.is_empty() {
        return String::new();
    }
    let lowered = normalized.to_ascii_lowercase();

    let candidate = if let Some(idx) = lowered.rfind("/contents/") {
        normalized[idx..].to_string()
    } else if lowered.starts_with("contents/") {
        format!("/{normalized}")
    } else {
        normalized
    };

    if lowercase_output {
        candidate.to_ascii_lowercase()
    } else {
        candidate
    }
}

pub(crate) fn normalize_track_path_for_identity(value: &str) -> String {
    normalize_contents_path(value, true)
}

pub(crate) fn normalize_analysis_path_for_identity(value: &str) -> String {
    let normalized = repair_utf8_mojibake(value.trim()).replace('\\', "/");
    if normalized.is_empty() {
        return String::new();
    }
    canonicalize_playlist_name(&normalized)
}

pub(super) fn unique_preserve_order(input: &[String]) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::<String>::new();
    for item in input {
        if seen.insert(item.clone()) {
            out.push(item.clone());
        }
    }
    out
}

pub(crate) fn normalize_pdb_path_for_edb_lookup(value: &str) -> String {
    normalize_contents_path(value, false)
}

/// Trim only the "trailing space before the extension" filename alias (e.g.
/// "Track .mp3" vs "Track.mp3"). Deliberately narrower than
/// `normalize_path_for_contents_match`, which also collapses a trailing
/// `-<digits>` stem suffix — appropriate for that function's alias-grouping
/// callers, but wrong for a strict indexed-vs-on-disk presence comparison:
/// it collapses genuinely distinct files whose real filenames happen to end
/// in a numeric suffix (release years, remix/version numbers, catalog
/// codes), which are common enough in a large library to produce mass false
/// "missing"/"unindexed" results instead of fixing the one alias case.
pub(super) fn normalize_path_for_strict_presence_match(value: &str) -> String {
    let normalized = normalize_pdb_path_for_edb_lookup(value);
    if normalized.is_empty() {
        return normalized;
    }
    let path = std::path::Path::new(&normalized);
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let alias_stripped = if let Some((stem, ext)) = file.rsplit_once('.') {
        format!("{}.{ext}", stem.trim_end())
    } else {
        file.to_string()
    };
    if parent.is_empty() {
        alias_stripped
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), alias_stripped)
    }
}

/// Case-insensitive key for a normalized `/Contents/...` path, for HashSet membership
/// checks only. Exported USB drives are FAT32/exFAT, which resolve paths
/// case-insensitively — a folder created under an artist tag's original casing keeps that
/// casing on disk even after the tag is later corrected, since FAT doesn't rename an
/// existing directory entry just because a later write used different casing for the
/// "same" (case-insensitively) path. Presence/coverage comparisons between PDB/eDB-recorded
/// paths and real on-disk paths must not be case-sensitive, or a casing drift like that
/// produces a false "missing"/"unindexed" pair even though the file is perfectly reachable.
/// Never apply this to a path used for display or file I/O — only for `.contains()` checks,
/// so warnings/output still show the real on-disk casing.
pub(crate) fn contents_path_match_key(normalized: &str) -> String {
    normalized.to_ascii_lowercase()
}

pub(crate) fn normalize_path_for_contents_match(value: &str) -> String {
    let normalized = normalize_pdb_path_for_edb_lookup(value);
    if normalized.is_empty() {
        return normalized;
    }
    let path = std::path::Path::new(&normalized);
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let alias_stripped = if let Some((stem, ext)) = file.rsplit_once('.') {
        // Trim trailing whitespace: some DJ software appends a space before the
        // extension as a naming-conflict alias (e.g. "Track .mp3"). Both the
        // alias form and the original may exist on disk, so normalise both to
        // the same shape for comparison.
        let stem = stem.trim_end();
        let next_stem = stem
            .rsplit_once('-')
            .and_then(|(prefix, suffix)| {
                // `suffix.chars().all(...)` is vacuously true for an empty suffix, so a
                // stem ending in a bare "-" (no digits after it, e.g. a truncation cutoff
                // landing exactly on a hyphen) was previously stripped as if it were a
                // "-<digits>" dedup suffix, silently dropping a real trailing hyphen.
                if !prefix.is_empty()
                    && !suffix.is_empty()
                    && suffix.chars().all(|c| c.is_ascii_digit())
                {
                    Some(prefix.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| stem.to_string());
        format!("{next_stem}.{ext}")
    } else {
        file.to_string()
    };
    if parent.is_empty() {
        alias_stripped
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), alias_stripped)
    }
}

pub(super) fn normalize_usb_path_for_parity(value: &str) -> String {
    repair_utf8_mojibake(value.trim())
        .replace('\\', "/")
        .to_ascii_lowercase()
}
