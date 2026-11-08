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
//! `format_ext`, so its verdict is whether players read the extension and
//! whether the stick's USB audio formats allow it; see
//! [`compute_usb_format_compat`]). Missing
//! technical fields mean "can't judge" -> `Ok`, matching the old frontend
//! behaviour where a `null` sample rate / bitrate short-circuited the check.

use std::path::Path;

use crate::audio_format::{
    AudioFormat, UsbAudioFormat, UsbAudioFormatOfExtension, is_alac_file, is_player_extension,
};
use crate::i18n::tr;
use crate::models::{FormatCompat, FormatCompatSeverity};
use crate::service::usb_identity::UsbAudioFormats;
use crate::wav_format::WavFormatIssue;

const RATES_WAV: [u32; 3] = [44_100, 48_000, 96_000];
const RATES_HIRES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
const RATES_AAC: [u32; 6] = [16_000, 22_050, 24_000, 32_000, 44_100, 48_000];
const BITS_LOSSLESS: [u8; 2] = [16, 24];

impl FormatCompat {
    fn ok() -> Self {
        Self::default()
    }

    fn warn(message: &str) -> Self {
        Self {
            severity: FormatCompatSeverity::Warn,
            warning: Some(message.to_string()),
            ..Self::default()
        }
    }

    fn autofix(message: &str) -> Self {
        Self {
            severity: FormatCompatSeverity::Autofix,
            warning: Some(message.to_string()),
            ..Self::default()
        }
    }

    /// Fills in the FMT badge and its tooltip: `details` (see
    /// [`format_details`]), then the warning.
    fn with_badge(mut self, format_ext: Option<&str>, details: &[String]) -> Self {
        self.badge = format_badge(format_ext);
        let details = (!details.is_empty()).then(|| details.join(" · "));
        self.tooltip = match (details, &self.warning) {
            (Some(details), Some(warning)) => Some(format!("{details} — {warning}")),
            (details, warning) => details.or_else(|| warning.clone()),
        };
        self
    }
}

/// The FMT badge's text for a file: its extension in capitals, or "Unknown".
pub(crate) fn format_badge(format_ext: Option<&str>) -> String {
    format_ext
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map_or_else(
            || tr::track_table_format_unknown().render(),
            str::to_ascii_uppercase,
        )
}

/// What the FMT column says about a file besides its format: `["44.1 kHz",
/// "16-bit"]`, or the bitrate when there's no bit depth (`["320 kbps"]`).
pub(crate) fn format_details(
    sample_rate_hz: Option<u32>,
    bit_depth: Option<u8>,
    bitrate_kbps: Option<u32>,
) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(rate) = sample_rate_hz.filter(|&rate| rate > 0) {
        let khz = f64::from(rate) / 1000.0;
        let khz = if khz.fract() == 0.0 {
            format!("{khz:.0}")
        } else {
            format!("{khz:.1}")
        };
        parts.push(tr::track_table_format_sample_rate(khz).render());
    }
    if let Some(depth) = bit_depth.filter(|&depth| depth > 0) {
        parts.push(tr::track_table_format_bit_depth(depth).render());
    } else if let Some(bitrate) = bitrate_kbps.filter(|&rate| rate > 0) {
        parts.push(tr::track_table_format_bitrate(bitrate).render());
    }
    parts
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
    format_compat_verdict(
        format_ext,
        sample_rate_hz,
        bit_depth,
        bitrate_kbps,
        wav_extensible_kind,
    )
    .with_badge(
        format_ext,
        &format_details(sample_rate_hz, bit_depth, bitrate_kbps),
    )
}

fn format_compat_verdict(
    format_ext: Option<&str>,
    sample_rate_hz: Option<u32>,
    bit_depth: Option<u8>,
    bitrate_kbps: Option<u32>,
    wav_extensible_kind: Option<&str>,
) -> FormatCompat {
    let format = format_ext.and_then(AudioFormat::from_extension);

    if format == Some(AudioFormat::Wav)
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

    let Some(format) = format else {
        return FormatCompat::warn("Unlisted format");
    };
    let verdict = format_verdict(format, sample_rate_hz, bit_depth, bitrate_kbps);
    let is_wave = format_ext.is_some_and(|ext| ext.trim().eq_ignore_ascii_case("wave"));
    if is_wave && verdict.severity == FormatCompatSeverity::Ok {
        return FormatCompat::autofix(
            "Uses the .wave extension, which CDJs don't recognise. Will be exported as .wav.",
        );
    }
    verdict
}

