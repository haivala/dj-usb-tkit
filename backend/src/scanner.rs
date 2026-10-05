use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::Accessor;
use lofty::probe::{Probe, read_from_path};
use lofty::tag::{ItemKey, Tag};
use walkdir::WalkDir;

use crate::error::{BackendError, BackendResult};
use crate::wav_format::{WavFormatIssue, detect_wav_format_issue};

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "aif", "aiff", "wav", "aac", "m4a", "ogg", "opus", "mp4", "m4p",
];

const FORMAT_ONLY_FOLDERS: &[&str] = &[
    "flac", "mp3", "wav", "aiff", "aif", "m4a", "aac", "ogg", "opus",
];

#[derive(Default)]
struct EmbeddedMetadata {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    track_number: Option<u32>,
    tonality: Option<String>,
    disc_number: Option<u32>,
    subtitle: Option<String>,
    comment: Option<String>,
    isrc: Option<String>,
    release_year: Option<u32>,
    release_date: Option<String>,
    recorded_date: Option<String>,
    genre: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScannedTrack {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub tonality: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub file_modified_at: Option<String>,
    pub format_ext: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub wav_extensible_kind: Option<WavFormatIssue>,
    pub disc_number: Option<u32>,
    pub subtitle: Option<String>,
    pub comment: Option<String>,
    pub isrc: Option<String>,
    pub release_year: Option<u32>,
    pub release_date: Option<String>,
    pub recorded_date: Option<String>,
    pub genre: Option<String>,
}

pub fn scan_audio_files(source_roots: &[String]) -> BackendResult<Vec<ScannedTrack>> {
    let mut tracks = Vec::new();

    for root in source_roots {
        let root_path = PathBuf::from(root);
        if !root_path.exists() {
            continue;
        }

        for entry in WalkDir::new(&root_path).follow_links(true) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            if !is_library_audio_file(path) {
                continue;
            }

            tracks.push(scan_audio_file(path)?);
        }
    }

    Ok(tracks)
}

/// Read tags and file facts for one audio file -- the per-file step of
/// `scan_audio_files`, also used to index audio already sitting on a USB.
pub fn scan_audio_file(path: &Path) -> BackendResult<ScannedTrack> {
    let metadata = read_file_metadata(path)?;
    let meta = read_embedded_metadata(path);
    let (sample_rate_hz, bit_depth, bitrate_kbps) = read_audio_technical_metadata(path);
    let wav_extensible_kind = read_wav_extensible_kind(path);
    let (fallback_artist, fallback_title) = infer_artist_title(path);
    let fallback_album = infer_album_from_path(path);
    let fallback_track_number = infer_track_number_from_name(
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default(),
    );
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs().to_string());

    Ok(ScannedTrack {
        path: path.to_string_lossy().to_string(),
        title: meta.title.unwrap_or(fallback_title),
        artist: meta.artist.unwrap_or(fallback_artist),
        album: meta.album.or(fallback_album),
        track_number: meta.track_number.or(fallback_track_number),
        tonality: meta.tonality,
        file_size_bytes: i64::try_from(metadata.len()).ok(),
        file_modified_at: modified,
        format_ext: crate::utils::format_ext_from_path(&path.to_string_lossy()),
        sample_rate_hz,
        bit_depth,
        bitrate_kbps,
        wav_extensible_kind,
        disc_number: meta.disc_number,
        subtitle: meta.subtitle,
        comment: meta.comment,
        isrc: meta.isrc,
        release_year: meta.release_year,
        release_date: meta.release_date,
        recorded_date: meta.recorded_date,
        genre: meta.genre,
    })
}

fn read_audio_technical_metadata(path: &Path) -> (Option<u32>, Option<u8>, Option<u32>) {
    let tagged = match Probe::open(path).and_then(|p| p.read()) {
        Ok(file) => file,
        Err(_) => return (None, None, None),
    };
    let props = tagged.properties();
    let sample_rate_hz = props.sample_rate();
    let bit_depth = props.bit_depth();
    let bitrate_kbps = props.audio_bitrate();
    (sample_rate_hz, bit_depth, bitrate_kbps)
}

/// Detects a WAVE_FORMAT_EXTENSIBLE issue for `.wav`/`.wave` files only -
/// `lofty`'s properties don't expose the raw format tag, so this parses the
/// RIFF `fmt ` chunk directly. Other formats don't have this issue.
fn read_wav_extensible_kind(path: &Path) -> Option<WavFormatIssue> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())?;
    if ext != "wav" && ext != "wave" {
        return None;
    }
    detect_wav_format_issue(path)
}

