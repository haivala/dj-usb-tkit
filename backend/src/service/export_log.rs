use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{BackendError, BackendResult};
use crate::models::{UsbHistory, UsbTrack};

use super::diagnostics::track_identity_key;
use super::export_helpers::{ExportManifest, ExportPlaylistData};
use super::usb_identity::app_marker_dir;
use super::usb_vendor_compat::vendor_db_dir;

const EXPORT_LOG_FILE_NAME: &str = "dj_usb_tkit_export_log.v1.json";
const EXPORT_LOG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsbExportLog {
    pub schema_version: u32,
    pub records: Vec<UsbExportLogRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsbExportLogRecord {
    pub playlist_id: String,
    pub playlist_name: String,
    pub exported_at: String,
    pub export_date: String,
    /// "additive" or "mirror" -- which sync mode produced this record (see
    /// `mirror_playlist_entries`/`options.prune_stale`). `#[serde(default)]`
    /// so logs written before this field existed still parse.
    #[serde(default = "default_export_mode")]
    pub mode: String,
    /// Track identity fingerprints in actual playlist order (not sorted) --
    /// this is the manifest's own track sequence, i.e. what was actually
    /// written to the drive.
    pub track_fingerprints: Vec<String>,
}

fn default_export_mode() -> String {
    "unknown".to_string()
}

impl Default for UsbExportLog {
    fn default() -> Self {
        Self {
            schema_version: EXPORT_LOG_SCHEMA_VERSION,
            records: Vec::new(),
        }
    }
}

pub(crate) fn export_log_path(usb_root: &Path) -> PathBuf {
    app_marker_dir(usb_root).join(EXPORT_LOG_FILE_NAME)
}

/// Where the export log used to live, before it moved into the app's own
/// `.dj-usb-tkit/` marker dir -- it originally piggybacked on rekordbox's
/// vendor directory only because that was convenient, not because it
/// belongs there. Kept only so `load_export_log` can migrate old files
/// forward; nothing writes here anymore.
fn legacy_export_log_path(usb_root: &Path) -> PathBuf {
    vendor_db_dir(usb_root).join(EXPORT_LOG_FILE_NAME)
}

/// Best-effort rename, falling back to copy+remove when the source and
/// destination are on different filesystems (e.g. USB -> internal HDD
/// staging cache), same fallback `usb_backups::move_file` uses.
fn migrate_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dest)?;
    std::fs::remove_file(src)
}

pub(crate) fn append_export_log_record(
    usb_root: &Path,
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
) -> BackendResult<()> {
    let mut log = load_export_log(usb_root)?.unwrap_or_default();
    log.records
        .push(build_export_log_record(playlist, manifest));
    save_export_log(usb_root, &log)
}

pub(crate) fn load_export_log(usb_root: &Path) -> BackendResult<Option<UsbExportLog>> {
    let mut path = export_log_path(usb_root);
    if !path.is_file() {
        let legacy_path = legacy_export_log_path(usb_root);
        if !legacy_path.is_file() {
            return Ok(None);
        }
        // Migrate forward: a file at the old PIONEER/rekordbox location
        // means an older version of the app wrote it there. Move it into
        // place so it only lives in one spot from here on, and future
        // saves land at the new path. If the move itself fails (e.g. a
        // read-only drive), fall back to reading the legacy file in place
        // rather than losing the log entirely.
        if migrate_file(&legacy_path, &path).is_err() {
            path = legacy_path;
        }
    }
    let raw = std::fs::read_to_string(&path)?;
    let parsed = serde_json::from_str::<UsbExportLog>(&raw).map_err(|err| {
        BackendError::Internal(format!("parse USB export log {}: {err}", path.display()))
    })?;
    if parsed.schema_version != EXPORT_LOG_SCHEMA_VERSION {
        return Err(BackendError::Validation(format!(
            "unsupported USB export log schema {} at {}",
            parsed.schema_version,
            path.display()
        )));
    }
    Ok(Some(parsed))
}

