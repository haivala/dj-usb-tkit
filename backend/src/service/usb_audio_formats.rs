//! USB audio formats: per USB stick, which audio formats export puts on it, in
//! the user's order of preference. Stored in the stick's
//! `drive.json` (see `usb_identity::UsbAudioFormats`), so the setting travels
//! with the stick.
//!
//! USB rows in an unticked format get a warning (see
//! `format_compat::compute_usb_format_compat`), and export picks each track's
//! file by it (see `transcode_export`).

use crate::audio_format::{
    UsbAudioFormat, UsbAudioFormatOfExtension, format_ext_from_path, m4a_is_alac_by_bit_depth,
};
use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::models::{
    GetUsbAudioFormatsRequest, SetUsbAudioFormatsRequest, UsbAudioFormatSetting,
    UsbAudioFormatsData,
};

use super::BackendService;
use super::usb_identity::{UsbAudioFormats, read_usb_audio_formats, write_usb_audio_formats};
use super::usb_utils::{can_write_file_at, has_write_access, resolve_usb_root};

fn formats_data(formats: &UsbAudioFormats, writable: bool) -> UsbAudioFormatsData {
    let ticked = formats
        .entries()
        .iter()
        .filter(|(_, allowed)| *allowed)
        .count();
    UsbAudioFormatsData {
        formats: formats
            .entries()
            .iter()
            .map(|&(format, allowed)| {
                // At least one stays ticked (`UsbAudioFormats::from_request`).
                let last_ticked = allowed && ticked == 1;
                UsbAudioFormatSetting {
                    format: format.id().to_string(),
                    label: format.label().to_string(),
                    allowed,
                    locked: !writable || last_ticked,
                    tooltip: last_ticked.then(|| tr::usb_audio_formats_last_ticked().render()),
                }
            })
            .collect(),
        writable,
        hint: if writable {
            tr::usb_audio_formats_hint()
        } else {
            tr::usb_audio_formats_read_only()
        }
        .render(),
    }
}

impl BackendService {
    pub fn get_usb_audio_formats(
        &self,
        req: GetUsbAudioFormatsRequest,
    ) -> BackendResult<UsbAudioFormatsData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        Ok(formats_data(
            &read_usb_audio_formats(&usb_root),
            has_write_access(&usb_root),
        ))
    }

    /// Writes the whole ordered list to the stick's `drive.json`. USB rows
    /// are judged against it on every page load, so nothing cached needs
    /// clearing.
    pub fn set_usb_audio_formats(
        &self,
        req: SetUsbAudioFormatsRequest,
    ) -> BackendResult<UsbAudioFormatsData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let items: Vec<(String, bool)> = req
            .formats
            .iter()
            .map(|item| (item.format.clone(), item.allowed))
            .collect();
        let formats = UsbAudioFormats::from_request(&items).map_err(BackendError::Validation)?;
        if !can_write_file_at(&usb_root) {
            return Err(BackendError::Validation(
                "The USB is read-only, so its USB audio formats can't be changed".to_string(),
            ));
        }
        write_usb_audio_formats(&usb_root, &formats).map_err(|err| {
            BackendError::Internal(format!("failed to write the USB audio formats: {err}"))
        })?;
        Ok(formats_data(&formats, true))
    }
}

/// A library track's Player format. `.m4a` is ALAC when the scan recorded a
/// bit depth (only ALAC has one), otherwise AAC. `None` for a format with no
/// USB audio formats entry.
pub(crate) fn library_usb_audio_format(
    file_path: &str,
    bit_depth: Option<u8>,
) -> Option<UsbAudioFormat> {
    let ext = format_ext_from_path(file_path)?;
    match UsbAudioFormat::of_extension(&ext) {
        UsbAudioFormatOfExtension::Known(format) => Some(format),
        UsbAudioFormatOfExtension::AlacOrAac => Some(if m4a_is_alac_by_bit_depth(bit_depth) {
            UsbAudioFormat::Alac
        } else {
            UsbAudioFormat::Aac
        }),
        UsbAudioFormatOfExtension::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m4a_in_the_library_is_alac_when_it_has_a_bit_depth() {
        assert_eq!(
            library_usb_audio_format("/m/a.m4a", Some(24)),
            Some(UsbAudioFormat::Alac)
        );
        assert_eq!(
            library_usb_audio_format("/m/a.m4a", None),
            Some(UsbAudioFormat::Aac)
        );
        assert_eq!(
            library_usb_audio_format("/m/a.FLAC", None),
            Some(UsbAudioFormat::Flac)
        );
        assert_eq!(library_usb_audio_format("/m/a.ogg", None), None);
    }

    #[test]
    fn set_usb_audio_formats_refuses_a_usb_it_cannot_write() {
        let dir = tempfile::tempdir().unwrap();
        let service = BackendService::new(dir.path()).unwrap();
        let usb = tempfile::tempdir().unwrap();
        let root = usb.path().to_string_lossy().to_string();
        let list = service
            .get_usb_audio_formats(GetUsbAudioFormatsRequest {
                usb_root: root.clone(),
            })
            .unwrap()
            .formats;
        let mut perms = std::fs::metadata(usb.path()).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(usb.path(), perms.clone()).unwrap();
        // Root writes anyway, so there's nothing to refuse.
        let writes = can_write_file_at(usb.path());
        let result = service.set_usb_audio_formats(SetUsbAudioFormatsRequest {
            usb_root: root,
            formats: list,
        });
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(usb.path(), perms).unwrap();
        if writes {
            return;
        }
        match result.unwrap_err() {
            BackendError::Validation(message) => assert_eq!(
                message,
                "The USB is read-only, so its USB audio formats can't be changed"
            ),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn set_usb_audio_formats_refuses_an_empty_selection_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let service = BackendService::new(dir.path()).unwrap();
        let usb = tempfile::tempdir().unwrap();
        let root = usb.path().to_string_lossy().to_string();
        let mut list = service
            .get_usb_audio_formats(GetUsbAudioFormatsRequest {
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
            .set_usb_audio_formats(SetUsbAudioFormatsRequest {
                usb_root: root.clone(),
                formats: list.clone(),
            })
            .unwrap_err();
        assert!(matches!(err, BackendError::Validation(_)));

        list.swap(0, 4);
        list[0].allowed = true;
        let saved = service
            .set_usb_audio_formats(SetUsbAudioFormatsRequest {
                usb_root: root.clone(),
                formats: list,
            })
            .unwrap();
        let read = service
            .get_usb_audio_formats(GetUsbAudioFormatsRequest { usb_root: root })
            .unwrap();
        let ids = |data: &UsbAudioFormatsData| {
            data.formats
                .iter()
                .map(|item| (item.format.clone(), item.allowed))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&read), ids(&saved));
        assert_eq!(read.formats[0].format, "mp3");
        assert_eq!(read.formats[0].label, "MP3");
        assert!(read.writable);
        // MP3 is the only one ticked: its box is locked, and says why.
        assert!(read.formats[0].locked);
        assert_eq!(
            read.formats[0].tooltip.as_deref(),
            Some("At least one format stays ticked")
        );
        assert!(
            read.formats[1..]
                .iter()
                .all(|item| !item.locked && item.tooltip.is_none())
        );
        assert_eq!(
            read.hint,
            "The formats export puts on this USB · drag to set the order of preference"
        );
    }
}
