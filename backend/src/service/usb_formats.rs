//! Player formats: per USB stick, which audio formats the players it goes
//! into read, in the user's order of preference. Stored in the stick's
//! `drive.json` (see `usb_identity::DriveFormats`), so the setting travels
//! with the stick.
//!
//! The app warns with it: USB rows in an unticked format (see
//! `format_compat::compute_usb_format_compat`), and the app playlist heading
//! counts the playlist's tracks in one (`PlaylistExportSpace`). Export
//! doesn't change yet.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::params_from_iter;

use crate::audio_format::{
    PlayerFormat, PlayerFormatOfExtension, format_ext_from_path, m4a_is_alac_by_bit_depth,
};
use crate::edb::ExportTrackData;
use crate::error::{BackendError, BackendResult};
use crate::models::{GetUsbFormatsRequest, SetUsbFormatsRequest, UsbFormatSetting, UsbFormatsData};

use super::BackendService;
use super::usb_identity::{DriveFormats, read_drive_formats, write_drive_formats};
use super::usb_utils::{has_write_access, resolve_usb_root};

fn formats_data(formats: &DriveFormats, writable: bool) -> UsbFormatsData {
    UsbFormatsData {
        formats: formats
            .entries()
            .iter()
            .map(|&(format, allowed)| UsbFormatSetting {
                format: format.id().to_string(),
                label: format.label().to_string(),
                allowed,
            })
            .collect(),
        writable,
    }
}

impl BackendService {
    pub fn get_usb_formats(&self, req: GetUsbFormatsRequest) -> BackendResult<UsbFormatsData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        Ok(formats_data(
            &read_drive_formats(&usb_root),
            has_write_access(&usb_root),
        ))
    }

    /// Writes the whole ordered list to the stick's `drive.json`. USB rows
    /// are judged against it on every page load, so nothing cached needs
    /// clearing.
    pub fn set_usb_formats(&self, req: SetUsbFormatsRequest) -> BackendResult<UsbFormatsData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let items: Vec<(String, bool)> = req
            .formats
            .iter()
            .map(|item| (item.format.clone(), item.allowed))
            .collect();
        let formats = DriveFormats::from_request(&items).map_err(BackendError::Validation)?;
        if !has_write_access(&usb_root) {
            return Err(BackendError::Validation(
                "The USB is read-only, so its Player formats can't be changed".to_string(),
            ));
        }
        write_drive_formats(&usb_root, &formats).map_err(|err| {
            BackendError::Internal(format!("failed to write the USB's Player formats: {err}"))
        })?;
        Ok(formats_data(&formats, true))
    }
}

/// A library track's Player format. `.m4a` is ALAC when the scan recorded a
/// bit depth (only ALAC has one), otherwise AAC. `None` for a format with no
/// Player formats entry.
pub(crate) fn library_player_format(
    file_path: &str,
    bit_depth: Option<u8>,
) -> Option<PlayerFormat> {
    let ext = format_ext_from_path(file_path)?;
    match PlayerFormat::of_extension(&ext) {
        PlayerFormatOfExtension::Known(format) => Some(format),
        PlayerFormatOfExtension::AlacOrAac => Some(if m4a_is_alac_by_bit_depth(bit_depth) {
            PlayerFormat::Alac
        } else {
            PlayerFormat::Aac
        }),
        PlayerFormatOfExtension::None => None,
    }
}

/// The playlist heading's Player formats counts.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DisallowedTracks {
    pub(crate) tracks: usize,
    pub(crate) with_transcode: usize,
    /// The label of those transcodes' format, when they all share one.
    pub(crate) transcode_format: Option<String>,
}

