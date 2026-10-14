//! Audio file formats: which files the app picks up, and the per-format facts
//! the scan, export, analysis and repair code branch on.
//!
//! Format is decided by file extension. Spelling variants of one format
//! (`.wav`/`.wave`, `.aif`/`.aiff`, `.flac`/`.fla`) hold the same audio and
//! classify the same. Players, though, recognise a file by its extension, so
//! which spellings they read is a separate list: `.wave` is not on it and is
//! exported as `.wav`.

use std::borrow::Cow;
use std::path::Path;

/// Extensions the library folder scan and the external-library imports take.
/// An `.mp4` carrying a video track is still excluded (see
/// `scanner::is_library_audio_file`). Formats players can't read are taken
/// too; their format check warns.
const LIBRARY_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "aif", "aiff", "wav", "wave", "aac", "m4a", "ogg", "opus", "mp4", "m4p",
];

/// Extensions Pioneer players read from a USB stick (CDJ-3000 format table).
const PLAYER_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "aac", "mp4", "wav", "aif", "aiff", "flac", "fla",
];

/// Folder names that only say which format the files in them are (`FLAC/`,
/// `MP3/`), so album inference looks one level further up.
const FORMAT_FOLDER_NAMES: &[&str] = &[
    "flac", "mp3", "wav", "aiff", "aif", "m4a", "aac", "ogg", "opus",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Mp3,
    Flac,
    Wav,
    Aiff,
    Aac,
    M4a,
    Mp4,
    M4p,
    Ogg,
    Opus,
    /// Not a file extension, but a format name some sources store in place of
    /// one.
    Alac,
}

impl AudioFormat {
    /// Case-insensitive, surrounding whitespace ignored. `None` for anything
    /// that isn't a known audio format.
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext.trim().to_ascii_lowercase().as_str() {
            "mp3" => Self::Mp3,
            "flac" | "fla" => Self::Flac,
            "wav" | "wave" => Self::Wav,
            "aif" | "aiff" => Self::Aiff,
            "aac" => Self::Aac,
            "m4a" => Self::M4a,
            "mp4" => Self::Mp4,
            "m4p" => Self::M4p,
            "ogg" => Self::Ogg,
            "opus" => Self::Opus,
            "alac" => Self::Alac,
            _ => return None,
        })
    }

    pub fn of_path(path: &Path) -> Option<Self> {
        Self::from_extension(path.extension()?.to_str()?)
    }

    /// [`Self::of_path`] for a stored path string (local or USB-relative).
    pub fn of_path_str(path: &str) -> Option<Self> {
        Self::of_path(Path::new(path.trim()))
    }

    /// One spelling per format, for comparing two files' formats.
    pub fn canonical_extension(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Aiff => "aif",
            Self::Aac => "aac",
            Self::M4a => "m4a",
            Self::Mp4 => "mp4",
            Self::M4p => "m4p",
            Self::Ogg => "ogg",
            Self::Opus => "opus",
            Self::Alac => "alac",
        }
    }

    /// The PDB/edb track `file_type` code: MP3=1, MP4=3, M4A=4, FLAC=5,
    /// ALAC=6, WAV=11, AIFF=12; 0 for a format rekordbox has no code for.
    pub fn rekordbox_file_type(self) -> i64 {
        match self {
            Self::Mp3 => 1,
            Self::Mp4 => 3,
            Self::M4a | Self::Aac | Self::M4p => 4,
            Self::Flac => 5,
            Self::Alac => 6,
            Self::Wav => 11,
            Self::Aiff => 12,
            Self::Ogg | Self::Opus => 0,
        }
    }
}

/// [`AudioFormat::rekordbox_file_type`] for a stored extension; 0 when unknown.
pub fn rekordbox_file_type(ext: &str) -> i64 {
    AudioFormat::from_extension(ext).map_or(0, AudioFormat::rekordbox_file_type)
}

/// Whether two paths are the same audio format, spelling variants included.
/// Two unknown extensions match when they are equal. This rules out an
/// MP3/FLAC match; it does not prove equal encoding or file identity.
pub fn same_format_extension(a: &str, b: &str) -> bool {
    fn format(path: &str) -> Option<String> {
        let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
        Some(
            AudioFormat::from_extension(&extension)
                .map_or(extension, |format| format.canonical_extension().to_string()),
        )
    }
    matches!((format(a), format(b)), (Some(a), Some(b)) if a == b)
}

/// `ext` must already be lowercase.
pub fn is_library_extension(ext: &str) -> bool {
    LIBRARY_EXTENSIONS.contains(&ext)
}

/// Whether players read `ext` (any case) as audio.
pub fn is_player_extension(ext: &str) -> bool {
    PLAYER_EXTENSIONS.contains(&ext.trim().to_ascii_lowercase().as_str())
}

/// A file on a USB stick that players can play.
pub fn is_player_audio(path: &Path) -> bool {
    format_ext_of(path).is_some_and(|ext| is_player_extension(&ext))
}

