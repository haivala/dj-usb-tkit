//! Player formats: per USB stick, which audio formats export puts on it, in
//! the user's order of preference. Stored in the stick's
//! `drive.json` (see `usb_identity::DriveFormats`), so the setting travels
//! with the stick.
//!
//! USB rows in an unticked format get a warning (see
//! `format_compat::compute_usb_format_compat`), and export picks each track's
//! file by it (see `transcode_export`).

use crate::audio_format::{
    PlayerFormat, PlayerFormatOfExtension, format_ext_from_path, m4a_is_alac_by_bit_depth,
};
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

#[cfg(test)]
mod tests {
    use super::*;

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