pub fn unique_paths(tracks: &[ScannedTrack]) -> HashSet<&str> {
    tracks.iter().map(|t| t.path.as_str()).collect()
}

fn infer_artist_title(path: &Path) -> (String, String) {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown Title");

    if let Some((artist, title)) = stem.split_once(" - ") {
        let artist = artist.trim();
        let title = title.trim();

        if !artist.is_empty() && !title.is_empty() {
            return (artist.to_string(), title.to_string());
        }
    }

    ("Unknown Artist".to_string(), stem.trim().to_string())
}

fn is_audio_path(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|x| x.to_str())
        .map(str::trim)
        .unwrap_or_default();
    if file_name.starts_with("._") {
        return false;
    }

    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| x.to_ascii_lowercase())
        .unwrap_or_default();

    is_supported_audio_extension(&ext)
}

/// A file the library takes: a supported audio extension and not an MP4 video.
/// Shared by the folder scan and the external-library imports.
pub(crate) fn is_library_audio_file(path: &Path) -> bool {
    is_audio_path(path) && !is_video_file(path)
}

/// A `.mp4` that carries a video track (a downloaded clip, not an AAC audio
/// file) -- the library leaves those out.
fn is_video_file(path: &Path) -> bool {
    let is_mp4 = path
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("mp4"));
    is_mp4 && File::open(path).is_ok_and(|mut file| mp4_has_video_track(&mut file))
}

/// Walks the MP4 box tree (moov > trak > mdia > hdlr) looking for a track
/// whose handler type is `vide`. Only box headers and the few handler bytes
/// are read, so a large file costs a handful of small reads.
fn mp4_has_video_track<R: Read + Seek>(reader: &mut R) -> bool {
    let Ok(file_end) = reader.seek(SeekFrom::End(0)) else {
        return false;
    };
    let Some(moov) = find_mp4_box(reader, 0, file_end, b"moov") else {
        return false;
    };
    let mut pos = moov.0;
    while let Some((trak_start, trak_end)) = find_mp4_box(reader, pos, moov.1, b"trak") {
        pos = trak_end;
        let handler = find_mp4_box(reader, trak_start, trak_end, b"mdia")
            .and_then(|(start, end)| find_mp4_box(reader, start, end, b"hdlr"))
            .and_then(|(start, _)| {
                // hdlr payload: version/flags (4), pre_defined (4), handler_type (4).
                let mut handler = [0u8; 4];
                reader.seek(SeekFrom::Start(start + 8)).ok()?;
                reader.read_exact(&mut handler).ok()?;
                Some(handler)
            });
        if handler == Some(*b"vide") {
            return true;
        }
    }
    false
}

/// The payload range (start, end) of the first `kind` box among the sibling
/// boxes in `start..end`.
fn find_mp4_box<R: Read + Seek>(
    reader: &mut R,
    mut pos: u64,
    end: u64,
    kind: &[u8; 4],
) -> Option<(u64, u64)> {
    const MAX_SIBLINGS: usize = 4096;
    for _ in 0..MAX_SIBLINGS {
        if pos.checked_add(8)? > end {
            return None;
        }
        reader.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        reader.read_exact(&mut header).ok()?;
        let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let (payload_start, box_end) = match size32 {
            // Box runs to the end of its parent.
            0 => (pos + 8, end),
            // 64-bit size follows the type.
            1 => {
                let mut large = [0u8; 8];
                reader.read_exact(&mut large).ok()?;
                (pos + 16, pos.checked_add(u64::from_be_bytes(large))?)
            }
            size => (pos + 8, pos.checked_add(u64::from(size))?),
        };
        if box_end <= pos || box_end > end || payload_start > box_end {
            return None;
        }
        if &header[4..8] == kind {
            return Some((payload_start, box_end));
        }
        pos = box_end;
    }
    None
}

