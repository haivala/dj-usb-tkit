//! Where a USB track row's display files (waveform preview, artwork) are read
//! from.
//!
//! Reading them off the stick is what makes a USB playlist page slow on
//! Windows: Defender scans every file the app opens there, so each row costs
//! a scan of its `.EXT` analysis file and its artwork. Each file is taken from
//! the first of:
//!
//! 1. the library's own files, when the row matched a local track
//!    (`UsbTrack::local_track_id`) whose files are not on a USB stick;
//! 2. a local cache of earlier stick reads (`usb_read_cache.sqlite` in the
//!    app data dir), keyed by device, the file's path on the stick and its
//!    size + mtime -- a `stat` doesn't trigger a scan, opening the file does;
//! 3. the stick itself, storing the result in that cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::logging::{self, Level};
use crate::models::UsbTrack;

use super::browse_path_matches_root;
use super::usb_utils::{
    artwork_path_to_data_url, load_waveform_preview_from_analysis_path, waveform_preview_candidates,
};

const CACHE_FILE_NAME: &str = "usb_read_cache.sqlite";

/// A local track's own display files.
#[derive(Debug, Clone, Default)]
pub(crate) struct LibraryDisplayFiles {
    pub waveform_path: Option<String>,
    pub artwork_path: Option<String>,
}

/// How many display files of a page came from each source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DisplayReadCounts {
    pub library: usize,
    pub cache: usize,
    pub usb: usize,
}

impl DisplayReadCounts {
    pub(crate) fn add(&mut self, other: Self) {
        self.library += other.library;
        self.cache += other.cache;
        self.usb += other.usb;
    }
}

#[derive(Debug, Clone, Copy)]
enum FileKind {
    Waveform,
    Artwork,
}

impl FileKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Waveform => "waveform",
            Self::Artwork => "artwork",
        }
    }
}

struct ReadCache {
    conn: Connection,
    device_key: String,
}

pub(crate) struct UsbDisplaySource {
    usb_root: PathBuf,
    cache: Option<ReadCache>,
    /// Keyed by local track id.
    library: HashMap<String, LibraryDisplayFiles>,
}

impl UsbDisplaySource {
    /// Reads straight from the stick: no cache, no library files.
    #[cfg(test)]
    pub(crate) fn direct(usb_root: &Path) -> Self {
        Self {
            usb_root: usb_root.to_path_buf(),
            cache: None,
            library: HashMap::new(),
        }
    }

    /// `device_key` identifies the stick across mounts (its `usb_devices`
    /// id). Without a usable cache file this falls back to reading the stick
    /// directly.
    pub(crate) fn new(
        data_dir: &Path,
        usb_root: &Path,
        device_key: String,
        library: HashMap<String, LibraryDisplayFiles>,
    ) -> Self {
        let cache = match open_cache(&data_dir.join(CACHE_FILE_NAME)) {
            Ok(conn) => Some(ReadCache { conn, device_key }),
            Err(err) => {
                logging::emit(
                    Level::Warn,
                    "usb-import",
                    &format!("USB read cache unavailable, reading from the USB: {err}"),
                );
                None
            }
        };
        Self {
            usb_root: usb_root.to_path_buf(),
            cache,
            library,
        }
    }

    /// Fills a row's missing waveform preview / artwork data URL.
    pub(crate) fn fill(&self, track: &mut UsbTrack) -> DisplayReadCounts {
        let mut counts = DisplayReadCounts::default();
        let library = track
            .local_track_id
            .as_deref()
            .and_then(|id| self.library.get(id));

        if track.waveform_preview.is_none() {
            let from_library = library
                .and_then(|files| files.waveform_path.as_deref())
                .map(Path::new)
                .and_then(load_waveform_preview_from_analysis_path)
                .filter(|preview| !preview.is_empty());
            track.waveform_preview = if from_library.is_some() {
                counts.library += 1;
                from_library
            } else {
                track.usb_analysis_path.as_deref().and_then(|path| {
                    self.read_usb_file(FileKind::Waveform, Path::new(path), &mut counts)
                })
            };
        }

        if track.artwork_data_url.is_none() {
            let from_library = library
                .and_then(|files| files.artwork_path.as_deref())
                .map(Path::new)
                .and_then(artwork_path_to_data_url);
            track.artwork_data_url = if from_library.is_some() {
                counts.library += 1;
                from_library
            } else {
                track.artwork_path.as_deref().and_then(|path| {
                    self.read_usb_file(FileKind::Artwork, Path::new(path), &mut counts)
                        .and_then(|bytes| String::from_utf8(bytes).ok())
                })
            };
        }
        counts
    }

