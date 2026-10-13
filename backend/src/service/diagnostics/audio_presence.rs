//! Which audio files the databases index and which are on the USB. The parity
//! report's "Indexed audio file presence" check, the diagnostics' PDB/eDB
//! contents comparison and the audio repairs (relink, add unindexed, remove
//! duplicate copies, remove missing references) all use this one rule, so a
//! file the report lists always has a matching repair and a repair leaves
//! nothing the report still lists.

use std::collections::BTreeMap;
use std::path::Path;

use super::*;

/// Match key of an audio path: the trailing-space alias trimmed (see
/// `normalize_path_for_strict_presence_match`) and lowercased, because
/// FAT32/exFAT resolve paths case-insensitively. Distinct files whose names
/// differ only by a `-1`/`-2` suffix keep distinct keys: rekordbox writes a
/// second copy of a track under such a name.
pub(crate) fn audio_path_key(path: &str) -> String {
    contents_path_match_key(&normalize_path_for_strict_presence_match(path))
}

/// Indexed vs on-USB audio, from `scan_audio_presence`.
#[derive(Debug, Default, Clone)]
pub(crate) struct AudioPresence {
    /// Number of distinct indexed paths (PDB track rows + eDB content rows).
    pub(crate) indexed_count: usize,
    /// Files under `Contents/` (USB-relative, as on disk) an indexed path
    /// points at, sorted.
    pub(crate) indexed_on_usb: Vec<String>,
    /// Files under `Contents/` no indexed path points at, sorted.
    pub(crate) unindexed: Vec<String>,
    /// Indexed paths (normalized, one per key) with no file on the USB, sorted.
    pub(crate) missing: Vec<String>,
}

impl AudioPresence {
    /// `audio_path_key` of every missing path.
    pub(crate) fn missing_keys(&self) -> HashSet<String> {
        self.missing.iter().map(|p| audio_path_key(p)).collect()
    }
}

/// Compares the paths the PDB track rows and the eDB content rows
/// (`edb_paths`, from `collect_edb_audio_paths`) point at with the audio files
/// under `Contents/`. A path either database holds counts as indexed, so a
/// track whose PDB row and eDB row name different copies keeps both files.
pub(crate) fn scan_audio_presence(
    usb_root: &Path,
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_paths: &[String],
) -> AudioPresence {
    let mut indexed = BTreeMap::<String, String>::new();
    for path in parsed
        .tracks
        .iter()
        .map(|t| t.track_file_path.as_str())
        .chain(edb_paths.iter().map(String::as_str))
    {
        let normalized = normalize_path_for_strict_presence_match(path);
        if !normalized.is_empty() {
            indexed
                .entry(contents_path_match_key(&normalized))
                .or_insert(normalized);
        }
    }
    let mut on_usb_keys = HashSet::<String>::new();
    let mut out = AudioPresence {
        indexed_count: indexed.len(),
        ..AudioPresence::default()
    };
    for file in collect_contents_audio_files(usb_root) {
        let key = audio_path_key(&file);
        if key.is_empty() {
            continue;
        }
        if indexed.contains_key(&key) {
            out.indexed_on_usb.push(file);
        } else {
            out.unindexed.push(file);
        }
        on_usb_keys.insert(key);
    }
    out.indexed_on_usb.sort();
    out.unindexed.sort();
    out.missing = indexed
        .into_iter()
        .filter(|(key, _)| !on_usb_keys.contains(key))
        .map(|(_, path)| path)
        .collect();
    out.missing.sort();
    out
}
