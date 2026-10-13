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
/// "Track .mp3" vs "Track.mp3"). A trailing `-<digits>` stem suffix is kept:
/// files whose names differ only by it are distinct files (rekordbox writes a
/// second copy of a track as `Track-1.mp3`; release years, version numbers and
/// catalog codes end that way too). Match through `audio_path_key`.
pub(crate) fn normalize_path_for_strict_presence_match(value: &str) -> String {
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

pub(super) fn normalize_usb_path_for_parity(value: &str) -> String {
    repair_utf8_mojibake(value.trim())
        .replace('\\', "/")
        .to_ascii_lowercase()
}