/// A file already on a USB stick: no export will rename or convert it, and
/// USB rows carry no sample rate / bit depth / bitrate, so the verdict is
/// whether players read its extension, and whether its format is ticked in
/// the stick's USB audio formats. `file_path` is the file on the stick; it is
/// only read for an `.m4a` when the stick allows one of ALAC and AAC but not
/// the other.
pub(crate) fn compute_usb_format_compat(
    format_ext: Option<&str>,
    file_path: &Path,
    formats: &UsbAudioFormats,
) -> FormatCompat {
    usb_format_compat_verdict(format_ext, file_path, formats).with_badge(format_ext, &[])
}

fn usb_format_compat_verdict(
    format_ext: Option<&str>,
    file_path: &Path,
    formats: &UsbAudioFormats,
) -> FormatCompat {
    let Some(ext) = format_ext.filter(|ext| is_player_extension(ext)) else {
        return FormatCompat::warn("Unlisted format");
    };
    let format = match UsbAudioFormat::of_extension(ext) {
        UsbAudioFormatOfExtension::Known(format) => format,
        UsbAudioFormatOfExtension::AlacOrAac => {
            if formats.allows(UsbAudioFormat::Alac) == formats.allows(UsbAudioFormat::Aac) {
                UsbAudioFormat::Aac
            } else if is_alac_file(file_path) {
                UsbAudioFormat::Alac
            } else {
                UsbAudioFormat::Aac
            }
        }
        UsbAudioFormatOfExtension::None => return FormatCompat::ok(),
    };
    if formats.allows(format) {
        FormatCompat::ok()
    } else {
        FormatCompat::warn(&format!(
            "{} is not allowed on this USB (USB audio formats)",
            format.label()
        ))
    }
}