fn read_embedded_metadata(path: &Path) -> EmbeddedMetadata {
    let tagged = match read_from_path(path) {
        Ok(file) => file,
        Err(_) => return EmbeddedMetadata::default(),
    };

    let tags = tagged.tags();
    if tags.is_empty() {
        return EmbeddedMetadata::default();
    }

    let mut out = EmbeddedMetadata::default();

    for tag in tags {
        if out.title.is_none() {
            out.title = clean_tag_text(tag.title().as_deref());
        }
        if out.artist.is_none() {
            out.artist = clean_tag_text(tag.artist().as_deref());
        }
        if out.album.is_none() {
            out.album = clean_tag_text(tag.album().as_deref());
        }
        if out.track_number.is_none() {
            out.track_number = tag.track().filter(|v| *v > 0);
        }
        if out.tonality.is_none() {
            out.tonality = tag_item_text(tag, &ItemKey::InitialKey);
        }
        if out.disc_number.is_none() {
            out.disc_number = tag.disk().filter(|v| *v > 0);
        }
        if out.subtitle.is_none() {
            out.subtitle = tag_item_text(tag, &ItemKey::TrackSubtitle);
        }
        if out.comment.is_none() {
            out.comment = tag_item_text(tag, &ItemKey::Comment);
        }
        if out.isrc.is_none() {
            out.isrc = tag_item_text(tag, &ItemKey::Isrc);
        }
        if out.release_year.is_none() {
            out.release_year = tag.year().filter(|v| *v > 0);
        }
        if out.release_date.is_none() {
            out.release_date = tag_item_text(tag, &ItemKey::ReleaseDate)
                .or_else(|| tag_item_text(tag, &ItemKey::OriginalReleaseDate));
        }
        if out.recorded_date.is_none() {
            out.recorded_date = tag_item_text(tag, &ItemKey::RecordingDate);
        }
        if out.genre.is_none() {
            out.genre = clean_tag_text(tag.genre().as_deref());
        }
    }

    out
}

fn infer_album_from_path(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let parent_name = parent
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;

    if let Some(cleaned) = strip_format_suffix_from_album_name(parent_name) {
        return Some(cleaned);
    }

    if is_format_only_folder(parent_name) {
        let grandparent_name = parent
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        return Some(grandparent_name.to_string());
    }

    Some(parent_name.to_string())
}

