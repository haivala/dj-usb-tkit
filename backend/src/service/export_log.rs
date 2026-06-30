use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{BackendError, BackendResult};
use crate::models::{UsbHistory, UsbTrack};

use super::diagnostics::track_identity_key;
use super::export_helpers::{ExportManifest, ExportPlaylistData};
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
    pub track_fingerprints: Vec<String>,
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
    vendor_db_dir(usb_root).join(EXPORT_LOG_FILE_NAME)
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
    let path = export_log_path(usb_root);
    if !path.is_file() {
        return Ok(None);
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

fn build_export_log_record(
    playlist: &ExportPlaylistData,
    manifest: &ExportManifest,
) -> UsbExportLogRecord {
    let exported_at = manifest.generated_at.clone();
    let export_date = exported_at
        .split('T')
        .next()
        .unwrap_or(exported_at.as_str())
        .to_string();
    let track_fingerprints = normalize_fingerprints(manifest.tracks.iter().map(|track| {
        track_identity_key(
            &track.exported_path,
            &track.title,
            &track.artist,
            Some(&track.id),
        )
    }));
    UsbExportLogRecord {
        playlist_id: playlist.id.clone(),
        playlist_name: playlist.name.clone(),
        exported_at,
        export_date,
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
    let mut out = fingerprints
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "unknown")
        .collect::<Vec<_>>();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::{
        UsbExportLog, UsbExportLogRecord, append_export_log_record,
        apply_history_dates_from_export_log, build_export_log_record, export_log_path,
        load_export_log,
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
            usb_media_path: Some(usb_media_path.to_string()),
            artwork_path: None,
            artwork_data_url: None,
            waveform_peaks_path: None,
            usb_analysis_path: None,
            usb_analysis_path_raw: None,
            waveform_preview: None,
            duration_ms: None,
        }
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
        assert_eq!(loaded.records[0].export_date, "2026-04-03");
        assert_eq!(loaded.records[1].export_date, "2026-04-04");
        assert!(export_log_path(temp.path()).is_file());
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
        }];
        let log = UsbExportLog {
            schema_version: 1,
            records: vec![
                UsbExportLogRecord {
                    playlist_id: "old".to_string(),
                    playlist_name: "Older".to_string(),
                    exported_at: "2026-04-03T09:00:00Z".to_string(),
                    export_date: "2026-04-03".to_string(),
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
}