fn format_verdict(
    format: AudioFormat,
    sample_rate_hz: Option<u32>,
    bit_depth: Option<u8>,
    bitrate_kbps: Option<u32>,
) -> FormatCompat {
    match format {
        AudioFormat::Wav => match (sample_rate_hz, bit_depth) {
            (Some(sr), Some(bd)) if !RATES_WAV.contains(&sr) || !BITS_LOSSLESS.contains(&bd) => {
                FormatCompat::warn("Outside WAV support (16/24-bit, 44.1/48/96 kHz)")
            }
            _ => FormatCompat::ok(),
        },
        AudioFormat::Mp3 => {
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
        AudioFormat::Aac | AudioFormat::Mp4 => match (sample_rate_hz, bitrate_kbps) {
            (Some(sr), Some(br)) if !RATES_AAC.contains(&sr) || !(16..=320).contains(&br) => {
                FormatCompat::warn("Outside AAC support (16-320 kbps, 16/22.05/24/32/44.1/48 kHz)")
            }
            _ => FormatCompat::ok(),
        },
        AudioFormat::M4a => {
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
        AudioFormat::Flac => match (sample_rate_hz, bit_depth) {
            (Some(sr), Some(bd)) if !RATES_HIRES.contains(&sr) || !BITS_LOSSLESS.contains(&bd) => {
                FormatCompat::warn("Outside FLAC support (16/24-bit, 44.1/48/88.2/96 kHz)")
            }
            _ => FormatCompat::ok(),
        },
        AudioFormat::Aiff => match (sample_rate_hz, bit_depth) {
            (Some(sr), Some(bd)) if !RATES_HIRES.contains(&sr) || !BITS_LOSSLESS.contains(&bd) => {
                FormatCompat::warn("Outside AIFF support (16/24-bit, 44.1/48/88.2/96 kHz)")
            }
            _ => FormatCompat::ok(),
        },
        // Export leaves these off the USB (`split_off_unplayable_tracks`).
        AudioFormat::M4p | AudioFormat::Ogg | AudioFormat::Opus => {
            FormatCompat::warn("Players don't play this format. Not exported to USB.")
        }
        AudioFormat::Alac => FormatCompat::warn("Unlisted format"),
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

    // The FMT cell shows these as they are: the badge, and a tooltip of the
    // technical details followed by the warning.
    #[test]
    fn badge_and_tooltip_describe_the_file() {
        let plain = compute_format_compat(Some(" wav "), Some(44_100), Some(16), None, None);
        assert_eq!(plain.badge, "WAV");
        assert_eq!(plain.tooltip.as_deref(), Some("44.1 kHz · 16-bit"));

        let mp3 = compute_format_compat(Some("mp3"), Some(48_000), None, Some(320), None);
        assert_eq!(mp3.tooltip.as_deref(), Some("48 kHz · 320 kbps"));

        let warned = compute_format_compat(Some("flac"), Some(192_000), Some(24), None, None);
        assert_eq!(
            warned.tooltip.as_deref(),
            Some("192 kHz · 24-bit — Outside FLAC support (16/24-bit, 44.1/48/88.2/96 kHz)")
        );

        let bare = compute_format_compat(Some("wav"), None, None, None, None);
        assert_eq!(bare.tooltip, None);

        let unknown = compute_format_compat(None, None, None, None, None);
        assert_eq!(unknown.badge, "Unknown");
        assert_eq!(unknown.tooltip.as_deref(), Some("Unlisted format"));

        let usb =
            compute_usb_format_compat(Some("mp3"), Path::new("a.mp3"), &UsbAudioFormats::default());
        assert_eq!((usb.badge.as_str(), usb.tooltip), ("MP3", None));
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
    fn wave_extension_is_autofix_unless_the_audio_itself_is_out_of_spec() {
        let wave = compute_format_compat(Some("WAVE"), Some(44_100), Some(16), None, None);
        assert_eq!(wave.severity, FormatCompatSeverity::Autofix);
        assert!(wave.warning.unwrap().contains("exported as .wav"));
        assert_eq!(
            compute_format_compat(Some("wave"), Some(192_000), Some(24), None, None).severity,
            FormatCompatSeverity::Warn
        );
    }

    #[test]
    fn usb_rows_warn_only_for_extensions_players_dont_read() {
        for ext in [
            "mp3", "M4A", "aac", "mp4", "wav", "aif", "aiff", "flac", "fla",
        ] {
            assert_eq!(
                compute_usb_format_compat(Some(ext), Path::new(""), &UsbAudioFormats::default())
                    .severity,
                FormatCompatSeverity::Ok,
                "{ext}"
            );
        }
        for ext in [Some("wave"), Some("ogg"), Some("opus"), Some("m4p"), None] {
            assert_eq!(
                compute_usb_format_compat(ext, Path::new(""), &UsbAudioFormats::default()).severity,
                FormatCompatSeverity::Warn,
                "{ext:?}"
            );
        }
    }

    fn formats_without(unticked: &[&str]) -> UsbAudioFormats {
        let items: Vec<(String, bool)> = UsbAudioFormat::DEFAULT_ORDER
            .into_iter()
            .map(|format| (format.id().to_string(), !unticked.contains(&format.id())))
            .collect();
        UsbAudioFormats::from_request(&items).unwrap()
    }

    #[test]
    fn usb_rows_warn_for_formats_the_stick_does_not_allow() {
        let no_flac = formats_without(&["flac"]);
        let flac = compute_usb_format_compat(Some("fla"), Path::new("a.fla"), &no_flac);
        assert_eq!(flac.severity, FormatCompatSeverity::Warn);
        assert_eq!(
            flac.warning.as_deref(),
            Some("FLAC is not allowed on this USB (USB audio formats)")
        );
        assert_eq!(
            compute_usb_format_compat(Some("mp3"), Path::new("a.mp3"), &no_flac).severity,
            FormatCompatSeverity::Ok
        );
    }

    #[test]
    fn usb_m4a_is_judged_by_the_codec_inside() {
        let alac = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/formats/track_format_alac.m4a");
        let verdict = |path: &Path, unticked: &[&str]| {
            compute_usb_format_compat(Some("m4a"), path, &formats_without(unticked)).severity
        };
        assert_eq!(verdict(&alac, &["alac"]), FormatCompatSeverity::Warn);
        assert_eq!(verdict(&alac, &["aac"]), FormatCompatSeverity::Ok);
        // Unreadable: counts as AAC.
        let missing = Path::new("missing.m4a");
        assert_eq!(verdict(missing, &["aac"]), FormatCompatSeverity::Warn);
        assert_eq!(verdict(missing, &["alac"]), FormatCompatSeverity::Ok);
        assert_eq!(
            verdict(missing, &["alac", "aac"]),
            FormatCompatSeverity::Warn
        );
    }

    #[test]
    fn wav_extensible_pcm_is_autofix_other_is_warn() {
        let pcm = compute_format_compat(
            Some("wav"),
            Some(44_100),
            Some(24),
            None,
            Some("extensible_pcm"),
        );
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
