//! The files on the stick: its Contents audio files, its ANLZ analysis
//! bundles, and which bundle the app wrote for which track.

use super::root::resolve_usb_side_path;
use super::vendor_compat::{
    USB_ANALYSIS_DIR, USB_CONTENTS_DIR, USB_VENDOR_ROOT_DIR, USB_VENDOR_ROOT_PREFIX,
};
use crate::edb::open_edb_from_usb_root;
use crate::i18n::tr;
use crate::logging;
use crate::logging::Level;
use crate::models::WarningEntry;
use crate::pdb_reader::parse_pdb;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// What [`scan_anlz`] found under USBANLZ: warnings for the event log, and
/// the empty files themselves for the repair to act on.
#[derive(Default)]
pub(crate) struct AnlzScan {
    pub(crate) warnings: Vec<WarningEntry>,
    /// Empty `.DAT`/`.EXT`/`.2EX` bundle members.
    pub(crate) empty_files: Vec<PathBuf>,
    /// Empty files that aren't bundle members at all.
    pub(crate) empty_malformed_entries: Vec<PathBuf>,
}

pub(crate) fn scan_anlz(usb_root: &Path) -> AnlzScan {
    let anlz_root = usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR);
    if !anlz_root.exists() {
        return AnlzScan::default();
    }

    let mut scan = AnlzScan::default();
    let warnings = &mut scan.warnings;
    for entry in WalkDir::new(&anlz_root) {
        match entry {
            Ok(e) => {
                // Surface malformed/anomalous entries under USBANLZ as diagnostics warnings.
                if is_malformed_anlz_entry_name(e.path()) {
                    warnings.push(logging::log_text(
                        Level::Warn,
                        "usb-analysis",
                        "usb.analysis.entry-malformed",
                        &tr::diagnostics_warning_anlz_malformed(sanitize_warning_path(e.path())),
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
                    if let Ok(md) = e.metadata()
                        && md.len() == 0
                    {
                        scan.empty_malformed_entries.push(e.path().to_path_buf());
                        warnings.push(logging::log_text(
                            Level::Warn,
                            "usb-analysis",
                            "usb.analysis.malformed-entry-empty",
                            &tr::diagnostics_warning_anlz_malformed_empty(sanitize_warning_path(
                                e.path(),
                            )),
                        ));
                    }
                    warnings.push(logging::log_text(
                        Level::Warn,
                        "usb-analysis",
                        "usb.analysis.entry-malformed",
                        &tr::diagnostics_warning_anlz_malformed(sanitize_warning_path(e.path())),
                    ));
                    continue;
                }
                match e.metadata() {
                    Ok(md) => {
                        if md.len() == 0 {
                            scan.empty_files.push(e.path().to_path_buf());
                            warnings.push(logging::log_text(
                                Level::Warn,
                                "usb-analysis",
                                "usb.analysis.file-empty",
                                &tr::diagnostics_warning_anlz_empty(sanitize_warning_path(
                                    e.path(),
                                )),
                            ));
                        }
                    }
                    Err(err) => warnings.push(logging::log_text(
                        Level::Error,
                        "usb-analysis",
                        "usb.analysis.metadata-read-failed",
                        &tr::diagnostics_warning_anlz_unreadable(
                            sanitize_warning_path(e.path()),
                            err,
                        ),
                    )),
                }
            }
            Err(err) => warnings.push(logging::log_text(
                Level::Error,
                "usb-analysis",
                "usb.analysis.walk-failed",
                &tr::diagnostics_warning_anlz_walk_failed(err),
            )),
        }
    }
    scan
}

fn is_known_anlz_metadata_file(path: &Path) -> bool {
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

fn is_malformed_anlz_entry_name(path: &Path) -> bool {
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

pub(crate) fn sanitize_warning_path(path: &Path) -> String {
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

/// Whether the `.DAT` + `.EXT` pair of an analysis bundle exists. `anlz_path`
/// may name any member of the bundle: a rekordbox import stores the `.EXT`.
/// The `.2EX` is optional, since rekordbox's own bundles from older analyses
/// have none, and exporting the pair is still what rekordbox would put on a stick.
pub(crate) fn analysis_bundle_exists(usb_root: &Path, anlz_path: &str) -> bool {
    let Some(path) = resolve_usb_side_path(usb_root, anlz_path) else {
        return false;
    };
    path.with_extension("DAT").is_file() && path.with_extension("EXT").is_file()
}

/// Whether a library track's analysis bundle is gone from the local disk: an
/// absolute `anlz_path` whose `.DAT` or `.EXT` is missing (see
/// [`analysis_bundle_exists`]). A USB-side path (`/PIONEER/...`,
/// `/Contents/...`) only resolves against a stick, so it is not checked here;
/// the export gate checks it against the target.
pub(crate) fn local_analysis_bundle_missing(anlz_path: &str) -> bool {
    let normalized = anlz_path.trim().replace('\\', "/");
    let usb_side = normalized.starts_with(USB_VENDOR_ROOT_PREFIX)
        || normalized.starts_with(&format!("/{USB_CONTENTS_DIR}/"));
    let path = Path::new(&normalized);
    !usb_side
        && path.is_absolute()
        && !(path.with_extension("DAT").is_file() && path.with_extension("EXT").is_file())
}

pub(crate) fn load_existing_analysis_paths_by_content_path(
    usb_root: &Path,
    warnings: &mut Vec<WarningEntry>,
) -> HashMap<String, String> {
    let Some(conn) = open_edb_from_usb_root(usb_root, warnings) else {
        return HashMap::new();
    };
    load_existing_analysis_paths_by_content_path_with_conn(&conn)
}

pub(crate) fn load_existing_analysis_paths_by_content_path_with_conn(
    conn: &rusqlite::Connection,
) -> HashMap<String, String> {
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
        out.insert(
            crate::service::export_helpers::canonicalize_track_path_identity(p),
            a.to_string(),
        );
    }
    out
}

pub(crate) fn load_existing_analysis_paths_by_pdb_track_path(
    usb_root: &Path,
) -> HashMap<String, String> {
    let Ok(Some(databases)) = crate::service::usb::staging::UsbDatabases::present(usb_root) else {
        return HashMap::new();
    };
    let Some(Ok(parsed)) = databases.pdb().map(parse_pdb) else {
        return HashMap::new();
    };
    let mut out = HashMap::<String, String>::new();
    for track in parsed.tracks {
        let path = track.track_file_path.trim();
        let anlz = track.anlz_path.trim();
        if path.is_empty() || anlz.is_empty() {
            continue;
        }
        out.insert(
            crate::service::export_helpers::canonicalize_track_path_identity(path),
            anlz.to_string(),
        );
    }
    out
}

/// Every file under `Contents/`, USB-relative (`/Contents/...`). Not filtered
/// by type: an indexed path is found whatever its format (an exported `.ogg`
/// is on the stick, not missing), and `scan_audio_presence` decides which
/// unindexed files are audio.
pub(crate) fn collect_contents_files(usb_root: &Path) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WarningEntry;
    use crate::service::usb::vendor_compat::{
        USB_ANALYSIS_DIR, USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR,
    };
    use uuid::Uuid;

    #[test]
    fn scan_anlz_ignores_usbmng_dat() {
        let temp = tempfile::tempdir().expect("tempdir");
        let anlz_root = temp.path().join(USB_VENDOR_ROOT_DIR).join(USB_ANALYSIS_DIR);
        std::fs::create_dir_all(&anlz_root).expect("create USBANLZ");
        std::fs::write(anlz_root.join("USBMNG.DAT"), b"stub").expect("write USBMNG.DAT");

        let warnings = scan_anlz(temp.path()).warnings;

        assert!(warnings.iter().all(|w| !w.message.contains("USBMNG.DAT")
            && !w.message.contains("analysis entry malformed")));
    }

    #[test]
    fn scan_anlz_returns_empty_files_as_real_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        // The control character is escaped in the warning text, so only the
        // returned path still names the file on disk.
        let anlz_dir = temp
            .path()
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_ANALYSIS_DIR)
            .join("P001")
            .join("0000\u{7}001");
        std::fs::create_dir_all(&anlz_dir).expect("create ANLZ dir");
        let empty = anlz_dir.join("ANLZ0000.DAT");
        std::fs::write(&empty, b"").expect("write empty DAT");
        std::fs::write(anlz_dir.join("ANLZ0000.EXT"), b"stub").expect("write EXT");

        let scan = scan_anlz(temp.path());

        assert_eq!(scan.empty_files, vec![empty.clone()]);
        assert!(scan.empty_malformed_entries.is_empty());
        assert!(
            scan.warnings
                .iter()
                .any(|w| w.code == "usb.analysis.file-empty" && w.message.contains("\\x07")),
            "the warning still shows the escaped name: {:?}",
            scan.warnings
        );
    }

    #[test]
    fn scan_anlz_accepts_numbered_anlz_bundle_members() {
        let temp = tempfile::tempdir().expect("tempdir");
        let anlz_dir = temp
            .path()
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_ANALYSIS_DIR)
            .join("P04F")
            .join("00019DDD");
        std::fs::create_dir_all(&anlz_dir).expect("create ANLZ dir");
        std::fs::write(anlz_dir.join("ANLZ0001.2EX"), b"stub").expect("write numbered 2EX");

        let warnings = scan_anlz(temp.path()).warnings;

        assert!(
            warnings.iter().all(|w| !w.message.contains("ANLZ0001.2EX")
                && !w.message.contains("analysis entry malformed")),
            "numbered ANLZ bundle members should not be treated as malformed"
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

        let mut warnings = Vec::<WarningEntry>::new();
        let map = load_existing_analysis_paths_by_content_path(&tmp, &mut warnings);
        let anlz = map
            .get(
                &crate::service::export_helpers::canonicalize_track_path_identity(
                    "/Contents/Artist/Album/Track.mp3",
                ),
            )
            .cloned()
            .expect("analysis path should be mapped");
        assert!(
            analysis_bundle_exists(&tmp, &anlz),
            "bundle should be detected"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }
}