/// The lowercase extension a file gets when exported to a USB stick: the
/// source's own, except `.wave`, which players don't read and becomes `.wav`.
/// `bin` when the source has none.
pub fn export_extension(source: &Path) -> String {
    match format_ext_of(source) {
        Some(ext) if ext == "wave" => "wav".to_string(),
        Some(ext) => ext,
        None => "bin".to_string(),
    }
}

/// `file_name` with the extension it gets on a USB stick (see
/// [`export_extension`]). Other names, letter case included, are unchanged.
pub fn export_file_name(file_name: &str) -> Cow<'_, str> {
    match file_name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("wave") => Cow::Owned(format!("{stem}.wav")),
        _ => Cow::Borrowed(file_name),
    }
}

pub fn is_format_folder_name(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    FORMAT_FOLDER_NAMES.contains(&lowered.as_str())
}

/// The lowercase file extension of `path`'s last segment, or `None` when it has
/// none. Shared by the scanner, `row_to_track`, and USB track building so a
/// track's format is always known on the wire even when the DB column or PDB
/// row omits it.
pub fn format_ext_from_path(path: &str) -> Option<String> {
    format_ext_of(Path::new(path.trim()))
}

/// [`format_ext_from_path`] for a path on this machine.
pub fn format_ext_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.trim().to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ext_from_path_lowercases_and_handles_missing() {
        assert_eq!(
            format_ext_from_path("/Contents/Artist/Song.MP3").as_deref(),
            Some("mp3")
        );
        assert_eq!(
            format_ext_from_path("relative/Track.FlAc").as_deref(),
            Some("flac")
        );
        assert_eq!(format_ext_from_path("/no/extension/here"), None);
        assert_eq!(format_ext_from_path(""), None);
        assert_eq!(format_ext_from_path("  /x/y.wav  ").as_deref(), Some("wav"));
    }

    #[test]
    fn spelling_variants_classify_the_same() {
        assert_eq!(AudioFormat::of_path_str("a.WAVE"), Some(AudioFormat::Wav));
        assert_eq!(AudioFormat::of_path_str("a.aiff"), Some(AudioFormat::Aiff));
        assert_eq!(AudioFormat::of_path_str("a.aif"), Some(AudioFormat::Aiff));
        assert_eq!(AudioFormat::of_path_str("a.txt"), None);
        assert_eq!(AudioFormat::of_path_str("noext"), None);
    }

    #[test]
    fn wave_is_exported_as_wav() {
        assert_eq!(export_extension(Path::new("/m/Track.WAVE")), "wav");
        assert_eq!(export_extension(Path::new("/m/Track.MP3")), "mp3");
        assert_eq!(export_extension(Path::new("/m/Track")), "bin");
        assert_eq!(export_file_name("Track.Wave"), "Track.wav");
        assert_eq!(export_file_name("Track.MP3"), "Track.MP3");
        assert_eq!(export_file_name("Track"), "Track");
    }

    #[test]
    fn rekordbox_file_type_maps_known_and_unknown_extensions() {
        assert_eq!(rekordbox_file_type("mp3"), 1);
        assert_eq!(rekordbox_file_type("MP4"), 3);
        assert_eq!(rekordbox_file_type("m4a"), 4);
        assert_eq!(rekordbox_file_type("aac"), 4);
        assert_eq!(rekordbox_file_type("flac"), 5);
        assert_eq!(rekordbox_file_type("alac"), 6);
        assert_eq!(rekordbox_file_type(" wav "), 11);
        assert_eq!(rekordbox_file_type("aiff"), 12);
        assert_eq!(rekordbox_file_type("ogg"), 0);
        assert_eq!(rekordbox_file_type("txt"), 0);
    }

    #[test]
    fn same_format_extension_folds_spellings_but_not_formats() {
        assert!(same_format_extension("/a/x.WAV", "/b/y.wave"));
        assert!(same_format_extension("x.aiff", "y.aif"));
        assert!(same_format_extension("x.xyz", "y.XYZ"));
        assert!(!same_format_extension("x.mp3", "y.flac"));
        assert!(!same_format_extension("x.m4a", "y.aac"));
        assert!(!same_format_extension("x", "y"));
    }

    #[test]
    fn library_and_player_sets() {
        assert!(is_library_extension("opus"));
        assert!(is_library_extension("wave"));
        assert!(is_player_audio(Path::new("/Contents/a/b.FLAC")));
        assert!(is_player_audio(Path::new("/Contents/a/b.fla")));
        assert!(is_player_audio(Path::new("/Contents/a/b.mp4")));
        assert!(!is_player_audio(Path::new("/Contents/a/b.wave")));
        assert!(!is_player_audio(Path::new("/Contents/a/b.ogg")));
        assert!(!is_player_audio(Path::new("/Contents/a/b.opus")));
        assert!(!is_player_audio(Path::new("/Contents/a/b.m4p")));
        assert!(is_format_folder_name(" FLAC "));
        assert!(!is_format_folder_name("mp4"));
    }
}