fn strip_format_suffix_from_album_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    let lowered = trimmed.to_ascii_lowercase();
    let suffixes = [
        " flacs", " mp3s", " flac", " mp3", " wav", " aiff", " aif", " m4a", " aac",
    ];

    for suffix in suffixes {
        if lowered.ends_with(suffix) {
            let cut = trimmed.len().saturating_sub(suffix.len());
            let candidate = trimmed[..cut].trim_end_matches([' ', '-', '_']).trim();
            if !candidate.is_empty() {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

fn is_format_only_folder(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    FORMAT_ONLY_FOLDERS.contains(&lowered.as_str())
}

fn is_supported_audio_extension(ext: &str) -> bool {
    AUDIO_EXTENSIONS.contains(&ext)
}

fn clean_tag_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn tag_item_text(tag: &Tag, key: &ItemKey) -> Option<String> {
    clean_tag_text(tag.get_string(key))
}

fn read_file_metadata(path: &Path) -> BackendResult<std::fs::Metadata> {
    std::fs::metadata(path).map_err(|err| {
        BackendError::Io(std::io::Error::new(
            err.kind(),
            format!("failed to read file metadata for {}: {err}", path.display()),
        ))
    })
}

fn infer_track_number_from_name(name: &str) -> Option<u32> {
    let mut digits = String::new();
    for ch in name.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            if digits.len() >= 3 {
                break;
            }
            continue;
        }
        if !digits.is_empty() {
            break;
        }
        if ch.is_ascii_whitespace() || ch == '-' || ch == '_' || ch == '.' {
            continue;
        }
        return None;
    }

    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok().filter(|v| *v > 0)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    fn mp4_trak(handler: &[u8; 4]) -> Vec<u8> {
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(handler);
        hdlr.extend_from_slice(&[0u8; 12]);
        mp4_box(b"trak", &mp4_box(b"mdia", &mp4_box(b"hdlr", &hdlr)))
    }

    fn mp4_file(handlers: &[&[u8; 4]]) -> Vec<u8> {
        let traks = handlers
            .iter()
            .flat_map(|h| mp4_trak(h))
            .collect::<Vec<_>>();
        let mut out = mp4_box(b"ftyp", b"isom\0\0\0\0");
        out.extend(mp4_box(b"mdat", &[0u8; 64]));
        out.extend(mp4_box(b"moov", &traks));
        out
    }

    #[test]
    fn mp4_with_a_video_track_is_detected() {
        assert!(super::mp4_has_video_track(&mut Cursor::new(mp4_file(&[
            b"vide", b"soun"
        ]))));
        assert!(super::mp4_has_video_track(&mut Cursor::new(mp4_file(&[
            b"soun", b"vide"
        ]))));
    }

    #[test]
    fn audio_only_or_malformed_mp4_is_not_video() {
        assert!(!super::mp4_has_video_track(&mut Cursor::new(mp4_file(&[
            b"soun"
        ]))));
        assert!(!super::mp4_has_video_track(&mut Cursor::new(
            b"not an mp4 at all".to_vec()
        )));
        // A box claiming to run past the end of the file.
        let mut truncated = mp4_file(&[b"vide"]);
        truncated.truncate(truncated.len() - 10);
        assert!(!super::mp4_has_video_track(&mut Cursor::new(truncated)));
    }

    #[test]
    fn scan_leaves_out_mp4_videos_but_keeps_audio_mp4() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("clip.mp4"), mp4_file(&[b"vide", b"soun"])).unwrap();
        std::fs::write(dir.path().join("song.mp4"), mp4_file(&[b"soun"])).unwrap();
        let tracks =
            super::scan_audio_files(&[dir.path().to_string_lossy().to_string()]).expect("scan");
        let names = tracks
            .iter()
            .map(|t| {
                std::path::Path::new(&t.path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["song.mp4".to_string()]);
    }

    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // --- infer_artist_title ---

    #[test]
    fn infer_artist_title_dash_separated() {
        let (artist, title) = infer_artist_title(Path::new("/music/DJ Shadow - Midnight.mp3"));
        assert_eq!(artist, "DJ Shadow");
        assert_eq!(title, "Midnight");
    }

    #[test]
    fn infer_artist_title_multiple_dashes_uses_first() {
        // split_once splits on the first " - " only
        let (artist, title) = infer_artist_title(Path::new("/music/Artist - Title - Remix.mp3"));
        assert_eq!(artist, "Artist");
        assert_eq!(title, "Title - Remix");
    }

    #[test]
    fn infer_artist_title_no_dash_fallback() {
        let (artist, title) = infer_artist_title(Path::new("/music/just_a_track.mp3"));
        assert_eq!(artist, "Unknown Artist");
        assert_eq!(title, "just_a_track");
    }

    #[test]
    fn infer_artist_title_dash_but_empty_artist() {
        // " - Title" → split_once gives ("", "Title"), empty artist → fallback
        let (artist, title) = infer_artist_title(Path::new("/music/ - Title.mp3"));
        assert_eq!(artist, "Unknown Artist");
        // Fallback uses the full stem trimmed
        assert_eq!(title, "- Title");
    }

    #[test]
    fn infer_artist_title_whitespace_around_dash() {
        let (artist, title) =
            infer_artist_title(Path::new("/music/  Demo Artist  -  Demo Title  .flac"));
        assert_eq!(artist, "Demo Artist");
        assert_eq!(title, "Demo Title");
    }

    // --- infer_track_number_from_name ---

    #[test]
    fn track_number_leading_digits() {
        assert_eq!(infer_track_number_from_name("01 Artist - Title"), Some(1));
        assert_eq!(infer_track_number_from_name("12 Track"), Some(12));
    }

    #[test]
    fn track_number_leading_zeros() {
        assert_eq!(infer_track_number_from_name("001 Track"), Some(1));
    }

    #[test]
    fn track_number_three_digit_cap() {
        // Stops collecting at 3 digits
        assert_eq!(infer_track_number_from_name("123456 Track"), Some(123));
    }

    #[test]
    fn track_number_no_digits() {
        assert_eq!(infer_track_number_from_name("Track Title"), None);
    }

    #[test]
    fn track_number_zero_returns_none() {
        // filter(|v| *v > 0) should reject 0
        assert_eq!(infer_track_number_from_name("00 Track"), None);
    }

    #[test]
    fn track_number_empty_string() {
        assert_eq!(infer_track_number_from_name(""), None);
    }

    #[test]
    fn track_number_digits_after_letters_rejected() {
        // Non-separator non-digit before any digits → None
        assert_eq!(infer_track_number_from_name("Track 01"), None);
    }

    #[test]
    fn track_number_with_separators_before_digits() {
        assert_eq!(infer_track_number_from_name("- 05 Track"), Some(5));
        assert_eq!(infer_track_number_from_name("_03_Track"), Some(3));
        assert_eq!(infer_track_number_from_name(".7 Track"), Some(7));
    }

    // --- infer_album_from_path ---

    #[test]
    fn album_from_parent_directory() {
        assert_eq!(
            infer_album_from_path(Path::new("/music/My Album/track.mp3")),
            Some("My Album".to_string())
        );
    }

    #[test]
    fn album_from_root_path() {
        // Root has no file_name component
        assert_eq!(infer_album_from_path(Path::new("/track.mp3")), None);
    }

    #[test]
    fn album_bare_filename() {
        // No parent directory context
        assert_eq!(infer_album_from_path(Path::new("track.mp3")), None);
    }

    #[test]
    fn album_strips_format_suffixes() {
        assert_eq!(
            infer_album_from_path(Path::new("/music/Compilation Disc B FLACs/track.flac")),
            Some("Compilation Disc B".to_string())
        );
        assert_eq!(
            infer_album_from_path(Path::new("/music/Compilation Disc B Mp3s/track.mp3")),
            Some("Compilation Disc B".to_string())
        );
    }

    #[test]
    fn album_uses_grandparent_for_format_only_folder() {
        assert_eq!(
            infer_album_from_path(Path::new("/music/Example Anthology/FLAC/track.flac")),
            Some("Example Anthology".to_string())
        );
    }

    // --- is_audio_path ---

    #[test]
    fn is_audio_all_supported_extensions() {
        for ext in [
            "mp3", "flac", "aif", "aiff", "wav", "aac", "m4a", "ogg", "opus", "mp4", "m4p",
        ] {
            let path = PathBuf::from(format!("/music/track.{ext}"));
            assert!(is_audio_path(&path), "expected {ext} to be audio");
        }
    }

    #[test]
    fn is_audio_case_insensitive() {
        assert!(is_audio_path(Path::new("/music/track.MP3")));
        assert!(is_audio_path(Path::new("/music/track.FlAc")));
        assert!(is_audio_path(Path::new("/music/track.WAV")));
    }

    #[test]
    fn is_audio_rejects_non_audio() {
        assert!(!is_audio_path(Path::new("/music/readme.txt")));
        assert!(!is_audio_path(Path::new("/music/cover.jpg")));
        assert!(!is_audio_path(Path::new("/music/data.pdf")));
        assert!(!is_audio_path(Path::new("/music/._track.flac")));
        assert!(!is_audio_path(Path::new("/music/._track.mp3")));
    }

    #[test]
    fn is_audio_no_extension() {
        assert!(!is_audio_path(Path::new("/music/track")));
    }

    // --- scan_audio_files ---

    #[test]
    fn scan_skips_missing_roots() {
        let result = scan_audio_files(&["/nonexistent/path/12345".to_string()]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn scan_skips_non_audio_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("readme.txt"), "hello").unwrap();
        fs::write(dir.path().join("cover.jpg"), [0xFF, 0xD8]).unwrap();

        let result = scan_audio_files(&[dir.path().to_string_lossy().to_string()]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn read_file_metadata_reports_path_context() {
        let missing = Path::new("/definitely/not/here/track.mp3");
        let err = read_file_metadata(missing).expect_err("missing file should fail");
        let message = err.to_string();
        assert!(
            message.contains("failed to read file metadata for /definitely/not/here/track.mp3"),
            "error should include file path context, got: {message}"
        );
    }

    fn make_minimal_wav() -> Vec<u8> {
        let sample_rate: u32 = 44_100;
        let num_samples: u32 = 4_410; // ~100ms
        let data_len = num_samples * 2; // 16-bit mono
        let riff_len = 36 + data_len;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&riff_len.to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.resize(44 + data_len as usize, 0); // silence
        out
    }

    fn make_minimal_wav_extensible(sub_format_tag: u16) -> Vec<u8> {
        let sample_rate: u32 = 44_100;
        let num_samples: u32 = 4_410; // ~100ms
        let data_len = num_samples * 2; // 16-bit mono
        let fmt_len: u32 = 40;
        let riff_len = 12 + 8 + fmt_len + 8 + data_len - 8;
        let mut out = Vec::with_capacity(12 + 8 + fmt_len as usize + 8 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&riff_len.to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&fmt_len.to_le_bytes());
        out.extend_from_slice(&0xFFFEu16.to_le_bytes()); // WAVE_FORMAT_EXTENSIBLE
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        out.extend_from_slice(&22u16.to_le_bytes()); // cbSize
        out.extend_from_slice(&16u16.to_le_bytes()); // validBitsPerSample
        out.extend_from_slice(&0x4u32.to_le_bytes()); // channel mask (front center)
        // SubFormat GUID: sub_format_tag as Data1's low word, then the
        // standard KSDATAFORMAT tail (...-0000-0010-8000-00AA00389B71).
        out.extend_from_slice(&sub_format_tag.to_le_bytes());
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x10, 0x00]);
        out.extend_from_slice(&[0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]);
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.resize(out.len() + data_len as usize, 0); // silence
        out
    }

    #[test]
    fn scan_detects_wav_extensible_kind() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("plain.wav"), make_minimal_wav()).unwrap();
        fs::write(
            dir.path().join("extensible_pcm.wav"),
            make_minimal_wav_extensible(1),
        )
        .unwrap();
        fs::write(
            dir.path().join("extensible_other.wav"),
            make_minimal_wav_extensible(0x0006),
        )
        .unwrap();

        let result = scan_audio_files(&[dir.path().to_string_lossy().to_string()]).unwrap();
        assert_eq!(result.len(), 3);

        let by_name = |name: &str| {
            result
                .iter()
                .find(|t| t.path.ends_with(name))
                .unwrap_or_else(|| panic!("missing scanned track {name}"))
        };

        assert_eq!(by_name("plain.wav").wav_extensible_kind, None);
        assert_eq!(
            by_name("extensible_pcm.wav").wav_extensible_kind,
            Some(WavFormatIssue::ExtensiblePcm)
        );
        assert_eq!(
            by_name("extensible_other.wav").wav_extensible_kind,
            Some(WavFormatIssue::ExtensibleOther)
        );
    }

    #[test]
    fn scan_finds_audio_and_infers_metadata() {
        let dir = tempdir().unwrap();
        let album_dir = dir.path().join("My Album");
        fs::create_dir_all(&album_dir).unwrap();
        let track_path = album_dir.join("03 Artist - Title.wav");
        fs::write(&track_path, make_minimal_wav()).unwrap();

        let result = scan_audio_files(&[dir.path().to_string_lossy().to_string()]).unwrap();
        assert_eq!(result.len(), 1);
        let track = &result[0];
        // WAV has no embedded metadata, so fallbacks are used
        // infer_artist_title splits on " - ": "03 Artist - Title" → ("03 Artist", "Title")
        assert_eq!(track.artist, "03 Artist");
        assert_eq!(track.title, "Title");
        assert_eq!(track.album, Some("My Album".to_string()));
        assert_eq!(track.track_number, Some(3));
        assert!(track.file_size_bytes.unwrap() > 0);
        assert!(track.file_modified_at.is_some());
        assert_eq!(track.format_ext.as_deref(), Some("wav"));
    }

    // --- unique_paths ---

    #[test]
    fn unique_paths_deduplicates() {
        let tracks = vec![
            ScannedTrack {
                path: "/a.mp3".to_string(),
                title: "A".to_string(),
                artist: "X".to_string(),
                album: None,
                track_number: None,
                tonality: None,
                file_size_bytes: None,
                file_modified_at: None,
                format_ext: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                wav_extensible_kind: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                genre: None,
            },
            ScannedTrack {
                path: "/a.mp3".to_string(),
                title: "A".to_string(),
                artist: "X".to_string(),
                album: None,
                track_number: None,
                tonality: None,
                file_size_bytes: None,
                file_modified_at: None,
                format_ext: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                wav_extensible_kind: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                genre: None,
            },
            ScannedTrack {
                path: "/b.mp3".to_string(),
                title: "B".to_string(),
                artist: "Y".to_string(),
                album: None,
                track_number: None,
                tonality: None,
                file_size_bytes: None,
                file_modified_at: None,
                format_ext: None,
                sample_rate_hz: None,
                bit_depth: None,
                bitrate_kbps: None,
                wav_extensible_kind: None,
                disc_number: None,
                subtitle: None,
                comment: None,
                isrc: None,
                release_year: None,
                release_date: None,
                recorded_date: None,
                genre: None,
            },
        ];
        let paths = unique_paths(&tracks);
        assert_eq!(paths.len(), 2);
        assert!(paths.contains("/a.mp3"));
        assert!(paths.contains("/b.mp3"));
    }

    #[test]
    fn unique_paths_empty() {
        assert!(unique_paths(&[]).is_empty());
    }
}
