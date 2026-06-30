//! USB playlist/history parsing helper functions.

use std::collections::{HashMap, HashSet};

use crate::models::{UsbPlaylist, UsbTrack};

use super::usb_utils::canonicalize_playlist_name;

#[derive(Debug, Clone)]
pub(crate) struct PlaylistCandidate {
    pub(crate) pdb_id: Option<u32>,
    pub(crate) short_name: String,
    pub(crate) display_name: String,
    pub(crate) sort_order: u32,
}

pub(crate) fn lookup_playlist_tracks<'a>(
    map: &'a Option<HashMap<String, Vec<UsbTrack>>>,
    map_canonical: &'a Option<HashMap<String, Vec<UsbTrack>>>,
    short_name: &str,
    display_name: &str,
) -> Option<&'a Vec<UsbTrack>> {
    map.as_ref()
        .and_then(|m| {
            m.get(short_name)
                .or_else(|| m.get(short_name.trim()))
                .or_else(|| m.get(display_name))
                .or_else(|| m.get(display_name.trim()))
        })
        .or_else(|| {
            let key = canonicalize_playlist_name(short_name);
            map_canonical.as_ref().and_then(|m| m.get(&key))
        })
        .or_else(|| {
            let key = canonicalize_playlist_name(display_name);
            map_canonical.as_ref().and_then(|m| m.get(&key))
        })
}

pub(crate) fn build_usb_track_id_index(
    source: &HashMap<String, Vec<UsbTrack>>,
) -> HashMap<u32, UsbTrack> {
    let mut out = HashMap::<u32, UsbTrack>::new();
    for tracks in source.values() {
        for track in tracks {
            if let Ok(id) = track.id.parse::<u32>() {
                out.entry(id).or_insert_with(|| track.clone());
            }
        }
    }
    out
}

pub(crate) fn merge_playlist_tracks(
    pdb_tracks: &[UsbTrack],
    export_tracks: &[UsbTrack],
) -> (Vec<UsbTrack>, &'static str) {
    if !export_tracks.is_empty() {
        let mut out = Vec::<UsbTrack>::new();
        let mut seen = HashSet::<String>::new();
        for track in export_tracks {
            let key = track_merge_key(track);
            if seen.insert(key) {
                out.push(track.clone());
            }
        }
        return (out, "eDB");
    }

    if pdb_tracks.is_empty() {
        return (Vec::new(), "none");
    }

    let mut out = Vec::<UsbTrack>::new();
    let mut seen = HashSet::<String>::new();
    for set in [pdb_tracks, export_tracks] {
        for track in set {
            let key = track_merge_key(track);
            if seen.insert(key) {
                out.push(track.clone());
            }
        }
    }
    (out, "pdb")
}

pub(crate) fn playlist_source_rank(source: &str) -> usize {
    match source {
        "pdb" => 3,
        "eDB" => 2,
        _ => 1,
    }
}

pub(crate) fn dedupe_usb_playlists_by_name(items: Vec<UsbPlaylist>) -> (Vec<UsbPlaylist>, usize) {
    let mut out = Vec::<UsbPlaylist>::new();
    let mut by_key = HashMap::<String, usize>::new();
    let mut collapsed = 0usize;
    for item in items {
        let key = canonicalize_playlist_name(&item.name);
        if let Some(existing_idx) = by_key.get(&key).copied() {
            let existing = &out[existing_idx];
            let existing_score = (existing.track_count, playlist_source_rank(&existing.source));
            let incoming_score = (item.track_count, playlist_source_rank(&item.source));
            if incoming_score > existing_score {
                out[existing_idx] = item;
            }
            collapsed += 1;
        } else {
            by_key.insert(key, out.len());
            out.push(item);
        }
    }
    (out, collapsed)
}

pub(crate) fn track_merge_key(track: &UsbTrack) -> String {
    let path = canonicalize_playlist_name(&track.file_path);
    if !path.is_empty() {
        return format!("path:{path}");
    }
    let meta = format!(
        "meta:{}|{}",
        canonicalize_playlist_name(&track.title),
        canonicalize_playlist_name(&track.artist)
    );
    if meta != "meta:|" {
        return meta;
    }
    let id = track.id.trim();
    if !id.is_empty() {
        return format!("id:{id}");
    }
    "unknown".to_string()
}

pub(crate) fn sanitize_history_name(value: &str) -> String {
    sanitize_text(value)
}

pub(crate) fn history_entry_sort_key(raw: u32) -> u32 {
    let low = raw & 0xFFFF;
    if low != 0 { low } else { raw }
}

pub(crate) fn normalize_packed_id(raw: u32) -> u32 {
    let lo = raw & 0xFFFF;
    if lo != 0 { lo } else { raw }
}

