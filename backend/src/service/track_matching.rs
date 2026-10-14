//! Shared evidence for matching track files and metadata candidates.
//!
//! These are different kinds of evidence. A metadata key finds candidates;
//! identical bytes establish an identical file. Database record identity and
//! playback's best-effort ranking remain with their respective callers.

use std::hash::{Hash, Hasher};
use std::io::{BufReader, Read};
use std::path::Path;

use super::usb_utils::canonicalize_playlist_name;
use crate::audio_format::same_format_extension;

/// (file size, normalized title, normalized artist). This is a candidate
/// lookup key, not proof that two files contain the same recording.
pub(crate) type ContentFingerprint = (u32, String, String);

pub(crate) fn content_fingerprint_key(
    file_size_bytes: Option<i64>,
    title: &str,
    artist: &str,
) -> Option<ContentFingerprint> {
    let size = u32::try_from(file_size_bytes?).ok().filter(|&s| s > 0)?;
    let title_key = canonicalize_playlist_name(title);
    if title_key.is_empty() {
        return None;
    }
    Some((size, title_key, canonicalize_playlist_name(artist)))
}

/// Metadata-only key used to find Library candidates. It does not hash audio.
pub fn build_track_match_fingerprint(title: &str, artist: &str, album: Option<&str>) -> String {
    let normalized = format!(
        "{}|{}|{}",
        normalize_hash_part(title),
        normalize_hash_part(artist),
        normalize_hash_part(album.unwrap_or_default()),
    );
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(crate) fn normalize_hash_part(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_whitespace() {
                ' '
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Match a filename regardless of slash style or letter case. Keep the
/// extension: same-name repair fallback must not pair different formats.
pub(crate) fn matching_file_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

/// Playback also gives a weaker score to matching stems. Format checks are
/// applied separately before that score can select a candidate.
pub(crate) fn matching_file_stem(path: &str) -> String {
    let file = matching_file_name(path);
    file.rsplit_once('.')
        .map_or_else(|| file.clone(), |(stem, _)| stem.to_string())
}

pub(crate) const METADATA_DURATION_TOLERANCE_MS: i64 = 2000;

/// Prefer the recorded size. Older or incomplete rows may omit it, so read
/// only that file's metadata when needed; no audio bytes are read.
pub(crate) fn file_size_for_match(path: &str, recorded: Option<i64>) -> Option<i64> {
    recorded.filter(|&size| size > 0).or_else(|| {
        std::fs::metadata(path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .and_then(|metadata| i64::try_from(metadata.len()).ok())
            .filter(|&size| size > 0)
    })
}

/// Gates a metadata candidate before linking a USB file to a Library row.
/// A missing duration cannot disprove a candidate. Both file sizes must be
/// known and equal, and the format extensions must be known and equal.
pub(crate) fn library_metadata_candidate_compatible(
    incoming_path: &str,
    candidate_path: &str,
    incoming_duration_ms: Option<i64>,
    candidate_duration_ms: Option<i64>,
    incoming_size: Option<i64>,
    candidate_size: Option<i64>,
) -> bool {
    if !same_format_extension(incoming_path, candidate_path) {
        return false;
    }
    let duration_ok = match (incoming_duration_ms, candidate_duration_ms) {
        (Some(a), Some(b)) => (a - b).abs() <= METADATA_DURATION_TOLERANCE_MS,
        _ => true,
    };
    if !duration_ok {
        return false;
    }
    matches!(
        (
            file_size_for_match(incoming_path, incoming_size),
            file_size_for_match(candidate_path, candidate_size)
        ),
        (Some(a), Some(b)) if a == b
    )
}

/// Startup cleanup has no per-track review, so it requires duration and size
/// evidence to be known and agree before folding a USB placeholder into a local row.
pub(crate) fn placeholder_merge_candidate(
    local_path: &str,
    usb_path: &str,
    local_duration_ms: Option<i64>,
    usb_duration_ms: Option<i64>,
    local_size: Option<i64>,
    usb_size: Option<i64>,
) -> bool {
    same_format_extension(local_path, usb_path)
        && matches!(
            (local_duration_ms, usb_duration_ms),
            (Some(a), Some(b)) if (a - b).abs() <= METADATA_DURATION_TOLERANCE_MS
        )
        && matches!(
            (
                file_size_for_match(local_path, local_size),
                file_size_for_match(usb_path, usb_size)
            ),
            (Some(a), Some(b)) if a == b
        )
}

/// Compare complete files in bounded memory. Different names, including a
/// `-1` suffix, are irrelevant; an edited tag or different encoding changes
/// the bytes and therefore does not count as an identical file.
pub(crate) fn same_file_bytes(a: &Path, b: &Path) -> bool {
    let (Ok(a_file), Ok(b_file)) = (std::fs::File::open(a), std::fs::File::open(b)) else {
        return false;
    };
    let (Ok(a_size), Ok(b_size)) = (a_file.metadata(), b_file.metadata()) else {
        return false;
    };
    if a_size.len() != b_size.len() {
        return false;
    }

    let mut a_reader = BufReader::new(a_file);
    let mut b_reader = BufReader::new(b_file);
    let mut a_buf = [0u8; 64 * 1024];
    let mut b_buf = [0u8; 64 * 1024];
    let mut remaining = a_size.len();
    while remaining > 0 {
        let len = remaining.min(a_buf.len() as u64) as usize;
        if a_reader.read_exact(&mut a_buf[..len]).is_err()
            || b_reader.read_exact(&mut b_buf[..len]).is_err()
        {
            return false;
        }
        if a_buf[..len] != b_buf[..len] {
            return false;
        }
        remaining -= len as u64;
    }
    let mut a_tail = [0u8; 1];
    let mut b_tail = [0u8; 1];
    matches!(
        (a_reader.read(&mut a_tail), b_reader.read(&mut b_tail)),
        (Ok(0), Ok(0))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_files_match_despite_different_numeric_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("Track.mp3");
        let suffixed = dir.path().join("Track-1.mp3");
        let content = vec![42u8; 128 * 1024 + 1];
        std::fs::write(&original, &content).unwrap();
        std::fs::write(&suffixed, &content).unwrap();
        assert!(same_file_bytes(&original, &suffixed));
        let mut different = content;
        *different.last_mut().unwrap() = 43;
        std::fs::write(&suffixed, &different).unwrap();
        assert!(!same_file_bytes(&original, &suffixed));
    }

    #[test]
    fn metadata_keys_do_not_establish_file_identity() {
        assert_eq!(
            content_fingerprint_key(Some(12), "Song", "Artist"),
            content_fingerprint_key(Some(12), "song", "ARTIST")
        );
        assert_eq!(
            build_track_match_fingerprint("Song", "Artist", Some("Album")),
            build_track_match_fingerprint("song", "artist", Some("album"))
        );
    }

    #[test]
    fn filename_matching_handles_both_slash_styles_and_unicode_case() {
        assert_eq!(matching_file_name(r"C:\Music\Été.MP3"), "été.mp3");
        assert_eq!(matching_file_name("/usb/ÉTÉ.mp3"), "été.mp3");
        assert_eq!(matching_file_stem(r"C:\Music\Été.MP3"), "été");
        assert_eq!(matching_file_stem("/usb/no-extension"), "no-extension");
    }

    #[test]
    fn metadata_candidate_requires_same_format_and_available_file_facts() {
        assert!(library_metadata_candidate_compatible(
            "Track.mp3",
            "Track-1.mp3",
            Some(200_000),
            Some(201_000),
            Some(1000),
            Some(1000),
        ));
        assert!(!library_metadata_candidate_compatible(
            "Track.mp3",
            "Track.flac",
            Some(200_000),
            Some(200_000),
            Some(1000),
            Some(1000),
        ));
        assert!(!library_metadata_candidate_compatible(
            "missing-a.mp3",
            "missing-b.mp3",
            Some(200_000),
            Some(200_000),
            None,
            Some(1000),
        ));
    }

    #[test]
    fn missing_recorded_sizes_use_file_metadata_without_reading_audio() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("local.mp3");
        let usb = dir.path().join("usb.mp3");
        std::fs::write(&local, [1u8; 4]).unwrap();
        std::fs::write(&usb, [2u8; 4]).unwrap();
        let local = local.to_str().unwrap();
        let usb = usb.to_str().unwrap();

        assert!(library_metadata_candidate_compatible(
            usb, local, None, None, None, None,
        ));
        assert!(placeholder_merge_candidate(
            local,
            usb,
            Some(200_000),
            Some(200_000),
            None,
            None,
        ));
        std::fs::write(usb, [2u8; 5]).unwrap();
        assert!(!library_metadata_candidate_compatible(
            usb, local, None, None, None, None,
        ));
    }

    #[test]
    fn placeholder_merge_requires_known_file_facts_and_format() {
        assert!(placeholder_merge_candidate(
            "Track.mp3",
            "Track-1.mp3",
            Some(200_000),
            Some(201_000),
            Some(1000),
            Some(1000),
        ));
        assert!(!placeholder_merge_candidate(
            "Track.mp3",
            "Track.flac",
            Some(200_000),
            Some(200_000),
            Some(1000),
            Some(1000),
        ));
        assert!(!placeholder_merge_candidate(
            "Track.mp3",
            "Track-1.mp3",
            Some(200_000),
            None,
            Some(1000),
            Some(1000),
        ));
    }
}