    fn read_usb_file(
        &self,
        kind: FileKind,
        path: &Path,
        counts: &mut DisplayReadCounts,
    ) -> Option<Vec<u8>> {
        let read = || match kind {
            FileKind::Waveform => load_waveform_preview_from_analysis_path(path),
            FileKind::Artwork => artwork_path_to_data_url(path).map(String::into_bytes),
        };
        let Some(cache) = &self.cache else {
            counts.usb += 1;
            return read();
        };
        let sig = match kind {
            FileKind::Waveform => waveform_signature(path),
            FileKind::Artwork => file_signature(path),
        };
        // Nothing to stat -> nothing to read either.
        let sig = sig?;
        let usb_path = self.stick_relative_path(path);
        match cached_read(cache, kind, &usb_path, &sig) {
            Ok(Some(data)) => {
                counts.cache += 1;
                return data;
            }
            Ok(None) => {}
            Err(err) => logging::emit(
                Level::Warn,
                "usb-import",
                &format!("USB read cache lookup failed for {usb_path}: {err}"),
            ),
        }
        counts.usb += 1;
        let data = read();
        // A file with nothing usable is cached too (as NULL), so it isn't
        // re-read on every visit.
        if let Err(err) = store_read(cache, kind, &usb_path, &sig, data.as_deref()) {
            logging::emit(
                Level::Warn,
                "usb-import",
                &format!("USB read cache store failed for {usb_path}: {err}"),
            );
        }
        data
    }

    /// Path on the stick, independent of where it is mounted.
    fn stick_relative_path(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.usb_root).unwrap_or(path);
        relative.to_string_lossy().replace('\\', "/")
    }
}

/// Keeps only library files that are not on a USB stick (`usb_roots`):
/// reading those would be just as slow as reading the stick being browsed.
pub(crate) fn library_files_off_usb(
    files: LibraryDisplayFiles,
    usb_roots: &[String],
) -> LibraryDisplayFiles {
    let off_usb = |path: Option<String>| {
        path.filter(|path| {
            !path.trim().is_empty()
                && !usb_roots
                    .iter()
                    .any(|root| browse_path_matches_root(path, root))
        })
    };
    LibraryDisplayFiles {
        waveform_path: off_usb(files.waveform_path),
        artwork_path: off_usb(files.artwork_path),
    }
}

fn open_cache(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA busy_timeout = 5000;
        CREATE TABLE IF NOT EXISTS usb_file_reads (
          device_key TEXT NOT NULL,
          kind TEXT NOT NULL,
          usb_path TEXT NOT NULL,
          sig TEXT NOT NULL,
          data BLOB,
          PRIMARY KEY (device_key, kind, usb_path)
        );
        "#,
    )?;
    Ok(conn)
}

