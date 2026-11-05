//! USB History's Export Tracklist: the start-track labels and the text file.
//! Both read the history from the existing cache, so neither touches the stick.

use std::path::Path;

use super::BackendService;
use super::usb_utils::resolve_usb_root;
use crate::error::{BackendError, BackendResult};
use crate::models::{
    BuildUsbHistoryTracklistData, BuildUsbHistoryTracklistRequest,
    GetUsbHistoryTracklistOptionsData, GetUsbHistoryTracklistOptionsRequest, TracklistTimeMode,
    UsbHistory, UsbTrack,
};

const START_TRACK_LABEL_MAX_CHARS: usize = 64;

/// `Artist - Title`, with the same defaults the track tables show.
fn track_line(track: &UsbTrack) -> String {
    let artist = if track.artist.is_empty() {
        "Unknown Artist"
    } else {
        &track.artist
    };
    let title = if track.title.is_empty() {
        "Unknown Title"
    } else {
        &track.title
    };
    format!("{artist} - {title}")
}

/// "1. Artist - Title", cut to 64 characters. Cuts between characters, never
/// inside one.
fn start_track_labels(tracks: &[UsbTrack]) -> Vec<String> {
    tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let label = format!("{}. {}", index + 1, track_line(track));
            if label.chars().count() <= START_TRACK_LABEL_MAX_CHARS {
                return label;
            }
            let cut = label
                .chars()
                .take(START_TRACK_LABEL_MAX_CHARS - 1)
                .collect::<String>();
            format!("{cut}…")
        })
        .collect()
}

