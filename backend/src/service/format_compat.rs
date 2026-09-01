//! CDJ (Pioneer / rekordbox) playback-compatibility judgement for a track's
//! audio format.
//!
//! This rule used to live in the frontend (`vanilla-ui/track_table.mjs`
//! `validateFormatCompatibility` / `describeTrackFormat`). It moved here so it
//! has one home and so the "will be auto-converted on export" claim can't
//! drift from what the export pipeline actually does -- the WAV autofix branch
//! and `copy_wav_normalized_if_needed` both go through
//! [`WavFormatIssue::is_export_autofixable`].
//!
//! Inputs are all fields already present on `Track` (`UsbTrack` only carries
//! `format_ext`, so its verdict is limited to "unlisted format"). Missing
//! technical fields mean "can't judge" -> `Ok`, matching the old frontend
//! behaviour where a `null` sample rate / bitrate short-circuited the check.

use crate::models::{FormatCompat, FormatCompatSeverity};
use crate::wav_format::WavFormatIssue;

const RATES_WAV: [u32; 3] = [44_100, 48_000, 96_000];
const RATES_HIRES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
const RATES_AAC: [u32; 6] = [16_000, 22_050, 24_000, 32_000, 44_100, 48_000];
const BITS_LOSSLESS: [u8; 2] = [16, 24];

impl FormatCompat {
    fn ok() -> Self {
        Self {
            severity: FormatCompatSeverity::Ok,
            warning: None,
        }
    }

    fn warn(message: &str) -> Self {
        Self {
            severity: FormatCompatSeverity::Warn,
            warning: Some(message.to_string()),
        }
    }

    fn autofix(message: &str) -> Self {
        Self {
            severity: FormatCompatSeverity::Autofix,
            warning: Some(message.to_string()),
        }
    }
}

/// Verdict for a track's format. `wav_extensible_kind` is the stored
/// `WavFormatIssue::as_db_str` value (`Track.wav_extensible_kind`), or `None`
/// for non-WAV / clean WAV files.
pub(crate) fn compute_format_compat(
    format_ext: Option<&str>,
    sample_rate_hz: Option<u32>,
    bit_depth: Option<u8>,
    bitrate_kbps: Option<u32>,
    wav_extensible_kind: Option<&str>,
) -> FormatCompat {
    let ext = format_ext.unwrap_or_default().trim().to_ascii_lowercase();

    if ext == "wav"
        && let Some(issue) = wav_extensible_kind.and_then(WavFormatIssue::from_db_str)
    {
        return if issue.is_export_autofixable() {
            FormatCompat::autofix(
                "Uses an extended WAV header (WAVE_FORMAT_EXTENSIBLE) that some CDJs reject. \
                 Will be automatically converted to standard PCM on export.",
            )
        } else {
            FormatCompat::warn(
                "Uses an extended WAV header with a non-standard subformat - cannot be safely \
                 converted and may not play on CDJ hardware.",
            )
        };
    }

    match ext.as_str() {
        "wav" => match (sample_rate_hz, bit_depth) {
            (Some(sr), Some(bd)) if !RATES_WAV.contains(&sr) || !BITS_LOSSLESS.contains(&bd) => {
                FormatCompat::warn("Outside WAV support (16/24-bit, 44.1/48/96 kHz)")
            }
            _ => FormatCompat::ok(),
        },
        "mp3" => {
            let Some(sr) = sample_rate_hz else {
                return FormatCompat::ok();
            };
            if !RATES_AAC.contains(&sr) {
                return FormatCompat::warn("Outside MP3 sample-rate support");
            }
            // A CBR MPEG-1/2 Layer III stream can't exceed its tier's ceiling
            // (320 kbps at 32/44.1/48 kHz, 160 at the lower rates), so a higher
            // *average* bitrate just means the file is VBR -- which CDJs play
            // fine. Only an implausibly low bitrate is worth flagging.
            match bitrate_kbps {
                Some(br) if br < mp3_bitrate_floor(sr) => {
                    FormatCompat::warn("Outside MP3 bitrate support")
                }
                _ => FormatCompat::ok(),
            }
        }
        "aac" | "mp4" => match (sample_rate_hz, bitrate_kbps) {
            (Some(sr), Some(br)) if !RATES_AAC.contains(&sr) || !(16..=320).contains(&br) => {
                FormatCompat::warn("Outside AAC support (16-320 kbps, 16/22.05/24/32/44.1/48 kHz)")
            }
            _ => FormatCompat::ok(),
        },
        "m4a" => {
            let Some(sr) = sample_rate_hz else {
                return FormatCompat::ok();
            };
            if bitrate_kbps.is_none() && bit_depth.is_none() {
                return FormatCompat::ok();
            }
            // .m4a can be AAC or ALAC -- pass if either profile matches.
            let aac_ok =
                RATES_AAC.contains(&sr) && bitrate_kbps.is_some_and(|br| (16..=320).contains(&br));
            let alac_ok = RATES_HIRES.contains(&sr)
                && bit_depth.is_some_and(|bd| BITS_LOSSLESS.contains(&bd));
            if aac_ok || alac_ok {
                FormatCompat::ok()
            } else {
                FormatCompat::warn("Outside AAC/ALAC support for .m4a")
            }
        }
        "flac" | "fla" => match (sample_rate_hz, bit_depth) {
            (Some(sr), Some(bd)) if !RATES_HIRES.contains(&sr) || !BITS_LOSSLESS.contains(&bd) => {
                FormatCompat::warn("Outside FLAC support (16/24-bit, 44.1/48/88.2/96 kHz)")
            }
            _ => FormatCompat::ok(),
        },
        "aif" | "aiff" => match (sample_rate_hz, bit_depth) {
            (Some(sr), Some(bd)) if !RATES_HIRES.contains(&sr) || !BITS_LOSSLESS.contains(&bd) => {
                FormatCompat::warn("Outside AIFF support (16/24-bit, 44.1/48/88.2/96 kHz)")
            }
            _ => FormatCompat::ok(),
        },
        _ => FormatCompat::warn("Unlisted format"),
    }
}