pub(crate) fn apply_history_dates_from_export_log(
    histories: &mut [UsbHistory],
    log: Option<&UsbExportLog>,
) {
    let Some(log) = log else {
        return;
    };
    if histories.is_empty() || log.records.is_empty() {
        return;
    }

    let mut latest_by_fingerprint = HashMap::<Vec<String>, (String, String)>::new();
    for record in &log.records {
        if record.track_fingerprints.is_empty() || record.export_date.trim().is_empty() {
            continue;
        }
        let fingerprints = normalize_fingerprints(record.track_fingerprints.iter().cloned());
        let record_sort_key = if record.exported_at.trim().is_empty() {
            record.export_date.clone()
        } else {
            record.exported_at.clone()
        };
        match latest_by_fingerprint.get_mut(&fingerprints) {
            Some((current_sort_key, current_export_date)) => {
                if record_sort_key > *current_sort_key {
                    *current_sort_key = record_sort_key;
                    *current_export_date = record.export_date.clone();
                }
            }
            None => {
                latest_by_fingerprint
                    .insert(fingerprints, (record_sort_key, record.export_date.clone()));
            }
        }
    }

    for history in histories.iter_mut() {
        if history
            .created_at
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            continue;
        }
        let fingerprints = history_track_fingerprints(&history.tracks);
        if fingerprints.is_empty() {
            continue;
        }
        if let Some((_, export_date)) = latest_by_fingerprint.get(&fingerprints) {
            history.created_at = Some(export_date.clone());
        }
    }
}