/// `m:ss`, or `h:mm:ss` from an hour on.
fn time_stamp(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// One line per track. The clock starts at the first track, and a track with no
/// known length adds nothing to it.
fn tracklist_text(tracks: &[UsbTrack], time_mode: TracklistTimeMode) -> String {
    let mut clock_ms = 0u64;
    tracks
        .iter()
        .map(|track| {
            let line = track_line(track);
            let stamp = time_stamp(clock_ms);
            clock_ms = clock_ms.saturating_add(track.duration_ms.unwrap_or(0));
            match time_mode {
                TracklistTimeMode::Off => line,
                TracklistTimeMode::Before => format!("{stamp} {line}"),
                TracklistTimeMode::After => format!("{line} - {stamp}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A session name as a `.txt` file name that's safe on every desktop system.
fn tracklist_file_name(name: &str) -> String {
    const UNSAFE: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    // A run of unsafe characters becomes one "-".
    let mut replaced = String::with_capacity(name.len());
    let mut in_unsafe_run = false;
    for c in name.chars() {
        let is_unsafe = UNSAFE.contains(&c);
        if !is_unsafe {
            replaced.push(c);
        } else if !in_unsafe_run {
            replaced.push('-');
        }
        in_unsafe_run = is_unsafe;
    }
    let cleaned = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        "tracklist.txt".to_string()
    } else {
        format!("{cleaned}.txt")
    }
}

impl BackendService {
    fn usb_history_for_tracklist(&self, usb_root: &Path, id: &str) -> BackendResult<UsbHistory> {
        self.usb_histories_resolved(usb_root)?
            .items
            .iter()
            .find(|history| history.id == id)
            .cloned()
            .ok_or_else(|| BackendError::NotFound(format!("USB history not found: {id}")))
    }

    pub fn get_usb_history_tracklist_options(
        &self,
        req: GetUsbHistoryTracklistOptionsRequest,
    ) -> BackendResult<GetUsbHistoryTracklistOptionsData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let history = self.usb_history_for_tracklist(&usb_root, &req.id)?;
        Ok(GetUsbHistoryTracklistOptionsData {
            start_tracks: start_track_labels(&history.tracks),
        })
    }

    pub fn build_usb_history_tracklist(
        &self,
        req: BuildUsbHistoryTracklistRequest,
    ) -> BackendResult<BuildUsbHistoryTracklistData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let history = self.usb_history_for_tracklist(&usb_root, &req.id)?;
        let start_index = req.start_index.min(history.tracks.len().saturating_sub(1));
        let tracks = history.tracks.get(start_index..).unwrap_or_default();
        Ok(BuildUsbHistoryTracklistData {
            suggested_file_name: tracklist_file_name(&history.name),
            contents: tracklist_text(tracks, req.time_mode),
            exported_text: format!("Tracklist exported: {}", history.name),
            cancelled_text: "Tracklist export cancelled".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(artist: &str, title: &str, duration_ms: Option<u64>) -> UsbTrack {
        UsbTrack {
            artist: artist.to_string(),
            title: title.to_string(),
            duration_ms,
            ..Default::default()
        }
    }

    fn two_tracks() -> Vec<UsbTrack> {
        vec![
            track("Artist A", "Title A", Some(180_000)),
            track("Artist B", "Title B", Some(200_000)),
        ]
    }

    #[test]
    fn tracklist_with_time_mode_off_is_plain_artist_title_lines() {
        assert_eq!(
            tracklist_text(&two_tracks(), TracklistTimeMode::Off),
            "Artist A - Title A\nArtist B - Title B"
        );
    }

    #[test]
    fn tracklist_with_time_mode_before_prefixes_cumulative_time() {
        assert_eq!(
            tracklist_text(&two_tracks(), TracklistTimeMode::Before),
            "0:00 Artist A - Title A\n3:00 Artist B - Title B"
        );
    }

    #[test]
    fn tracklist_with_time_mode_after_suffixes_cumulative_time() {
        assert_eq!(
            tracklist_text(&two_tracks(), TracklistTimeMode::After),
            "Artist A - Title A - 0:00\nArtist B - Title B - 3:00"
        );
    }

    #[test]
    fn tracklist_missing_duration_adds_zero_to_the_clock() {
        let tracks = vec![
            track("Artist A", "Title A", None),
            track("Artist B", "Title B", Some(120_000)),
        ];
        assert_eq!(
            tracklist_text(&tracks, TracklistTimeMode::Before),
            "0:00 Artist A - Title A\n0:00 Artist B - Title B"
        );
    }

    #[test]
    fn tracklist_of_no_tracks_is_empty() {
        assert_eq!(tracklist_text(&[], TracklistTimeMode::Before), "");
        assert_eq!(tracklist_text(&[], TracklistTimeMode::Off), "");
    }

    #[test]
    fn tracklist_clock_rolls_over_to_hours() {
        let tracks = vec![
            track("A", "Long", Some(3_725_000)),
            track("B", "Next", Some(1_000)),
        ];
        assert_eq!(
            tracklist_text(&tracks, TracklistTimeMode::Before),
            "0:00 A - Long\n1:02:05 B - Next"
        );
    }

    #[test]
    fn blank_artist_and_title_use_the_table_defaults() {
        let tracks = vec![track("", "", None)];
        assert_eq!(
            tracklist_text(&tracks, TracklistTimeMode::Off),
            "Unknown Artist - Unknown Title"
        );
        assert_eq!(
            start_track_labels(&tracks),
            ["1. Unknown Artist - Unknown Title"]
        );
    }

    #[test]
    fn file_name_is_a_safe_txt_name() {
        assert_eq!(tracklist_file_name("HISTORY 003"), "HISTORY 003.txt");
        assert_eq!(
            tracklist_file_name("Set: A/B \"Live\"?"),
            "Set- A-B -Live-.txt"
        );
        assert_eq!(tracklist_file_name("a//:b"), "a-b.txt");
        assert_eq!(tracklist_file_name("   "), "tracklist.txt");
    }

    #[test]
    fn start_track_labels_are_numbered_and_short_ones_stay_whole() {
        assert_eq!(
            start_track_labels(&two_tracks()),
            ["1. Artist A - Title A", "2. Artist B - Title B"]
        );
    }

    #[test]
    fn long_start_track_label_is_cut_to_64_characters() {
        let label = &start_track_labels(&[track("DJ Four", &"x".repeat(200), None)])[0];
        assert_eq!(label.chars().count(), 64);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn long_finnish_title_is_not_cut_inside_a_character() {
        let label = &start_track_labels(&[track("Äänitaiteilija", &"äöå".repeat(40), None)])[0];
        assert_eq!(label.chars().count(), 64);
        assert!(label.starts_with("1. Äänitaiteilija - äöå"));
        assert!(label.ends_with('…'));
    }
}