/// How many of `tracks` are in a format `formats` leaves unticked, and how
/// many of those have a transcode (whose file exists) in a ticked format;
/// the most preferred one counts.
pub(crate) fn count_disallowed_tracks(
    conn: &rusqlite::Connection,
    tracks: &[ExportTrackData],
    formats: &DriveFormats,
) -> BackendResult<DisallowedTracks> {
    let disallowed: Vec<&ExportTrackData> = tracks
        .iter()
        .filter(|track| {
            library_player_format(&track.file_path, track.bit_depth)
                .is_some_and(|format| !formats.allows(format))
        })
        .collect();
    let mut result = DisallowedTracks {
        tracks: disallowed.len(),
        ..Default::default()
    };
    if disallowed.is_empty() {
        return Ok(result);
    }

    let mut transcodes = HashMap::<String, Vec<PlayerFormat>>::new();
    let ids: Vec<&str> = disallowed.iter().map(|track| track.id.as_str()).collect();
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT track_id, format, file_path FROM track_transcodes
             WHERE track_id IN ({placeholders})"
        ))?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (track_id, format, path) = row?;
            if let Some(format) = PlayerFormat::parse(&format)
                && formats.allows(format)
                && Path::new(&path).is_file()
            {
                transcodes.entry(track_id).or_default().push(format);
            }
        }
    }

    let mut seen_format: Option<PlayerFormat> = None;
    let mut one_format = true;
    for track in &disallowed {
        let Some(available) = transcodes.get(&track.id) else {
            continue;
        };
        let Some(preferred) = formats
            .entries()
            .iter()
            .map(|&(format, _)| format)
            .find(|format| available.contains(format))
        else {
            continue;
        };
        result.with_transcode += 1;
        match seen_format {
            None => seen_format = Some(preferred),
            Some(seen) if seen != preferred => one_format = false,
            Some(_) => {}
        }
    }
    if one_format {
        result.transcode_format = seen_format.map(|format| format.label().to_string());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formats_without(unticked: &[&str]) -> DriveFormats {
        let items: Vec<(String, bool)> = PlayerFormat::DEFAULT_ORDER
            .into_iter()
            .map(|format| (format.id().to_string(), !unticked.contains(&format.id())))
            .collect();
        DriveFormats::from_request(&items).unwrap()
    }

    fn track(id: &str, file_path: &str, bit_depth: Option<u8>) -> ExportTrackData {
        ExportTrackData {
            id: id.to_string(),
            file_path: file_path.to_string(),
            bit_depth,
            ..Default::default()
        }
    }

    fn conn_with_transcodes(rows: &[(&str, &str, &Path)]) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE track_transcodes (track_id TEXT, format TEXT, file_path TEXT)",
        )
        .unwrap();
        for (track_id, format, path) in rows {
            conn.execute(
                "INSERT INTO track_transcodes VALUES (?1, ?2, ?3)",
                rusqlite::params![track_id, format, path.to_string_lossy()],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn m4a_in_the_library_is_alac_when_it_has_a_bit_depth() {
        assert_eq!(
            library_player_format("/m/a.m4a", Some(24)),
            Some(PlayerFormat::Alac)
        );
        assert_eq!(
            library_player_format("/m/a.m4a", None),
            Some(PlayerFormat::Aac)
        );
        assert_eq!(
            library_player_format("/m/a.FLAC", None),
            Some(PlayerFormat::Flac)
        );
        assert_eq!(library_player_format("/m/a.ogg", None), None);
    }

    #[test]
    fn counts_disallowed_tracks_and_their_allowed_transcodes() {
        let dir = tempfile::tempdir().unwrap();
        let mp3 = dir.path().join("a.mp3");
        let wav = dir.path().join("c.wav");
        std::fs::write(&mp3, b"x").unwrap();
        std::fs::write(&wav, b"x").unwrap();
        let gone = dir.path().join("gone.mp3");
        let tracks = [
            track("a", "/m/a.flac", Some(16)),
            track("b", "/m/b.m4a", Some(24)),
            track("c", "/m/c.flac", Some(24)),
            track("d", "/m/d.mp3", None),
            track("e", "/m/e.flac", None),
        ];
        let conn =
            conn_with_transcodes(&[("a", "mp3", &mp3), ("c", "wav", &wav), ("e", "mp3", &gone)]);

        let no_lossless = formats_without(&["flac", "alac", "wav"]);
        assert_eq!(
            count_disallowed_tracks(&conn, &tracks, &no_lossless).unwrap(),
            DisallowedTracks {
                tracks: 4,
                with_transcode: 1,
                transcode_format: Some("MP3".to_string()),
            }
        );

        let no_flac = formats_without(&["flac"]);
        assert_eq!(
            count_disallowed_tracks(&conn, &tracks, &no_flac).unwrap(),
            DisallowedTracks {
                tracks: 3,
                with_transcode: 2,
                transcode_format: None,
            }
        );

        assert_eq!(
            count_disallowed_tracks(&conn, &tracks, &DriveFormats::default()).unwrap(),
            DisallowedTracks::default()
        );
    }

    #[test]
    fn set_usb_formats_refuses_an_empty_selection_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let service = BackendService::new(dir.path()).unwrap();
        let usb = tempfile::tempdir().unwrap();
        let root = usb.path().to_string_lossy().to_string();
        let mut list = service
            .get_usb_formats(GetUsbFormatsRequest {
                usb_root: root.clone(),
            })
            .unwrap()
            .formats;
        assert_eq!(list.len(), 6);
        assert!(list.iter().all(|item| item.allowed));

        for item in &mut list {
            item.allowed = false;
        }
        let err = service
            .set_usb_formats(SetUsbFormatsRequest {
                usb_root: root.clone(),
                formats: list.clone(),
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::Validation(_)));

        list.swap(0, 4);
        list[0].allowed = true;
        let saved = service
            .set_usb_formats(SetUsbFormatsRequest {
                usb_root: root.clone(),
                formats: list,
            })
            .unwrap();
        let read = service
            .get_usb_formats(GetUsbFormatsRequest { usb_root: root })
            .unwrap();
        let ids = |data: &UsbFormatsData| {
            data.formats
                .iter()
                .map(|item| (item.format.clone(), item.allowed))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&read), ids(&saved));
        assert_eq!(read.formats[0].format, "mp3");
        assert_eq!(read.formats[0].label, "MP3");
        assert!(read.writable);
    }
}