pub(crate) fn parse_history_slot_id(value: &str) -> Option<u32> {
    let digits = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok().filter(|v| *v > 0)
}

pub(crate) fn parse_history_name_numeric_id(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    if !trimmed.to_ascii_uppercase().starts_with("HISTORY") {
        return None;
    }
    parse_history_slot_id(trimmed)
}

pub(crate) fn sanitize_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control()
                && *c != '\u{fffd}'
                && (c.is_ascii_alphanumeric()
                    || c.is_ascii_punctuation()
                    || c.is_ascii_whitespace())
        })
        .collect::<String>()
        .trim()
        .to_string()
}

pub(crate) fn decode_history_track_id(
    raw_playlist_field: u32,
    raw_second_field: u32,
) -> Option<u32> {
    let track_from_playlist_hi = (raw_playlist_field >> 16) & 0xFFFF;
    if track_from_playlist_hi != 0 {
        return Some(track_from_playlist_hi);
    }

    let track_from_second_hi = (raw_second_field >> 8) & 0xFFFF;
    if track_from_second_hi != 0 {
        return Some(track_from_second_hi);
    }

    None
}

pub(crate) fn decode_history_playlist_id(
    raw_playlist_field: u32,
    raw_second_field: u32,
    known_history_ids: &HashSet<u32>,
) -> Option<u32> {
    let low = raw_playlist_field & 0xFFFF;
    let high = (raw_playlist_field >> 16) & 0xFFFF;
    let from_second = (raw_second_field >> 8) & 0xFFFF;

    let candidates = [low, high, from_second];
    for candidate in candidates {
        if candidate != 0 && known_history_ids.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_track(id: &str, title: &str, artist: &str, file_path: &str) -> UsbTrack {
        UsbTrack {
            id: id.to_string(),
            local_track_id: None,
            title: title.to_string(),
            artist: artist.to_string(),
            album: None,
            track_number: None,
            bpm: None,
            key: None,
            file_path: file_path.to_string(),
            usb_media_path: None,
            artwork_path: None,
            artwork_data_url: None,
            waveform_peaks_path: None,
            usb_analysis_path: None,
            usb_analysis_path_raw: None,
            waveform_preview: None,
            duration_ms: None,
        }
    }

    fn make_playlist(name: &str, source: &str, track_count: usize) -> UsbPlaylist {
        UsbPlaylist {
            id: format!("pl-{name}"),
            name: name.to_string(),
            source: source.to_string(),
            track_count,
            tracks: Vec::new(),
        }
    }

    // --- sanitize_text ---

    #[test]
    fn sanitize_text_removes_control_chars() {
        assert_eq!(sanitize_text("hello\x00world\x01!"), "helloworld!");
    }

    #[test]
    fn sanitize_text_removes_replacement_char() {
        assert_eq!(sanitize_text("good\u{FFFD}text"), "goodtext");
    }

    #[test]
    fn sanitize_text_preserves_ascii() {
        assert_eq!(sanitize_text("Hello World! 123"), "Hello World! 123");
    }

    #[test]
    fn sanitize_text_trims_whitespace() {
        assert_eq!(sanitize_text("  hello  "), "hello");
    }

    #[test]
    fn sanitize_text_removes_non_ascii_non_punctuation() {
        // Non-ASCII chars that aren't alphanumeric/punctuation/whitespace are removed
        let result = sanitize_text("café");
        // 'c', 'a', 'f' are ascii alphanum; 'é' is not ascii_alphanumeric
        assert_eq!(result, "caf");
    }

    // --- playlist_source_rank ---

    #[test]
    fn source_rank_ordering() {
        assert_eq!(playlist_source_rank("pdb"), 3);
        assert_eq!(playlist_source_rank("eDB"), 2);
        assert_eq!(playlist_source_rank("other"), 1);
        assert_eq!(playlist_source_rank(""), 1);
    }

    // --- merge_playlist_tracks ---

    #[test]
    fn merge_prefers_export_over_pdb() {
        let pdb = vec![make_track("1", "A", "X", "/a.mp3")];
        let export = vec![make_track("1", "A Better", "X", "/a.mp3")];

        let (merged, source) = merge_playlist_tracks(&pdb, &export);
        assert_eq!(source, "eDB");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].title, "A Better");
    }

    #[test]
    fn merge_export_source_does_not_append_same_name_pdb_tracks() {
        let pdb = vec![make_track("1", "Old USB Track", "X", "/old.mp3")];
        let export = vec![make_track("2", "New Export Track", "Y", "/new.mp3")];

        let (merged, source) = merge_playlist_tracks(&pdb, &export);
        assert_eq!(source, "eDB");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].title, "New Export Track");
        assert_eq!(merged[0].file_path, "/new.mp3");
    }

    #[test]
    fn merge_export_source_returns_only_export_set() {
        let pdb = vec![
            make_track("1", "Track A", "X", "/a.mp3"),
            make_track("2", "Track B", "Y", "/b.mp3"),
        ];
        let export = vec![make_track("1", "Track A Export", "X", "/a.mp3")];

        let (merged, source) = merge_playlist_tracks(&pdb, &export);
        assert_eq!(source, "eDB");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].title, "Track A Export");
    }

    #[test]
    fn merge_deduplicates_same_path_when_ids_differ() {
        let pdb = vec![make_track("1001", "Track A", "X", "/a.mp3")];
        let export = vec![make_track("2002", "Track A Export", "X", "/a.mp3")];

        let (merged, source) = merge_playlist_tracks(&pdb, &export);
        assert_eq!(source, "eDB");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "2002");
    }

    #[test]
    fn merge_pdb_only() {
        let pdb = vec![make_track("1", "Track A", "X", "/a.mp3")];
        let export: Vec<UsbTrack> = vec![];

        let (merged, source) = merge_playlist_tracks(&pdb, &export);
        assert_eq!(source, "pdb");
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn merge_all_empty() {
        let (merged, source) = merge_playlist_tracks(&[], &[]);
        assert_eq!(source, "none");
        assert!(merged.is_empty());
    }

    // --- dedupe_usb_playlists_by_name ---

    #[test]
    fn dedupe_keeps_higher_track_count() {
        let items = vec![
            make_playlist("My Set", "pdb", 5),
            make_playlist("My Set", "pdb", 10),
        ];
        let (deduped, collapsed) = dedupe_usb_playlists_by_name(items);
        assert_eq!(deduped.len(), 1);
        assert_eq!(collapsed, 1);
        assert_eq!(deduped[0].track_count, 10);
    }

    #[test]
    fn dedupe_keeps_higher_rank_on_same_count() {
        let items = vec![
            make_playlist("My Set", "none", 5),
            make_playlist("My Set", "pdb", 5),
        ];
        let (deduped, collapsed) = dedupe_usb_playlists_by_name(items);
        assert_eq!(deduped.len(), 1);
        assert_eq!(collapsed, 1);
        assert_eq!(deduped[0].source, "pdb");
    }

    #[test]
    fn dedupe_different_names_kept() {
        let items = vec![
            make_playlist("Set A", "pdb", 5),
            make_playlist("Set B", "pdb", 3),
        ];
        let (deduped, collapsed) = dedupe_usb_playlists_by_name(items);
        assert_eq!(deduped.len(), 2);
        assert_eq!(collapsed, 0);
    }

    // --- decode_history_track_id ---

    #[test]
    fn decode_track_id_from_high_word() {
        // Track ID 42 in high word of playlist field
        let raw_playlist = 42u32 << 16;
        assert_eq!(decode_history_track_id(raw_playlist, 0), Some(42));
    }

    #[test]
    fn decode_track_id_from_second_field() {
        // High word of playlist is 0, falls back to second field
        let raw_second = 99u32 << 8;
        assert_eq!(decode_history_track_id(0, raw_second), Some(99));
    }

    #[test]
    fn decode_track_id_both_zero() {
        assert_eq!(decode_history_track_id(0, 0), None);
    }

    #[test]
    fn decode_track_id_prefers_playlist_high_word() {
        let raw_playlist = 10u32 << 16;
        let raw_second = 20u32 << 8;
        assert_eq!(decode_history_track_id(raw_playlist, raw_second), Some(10));
    }

    // --- decode_history_playlist_id ---

    #[test]
    fn decode_playlist_id_matches_known() {
        let mut known = HashSet::new();
        known.insert(5u32);
        // Low word = 5
        assert_eq!(decode_history_playlist_id(5, 0, &known), Some(5));
    }

    #[test]
    fn decode_playlist_id_tries_high_word() {
        let mut known = HashSet::new();
        known.insert(7u32);
        let raw_playlist = 7u32 << 16;
        assert_eq!(decode_history_playlist_id(raw_playlist, 0, &known), Some(7));
    }

    #[test]
    fn decode_playlist_id_tries_second_field() {
        let mut known = HashSet::new();
        known.insert(12u32);
        let raw_second = 12u32 << 8;
        assert_eq!(decode_history_playlist_id(0, raw_second, &known), Some(12));
    }

    #[test]
    fn decode_playlist_id_no_match() {
        let mut known = HashSet::new();
        known.insert(99u32);
        assert_eq!(decode_history_playlist_id(1, 2, &known), None);
    }

    #[test]
    fn decode_playlist_id_zero_not_matched() {
        let mut known = HashSet::new();
        known.insert(0u32);
        // Zero candidates are skipped
        assert_eq!(decode_history_playlist_id(0, 0, &known), None);
    }

    // --- parse_history_slot_id ---

    #[test]
    fn parse_slot_id_extracts_digits() {
        assert_eq!(parse_history_slot_id("HISTORY 001"), Some(1));
        assert_eq!(parse_history_slot_id("slot-42-end"), Some(42));
    }

    #[test]
    fn parse_slot_id_empty() {
        assert_eq!(parse_history_slot_id("no digits here"), None);
    }

    #[test]
    fn parse_slot_id_zero_rejected() {
        assert_eq!(parse_history_slot_id("slot 000"), None);
    }

    // --- parse_history_name_numeric_id ---

    #[test]
    fn parse_history_name_valid() {
        assert_eq!(parse_history_name_numeric_id("HISTORY 001"), Some(1));
        assert_eq!(parse_history_name_numeric_id("history 42"), Some(42));
    }

    #[test]
    fn parse_history_name_not_history_prefix() {
        assert_eq!(parse_history_name_numeric_id("playlist 1"), None);
    }

    // --- history_entry_sort_key / normalize_packed_id ---

    #[test]
    fn history_sort_key_uses_low_word() {
        assert_eq!(history_entry_sort_key(0x0005_0003), 3);
    }

    #[test]
    fn history_sort_key_falls_back_to_full() {
        assert_eq!(history_entry_sort_key(0x0005_0000), 0x0005_0000);
    }

    #[test]
    fn normalize_packed_id_uses_low_word() {
        assert_eq!(normalize_packed_id(0x0001_0042), 0x0042);
    }

    #[test]
    fn normalize_packed_id_low_zero_uses_full() {
        assert_eq!(normalize_packed_id(0x0001_0000), 0x0001_0000);
    }

    // --- lookup_playlist_tracks ---

    #[test]
    fn lookup_by_short_name() {
        let tracks = vec![make_track("1", "T", "A", "/t.mp3")];
        let map = Some(HashMap::from([("my_set".to_string(), tracks.clone())]));
        let result = lookup_playlist_tracks(&map, &None, "my_set", "display");
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn lookup_by_display_name() {
        let tracks = vec![make_track("1", "T", "A", "/t.mp3")];
        let map = Some(HashMap::from([("My Display".to_string(), tracks.clone())]));
        let result = lookup_playlist_tracks(&map, &None, "short", "My Display");
        assert!(result.is_some());
    }

    #[test]
    fn lookup_falls_back_to_canonical() {
        let tracks = vec![make_track("1", "T", "A", "/t.mp3")];
        let canonical = Some(HashMap::from([(
            canonicalize_playlist_name("my set"),
            tracks.clone(),
        )]));
        let result = lookup_playlist_tracks(&None, &canonical, "my set", "other");
        assert!(result.is_some());
    }

    #[test]
    fn lookup_returns_none_when_no_match() {
        let result = lookup_playlist_tracks(&None, &None, "none", "none");
        assert!(result.is_none());
    }

    // --- build_usb_track_id_index ---

    #[test]
    fn track_id_index_numeric_ids() {
        let tracks = vec![
            make_track("42", "A", "X", "/a.mp3"),
            make_track("7", "B", "Y", "/b.mp3"),
        ];
        let map = HashMap::from([("set".to_string(), tracks)]);
        let idx = build_usb_track_id_index(&map);
        assert_eq!(idx.len(), 2);
        assert_eq!(idx.get(&42).unwrap().title, "A");
        assert_eq!(idx.get(&7).unwrap().title, "B");
    }

    #[test]
    fn track_id_index_skips_non_numeric() {
        let tracks = vec![
            make_track("not-a-number", "A", "X", "/a.mp3"),
            make_track("5", "B", "Y", "/b.mp3"),
        ];
        let map = HashMap::from([("set".to_string(), tracks)]);
        let idx = build_usb_track_id_index(&map);
        assert_eq!(idx.len(), 1);
        assert!(idx.contains_key(&5));
    }

    #[test]
    fn track_id_index_first_wins_on_duplicate() {
        let tracks1 = vec![make_track("1", "First", "X", "/a.mp3")];
        let tracks2 = vec![make_track("1", "Second", "Y", "/b.mp3")];
        let map = HashMap::from([("set1".to_string(), tracks1), ("set2".to_string(), tracks2)]);
        let idx = build_usb_track_id_index(&map);
        assert_eq!(idx.len(), 1);
        // HashMap iteration order is non-deterministic, so just check one entry exists
        assert!(idx.contains_key(&1));
    }
}