/// `Some(data)` on a hit (`data` is `None` for a file with nothing usable).
fn cached_read(
    cache: &ReadCache,
    kind: FileKind,
    usb_path: &str,
    sig: &str,
) -> rusqlite::Result<Option<Option<Vec<u8>>>> {
    cache
        .conn
        .query_row(
            "SELECT data FROM usb_file_reads
             WHERE device_key = ?1 AND kind = ?2 AND usb_path = ?3 AND sig = ?4",
            params![cache.device_key, kind.as_str(), usb_path, sig],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()
}

fn store_read(
    cache: &ReadCache,
    kind: FileKind,
    usb_path: &str,
    sig: &str,
    data: Option<&[u8]>,
) -> rusqlite::Result<()> {
    cache.conn.execute(
        "INSERT OR REPLACE INTO usb_file_reads (device_key, kind, usb_path, sig, data)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![cache.device_key, kind.as_str(), usb_path, sig, data],
    )?;
    Ok(())
}

fn file_signature(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Some(format!("{}:{mtime_ns}", meta.len()))
}

/// The preview can come from any of the `.EXT` / `.2EX` / `.DAT` siblings,
/// so the signature covers each one that exists.
fn waveform_signature(path: &Path) -> Option<String> {
    let parts: Vec<String> = waveform_preview_candidates(path)
        .iter()
        .filter_map(|candidate| {
            let sig = file_signature(candidate)?;
            let ext = candidate
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_uppercase())
                .unwrap_or_default();
            Some(format!("{ext}={sig}"))
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("|"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track_with_artwork(path: &Path) -> UsbTrack {
        UsbTrack {
            id: "1".to_string(),
            title: "T".to_string(),
            artist: "A".to_string(),
            file_path: "/Contents/a.mp3".to_string(),
            artwork_path: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn stick_reads_are_served_from_the_cache_until_the_file_changes() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let usb = tempfile::tempdir().expect("usb");
        let art = usb.path().join("art.jpg");
        std::fs::write(&art, [0xFF, 0xD8, 0xFF, 0xD9]).expect("write art");
        let source =
            || UsbDisplaySource::new(data_dir.path(), usb.path(), "dev".into(), HashMap::new());

        let mut first = track_with_artwork(&art);
        let counts = source().fill(&mut first);
        assert_eq!(counts.usb, 1);
        assert!(first.artwork_data_url.is_some());

        let mut second = track_with_artwork(&art);
        let counts = source().fill(&mut second);
        assert_eq!((counts.cache, counts.usb), (1, 0));
        assert_eq!(second.artwork_data_url, first.artwork_data_url);

        // A different stick with the same path doesn't share the entry.
        let mut other = track_with_artwork(&art);
        let counts =
            UsbDisplaySource::new(data_dir.path(), usb.path(), "dev2".into(), HashMap::new())
                .fill(&mut other);
        assert_eq!(counts.usb, 1);

        std::fs::write(&art, [0xFF, 0xD8, 0x00, 0xFF, 0xD9]).expect("rewrite art");
        let mut changed = track_with_artwork(&art);
        let counts = source().fill(&mut changed);
        assert_eq!(counts.usb, 1);
        assert_ne!(changed.artwork_data_url, first.artwork_data_url);
    }

    #[test]
    fn matched_local_track_supplies_its_own_artwork() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let usb = tempfile::tempdir().expect("usb");
        let local_art = data_dir.path().join("local.png");
        std::fs::write(&local_art, b"\x89PNG local").expect("write local art");
        let usb_art = usb.path().join("art.jpg");
        std::fs::write(&usb_art, [0xFF, 0xD8, 0xFF, 0xD9]).expect("write usb art");

        let library = HashMap::from([(
            "local-1".to_string(),
            LibraryDisplayFiles {
                waveform_path: None,
                artwork_path: Some(local_art.to_string_lossy().into_owned()),
            },
        )]);
        let source = UsbDisplaySource::new(data_dir.path(), usb.path(), "dev".into(), library);

        let mut track = track_with_artwork(&usb_art);
        track.local_track_id = Some("local-1".to_string());
        let counts = source.fill(&mut track);
        assert_eq!((counts.library, counts.usb), (1, 0));
        assert!(
            track
                .artwork_data_url
                .as_deref()
                .is_some_and(|url| url.starts_with("data:image/png;"))
        );

        // An unmatched row still reads the stick.
        let mut unmatched = track_with_artwork(&usb_art);
        let counts = source.fill(&mut unmatched);
        assert_eq!((counts.library, counts.usb), (0, 1));
    }

    #[test]
    fn library_files_on_a_usb_stick_are_not_used() {
        let files = LibraryDisplayFiles {
            waveform_path: Some("/media/STICK/PIONEER/USBANLZ/P000/ANLZ0000.DAT".into()),
            artwork_path: Some("/home/me/.local/share/app/artwork/a.jpg".into()),
        };
        let kept = library_files_off_usb(files, &["/media/STICK".to_string()]);
        assert_eq!(kept.waveform_path, None);
        assert_eq!(
            kept.artwork_path.as_deref(),
            Some("/home/me/.local/share/app/artwork/a.jpg")
        );
    }
}