fn mp3_bitrate_floor(sample_rate_hz: u32) -> u32 {
    if [16_000, 22_050, 24_000].contains(&sample_rate_hz) {
        8
    } else {
        32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_in_spec_formats_are_ok() {
        assert_eq!(
            compute_format_compat(Some("wav"), Some(44_100), Some(16), None, None).severity,
            FormatCompatSeverity::Ok
        );
        assert_eq!(
            compute_format_compat(Some("flac"), Some(88_200), Some(24), None, None).severity,
            FormatCompatSeverity::Ok
        );
        assert_eq!(
            compute_format_compat(Some("mp3"), Some(44_100), Some(0), Some(320), None).severity,
            FormatCompatSeverity::Ok
        );
    }

    #[test]
    fn missing_technical_fields_are_not_flagged() {
        assert_eq!(
            compute_format_compat(Some("wav"), None, None, None, None).severity,
            FormatCompatSeverity::Ok
        );
        assert_eq!(
            compute_format_compat(Some("mp3"), None, None, None, None).severity,
            FormatCompatSeverity::Ok
        );
    }

    #[test]
    fn out_of_spec_lossless_warns() {
        assert_eq!(
            compute_format_compat(Some("wav"), Some(192_000), Some(24), None, None).severity,
            FormatCompatSeverity::Warn
        );
        assert_eq!(
            compute_format_compat(Some("flac"), Some(44_100), Some(32), None, None).severity,
            FormatCompatSeverity::Warn
        );
    }

    #[test]
    fn vbr_mp3_above_320_is_allowed() {
        // A reported average bitrate over the CBR ceiling can only be VBR.
        assert_eq!(
            compute_format_compat(Some("mp3"), Some(44_100), None, Some(341), None).severity,
            FormatCompatSeverity::Ok
        );
        // ...but a nonsensically low bitrate still flags.
        assert_eq!(
            compute_format_compat(Some("mp3"), Some(44_100), None, Some(4), None).severity,
            FormatCompatSeverity::Warn
        );
    }

    #[test]
    fn wav_extensible_pcm_is_autofix_other_is_warn() {
        let pcm = compute_format_compat(Some("wav"), Some(44_100), Some(24), None, Some("extensible_pcm"));
        assert_eq!(pcm.severity, FormatCompatSeverity::Autofix);
        assert!(pcm.warning.unwrap().contains("converted to standard PCM"));

        let other = compute_format_compat(
            Some("wav"),
            Some(44_100),
            Some(24),
            None,
            Some("extensible_other"),
        );
        assert_eq!(other.severity, FormatCompatSeverity::Warn);
    }

    #[test]
    fn unrecognised_extension_warns() {
        assert_eq!(
            compute_format_compat(Some("ogg"), None, None, None, None).severity,
            FormatCompatSeverity::Warn
        );
        assert_eq!(
            compute_format_compat(Some(""), None, None, None, None).severity,
            FormatCompatSeverity::Warn
        );
    }

    #[test]
    fn m4a_passes_on_either_aac_or_alac_profile() {
        // ALAC profile
        assert_eq!(
            compute_format_compat(Some("m4a"), Some(96_000), Some(24), None, None).severity,
            FormatCompatSeverity::Ok
        );
        // AAC profile
        assert_eq!(
            compute_format_compat(Some("m4a"), Some(44_100), None, Some(256), None).severity,
            FormatCompatSeverity::Ok
        );
        // neither
        assert_eq!(
            compute_format_compat(Some("m4a"), Some(192_000), Some(32), Some(700), None).severity,
            FormatCompatSeverity::Warn
        );
    }
}