fn save_export_log(usb_root: &Path, log: &UsbExportLog) -> BackendResult<()> {
    let path = export_log_path(usb_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let encoded = serde_json::to_string_pretty(log)
        .map_err(|err| BackendError::Internal(format!("serialize USB export log: {err}")))?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, encoded)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

pub(crate) fn build_export_log_record(
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
) -> UsbExportLogRecord {
    // `manifest.generated_at` is UTC (see `now()` in service/mod.rs), which
    // is the right choice for the app's own internal bookkeeping timestamps.
    // But this log file lives on the USB next to the PDB/eDB backups, whose
    // filenames (`export_2025-04-23_14-32-01.pdb`) are stamped with local
    // time — so re-express the same instant in local time here to keep the
    // two USB-visible timestamps in the same time zone for a human reading
    // both.
    let exported_at = chrono::DateTime::parse_from_rfc3339(&manifest.generated_at)
        .map(|dt| dt.with_timezone(&chrono::Local).to_rfc3339())
        .unwrap_or_else(|_| manifest.generated_at.clone());
    let export_date = exported_at
        .split('T')
        .next()
        .unwrap_or(exported_at.as_str())
        .to_string();
    let track_fingerprints = fingerprints_in_order(manifest.tracks.iter().map(|track| {
        track_identity_key(
            &track.exported_path,
            &track.title,
            &track.artist,
            Some(&track.id),
        )
    }));
    let mode = if manifest.options.prune_stale {
        "mirror"
    } else {
        "additive"
    }
    .to_string();
    UsbExportLogRecord {
        playlist_id: playlist.id.clone(),
        playlist_name: playlist.name.clone(),
        exported_at,
        export_date,
        mode,
        track_fingerprints,
    }
}

fn history_track_fingerprints(tracks: &[UsbTrack]) -> Vec<String> {
    normalize_fingerprints(tracks.iter().map(|track| {
        track_identity_key(
            track.identity_path(),
            &track.title,
            &track.artist,
            Some(&track.id),
        )
    }))
}

fn normalize_fingerprints<I>(fingerprints: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut out = fingerprints_in_order(fingerprints);
    out.sort();
    out
}

/// Same cleanup as `normalize_fingerprints` (trim, drop empty/"unknown"
/// values) but preserves the caller's order -- used when *writing* a record
/// so the log reflects actual playlist order. Order-sensitive matching
/// (`apply_history_dates_from_export_log`, `history_track_fingerprints`)
/// always re-sorts via `normalize_fingerprints` before comparing, so it
/// doesn't matter that this list isn't sorted.
fn fingerprints_in_order<I>(fingerprints: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    fingerprints
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "unknown")
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::{
        UsbExportLog, UsbExportLogRecord, append_export_log_record,
        apply_history_dates_from_export_log, build_export_log_record, export_log_path,
        legacy_export_log_path, load_export_log,
    };
    use crate::models::{UsbHistory, UsbTrack};
    use crate::service::export_helpers::{ExportManifest, ExportManifestTrack, ExportPlaylistData};
    use tempfile::tempdir;

    fn manifest_track(
        id: &str,
        title: &str,
        artist: &str,
        exported_path: &str,
    ) -> ExportManifestTrack {
        ExportManifestTrack {
            id: id.to_string(),
            master_db_id: None,
            master_content_id: None,
            content_link: None,
            position: 1,
            track_number: None,
            title: title.to_string(),
            artist: artist.to_string(),
            album: None,
            bpm: None,
            key: None,
            source_path: format!("/src/{id}.mp3"),
            exported_path: exported_path.to_string(),
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
            owns_artwork: false,
            owns_waveform: false,
            artwork_path: None,
            waveform_path: None,
            duration_ms: None,
        }
    }

    fn manifest(generated_at: &str, tracks: Vec<ExportManifestTrack>) -> ExportManifest {
        ExportManifest {
            version: 1,
            generated_at: generated_at.to_string(),
            playlist_id: "pl-1".to_string(),
            playlist_name: "Warmup".to_string(),
            usb_root: "/usb".to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: true,
                include_analysis: true,
                prune_stale: true,
                ..Default::default()
            },
            exported_tracks: tracks.len(),
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks,
        }
    }

    fn history_track(id: &str, title: &str, artist: &str, usb_media_path: &str) -> UsbTrack {
        UsbTrack {
            id: id.to_string(),
            local_track_id: None,
            title: title.to_string(),
            artist: artist.to_string(),
            album: None,
            track_number: None,
            bpm: None,
            key: None,
            file_path: usb_media_path.to_string(),
            format_ext: crate::utils::format_ext_from_path(usb_media_path),
            needs_hydration: false,
            format_compat: Default::default(),
            usb_media_path: Some(usb_media_path.to_string()),
            artwork_path: None,
            artwork_data_url: None,
            waveform_peaks_path: None,
            usb_analysis_path: None,
            usb_analysis_path_raw: None,
            waveform_preview: None,
            duration_ms: None,
            file_size_bytes: None,
        }
    }

    /// Expected local calendar date for a UTC RFC3339 instant, computed the
    /// same way `build_export_log_record` does — kept independent of the
    /// machine's time zone so the test isn't flaky on CI/other machines.
    fn expected_local_date(utc_rfc3339: &str) -> String {
        chrono::DateTime::parse_from_rfc3339(utc_rfc3339)
            .expect("valid rfc3339")
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string()
    }

    #[test]
    fn append_export_log_record_keeps_existing_records() {
        let temp = tempdir().expect("tempdir");
        let playlist = ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "Warmup".to_string(),
            tracks: Vec::new(),
        };

        append_export_log_record(
            temp.path(),
            &playlist,
            &manifest(
                "2026-04-03T10:00:00Z",
                vec![manifest_track(
                    "1",
                    "Track One",
                    "Artist",
                    "/Contents/Artist/Album/one.mp3",
                )],
            ),
        )
        .expect("append first record");
        append_export_log_record(
            temp.path(),
            &playlist,
            &manifest(
                "2026-04-04T10:00:00Z",
                vec![manifest_track(
                    "2",
                    "Track Two",
                    "Artist",
                    "/Contents/Artist/Album/two.mp3",
                )],
            ),
        )
        .expect("append second record");

        let loaded = load_export_log(temp.path())
            .expect("load log")
            .expect("log present");
        assert_eq!(loaded.records.len(), 2);
        assert_eq!(
            loaded.records[0].export_date,
            expected_local_date("2026-04-03T10:00:00Z")
        );
        assert_eq!(
            loaded.records[1].export_date,
            expected_local_date("2026-04-04T10:00:00Z")
        );
        assert!(export_log_path(temp.path()).is_file());
    }

    #[test]
    fn build_export_log_record_uses_local_offset_not_utc() {
        // The export log lives on the USB next to the PDB/eDB backup files,
        // whose filenames are stamped with `chrono::Local::now()`. The log's
        // `exported_at` must carry that same local UTC offset instead of the
        // "Z" (UTC) suffix `manifest.generated_at` is stored with, so both
        // USB-visible timestamps read in the same time zone.
        let playlist = ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "Warmup".to_string(),
            tracks: Vec::new(),
        };
        let record = build_export_log_record(&playlist, &manifest("2026-04-03T10:00:00Z", vec![]));

        let expected_offset = chrono::Local::now().offset().to_string();
        assert!(
            record.exported_at.ends_with(&expected_offset),
            "exported_at {:?} should carry the local UTC offset {expected_offset:?}, not the \
             source UTC (Z) offset",
            record.exported_at
        );
        assert_eq!(
            record.export_date,
            expected_local_date("2026-04-03T10:00:00Z")
        );
    }

    #[test]
    fn build_export_log_record_falls_back_to_raw_value_on_unparseable_generated_at() {
        let playlist = ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "Warmup".to_string(),
            tracks: Vec::new(),
        };
        let record = build_export_log_record(&playlist, &manifest("not-a-timestamp", vec![]));
        assert_eq!(record.exported_at, "not-a-timestamp");
        assert_eq!(record.export_date, "not-a-timestamp");
    }

    fn manifest_with_prune_stale(
        generated_at: &str,
        tracks: Vec<ExportManifestTrack>,
        prune_stale: bool,
    ) -> ExportManifest {
        ExportManifest {
            version: 1,
            generated_at: generated_at.to_string(),
            playlist_id: "pl-1".to_string(),
            playlist_name: "Warmup".to_string(),
            usb_root: "/usb".to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: true,
                include_analysis: true,
                prune_stale,
                ..Default::default()
            },
            exported_tracks: tracks.len(),
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks,
        }
    }

    #[test]
    fn build_export_log_record_preserves_playlist_order_not_alphabetical() {
        let playlist = ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "Warmup".to_string(),
            tracks: Vec::new(),
        };
        let record = build_export_log_record(
            &playlist,
            &manifest(
                "2026-04-03T10:00:00Z",
                vec![
                    manifest_track("1", "Zebra", "Artist", "/Contents/Artist/Album/zebra.mp3"),
                    manifest_track("2", "Apple", "Artist", "/Contents/Artist/Album/apple.mp3"),
                    manifest_track("3", "Mango", "Artist", "/Contents/Artist/Album/mango.mp3"),
                ],
            ),
        );
        assert_eq!(record.track_fingerprints.len(), 3);
        assert!(
            record.track_fingerprints[0].contains("zebra"),
            "expected playlist order (Zebra, Apple, Mango), not alphabetical: {:?}",
            record.track_fingerprints
        );
        assert!(record.track_fingerprints[1].contains("apple"));
        assert!(record.track_fingerprints[2].contains("mango"));
    }

    #[test]
    fn build_export_log_record_reports_additive_or_mirror_mode() {
        let playlist = ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "Warmup".to_string(),
            tracks: Vec::new(),
        };
        let additive = build_export_log_record(
            &playlist,
            &manifest_with_prune_stale("2026-04-03T10:00:00Z", vec![], false),
        );
        assert_eq!(additive.mode, "additive");

        let mirror = build_export_log_record(
            &playlist,
            &manifest_with_prune_stale("2026-04-03T10:00:00Z", vec![], true),
        );
        assert_eq!(mirror.mode, "mirror");
    }

    #[test]
    fn usb_export_log_record_defaults_mode_when_reading_a_pre_mode_log() {
        // Logs written before `mode` existed have no such key at all --
        // confirm deserialization still succeeds and falls back rather than
        // failing to parse (which `load_export_log` would otherwise treat
        // as an unreadable log).
        let legacy_json = r#"{
            "playlistId": "pl-1",
            "playlistName": "Warmup",
            "exportedAt": "2026-04-03T10:00:00+00:00",
            "exportDate": "2026-04-03",
            "trackFingerprints": ["fp-1"]
        }"#;
        let record: UsbExportLogRecord =
            serde_json::from_str(legacy_json).expect("legacy record without mode must parse");
        assert_eq!(record.mode, "unknown");
        assert_eq!(record.track_fingerprints, vec!["fp-1".to_string()]);
    }

    #[test]
    fn apply_history_dates_from_export_log_prefers_latest_exact_match() {
        let mut histories = vec![UsbHistory {
            id: "usb-h-1".to_string(),
            name: "History 1".to_string(),
            created_at: None,
            tracks: vec![
                history_track("1", "Track One", "Artist", "/Contents/Artist/Album/one.mp3"),
                history_track("2", "Track Two", "Artist", "/Contents/Artist/Album/two.mp3"),
            ],
            total_duration_ms: 0,
            duration_known_count: 0,
        }];
        let log = UsbExportLog {
            schema_version: 1,
            records: vec![
                UsbExportLogRecord {
                    playlist_id: "old".to_string(),
                    playlist_name: "Older".to_string(),
                    exported_at: "2026-04-03T09:00:00Z".to_string(),
                    export_date: "2026-04-03".to_string(),
                    mode: "additive".to_string(),
                    track_fingerprints: build_export_log_record(
                        &ExportPlaylistData {
                            id: "pl".to_string(),
                            name: "Warmup".to_string(),
                            tracks: Vec::new(),
                        },
                        &manifest(
                            "2026-04-03T09:00:00Z",
                            vec![
                                manifest_track(
                                    "1",
                                    "Track One",
                                    "Artist",
                                    "/Contents/Artist/Album/one.mp3",
                                ),
                                manifest_track(
                                    "2",
                                    "Track Two",
                                    "Artist",
                                    "/Contents/Artist/Album/two.mp3",
                                ),
                            ],
                        ),
                    )
                    .track_fingerprints,
                },
                UsbExportLogRecord {
                    playlist_id: "new".to_string(),
                    playlist_name: "Newer".to_string(),
                    exported_at: "2026-04-04T11:00:00Z".to_string(),
                    export_date: "2026-04-04".to_string(),
                    mode: "additive".to_string(),
                    track_fingerprints: build_export_log_record(
                        &ExportPlaylistData {
                            id: "pl".to_string(),
                            name: "Warmup".to_string(),
                            tracks: Vec::new(),
                        },
                        &manifest(
                            "2026-04-04T11:00:00Z",
                            vec![
                                manifest_track(
                                    "1",
                                    "Track One",
                                    "Artist",
                                    "/Contents/Artist/Album/one.mp3",
                                ),
                                manifest_track(
                                    "2",
                                    "Track Two",
                                    "Artist",
                                    "/Contents/Artist/Album/two.mp3",
                                ),
                            ],
                        ),
                    )
                    .track_fingerprints,
                },
            ],
        };

        apply_history_dates_from_export_log(&mut histories, Some(&log));
        assert_eq!(histories[0].created_at.as_deref(), Some("2026-04-04"));
    }

    #[test]
    fn load_export_log_migrates_legacy_path_forward() {
        let temp = tempdir().expect("tempdir");
        let legacy_path = legacy_export_log_path(temp.path());
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        let log = UsbExportLog {
            schema_version: 1,
            records: vec![UsbExportLogRecord {
                playlist_id: "pl-1".to_string(),
                playlist_name: "Warmup".to_string(),
                exported_at: "2026-04-03T10:00:00+00:00".to_string(),
                export_date: "2026-04-03".to_string(),
                mode: "additive".to_string(),
                track_fingerprints: vec!["fp-1".to_string()],
            }],
        };
        std::fs::write(&legacy_path, serde_json::to_string_pretty(&log).unwrap()).unwrap();

        let loaded = load_export_log(temp.path())
            .expect("load")
            .expect("log present via legacy fallback");
        assert_eq!(loaded.records.len(), 1);
        assert!(
            export_log_path(temp.path()).is_file(),
            "reading a legacy log must relocate it to the new .dj-usb-tkit/ path"
        );
        assert!(
            !legacy_path.is_file(),
            "old PIONEER/rekordbox file must not be left behind after migration"
        );

        // A subsequent append must write only to the new path, never
        // recreating the legacy one.
        let playlist = ExportPlaylistData {
            id: "pl-2".to_string(),
            name: "Another".to_string(),
            tracks: Vec::new(),
        };
        append_export_log_record(
            temp.path(),
            &playlist,
            &manifest("2026-04-05T10:00:00Z", vec![]),
        )
        .expect("append after migration");
        assert!(!legacy_path.is_file());
    }
}
