//! Search and column sort shared by every paginated track list (Library,
//! playlists, USB playlists and history).
//!
//! Both run server-side over the whole list before paging, so they span rows
//! the frontend has not loaded yet. The frontend only sends the search text,
//! column key and direction.

use std::cmp::Ordering;

use crate::models::{Track, UsbTrack};

use super::key_notation;

/// The fields a track list searches and sorts by.
pub(crate) trait ListTrack {
    fn title(&self) -> &str;
    fn artist(&self) -> &str;
    fn album(&self) -> Option<&str>;
    fn year(&self) -> Option<u32>;
    fn genre(&self) -> Option<&str>;
    fn format_ext(&self) -> Option<&str>;
    fn bpm(&self) -> Option<f64>;
    fn duration_ms(&self) -> Option<u64>;
    fn key(&self) -> Option<&str>;
}

macro_rules! impl_list_track {
    ($ty:ty) => {
        impl ListTrack for $ty {
            fn title(&self) -> &str {
                &self.title
            }
            fn artist(&self) -> &str {
                &self.artist
            }
            fn album(&self) -> Option<&str> {
                self.album.as_deref()
            }
            fn year(&self) -> Option<u32> {
                self.year
            }
            fn genre(&self) -> Option<&str> {
                self.genre.as_deref()
            }
            fn format_ext(&self) -> Option<&str> {
                self.format_ext.as_deref()
            }
            fn bpm(&self) -> Option<f64> {
                self.bpm
            }
            fn duration_ms(&self) -> Option<u64> {
                self.duration_ms
            }
            fn key(&self) -> Option<&str> {
                self.key.as_deref()
            }
        }
    };
}

impl_list_track!(Track);
impl_list_track!(UsbTrack);

/// A search over title, artist, album, genre and year. Each word of the text
/// must appear, case-insensitively, in one of those fields, so a query may mix
/// them in any order ("techno 2009", "Artist Title"). Blank text matches
/// every track.
#[derive(Debug, Clone)]
pub(crate) struct TrackQuery {
    lowered: String,
}

impl TrackQuery {
    pub(crate) fn new(text: &str) -> Self {
        Self {
            lowered: text.trim().to_lowercase(),
        }
    }

    /// Normalized search text; part of the page-cursor signature.
    pub(crate) fn as_str(&self) -> &str {
        &self.lowered
    }

    pub(crate) fn matches<T: ListTrack>(&self, track: &T) -> bool {
        if self.lowered.is_empty() {
            return true;
        }
        let year = track.year().map(|y| y.to_string()).unwrap_or_default();
        let fields = [
            track.title(),
            track.artist(),
            track.album().unwrap_or_default(),
            track.genre().unwrap_or_default(),
            &year,
        ]
        .map(str::to_lowercase);
        self.lowered
            .split_whitespace()
            .all(|word| fields.iter().any(|field| field.contains(word)))
    }

    pub(crate) fn retain<T: ListTrack>(&self, items: &mut Vec<T>) {
        if !self.lowered.is_empty() {
            items.retain(|track| self.matches(track));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    Title,
    Artist,
    Album,
    Year,
    Format,
    Bpm,
    DurationMs,
    Key,
}

impl SortColumn {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "title" => Self::Title,
            "artist" => Self::Artist,
            "album" => Self::Album,
            "year" => Self::Year,
            "format" => Self::Format,
            "bpm" => Self::Bpm,
            "durationMs" => Self::DurationMs,
            "key" => Self::Key,
            _ => return None,
        })
    }

    /// Strings compare case-insensitively, missing year/bpm/duration count as 0,
    /// `artist` is tie-broken by title, and keys follow the Camelot wheel with
    /// unknown keys last.
    fn compare<T: ListTrack>(self, a: &T, b: &T) -> Ordering {
        let ci = |value: &str| value.to_lowercase();
        match self {
            Self::Title => ci(a.title()).cmp(&ci(b.title())),
            Self::Artist => ci(a.artist())
                .cmp(&ci(b.artist()))
                .then_with(|| ci(a.title()).cmp(&ci(b.title()))),
            Self::Album => {
                ci(a.album().unwrap_or_default()).cmp(&ci(b.album().unwrap_or_default()))
            }
            Self::Year => a.year().unwrap_or(0).cmp(&b.year().unwrap_or(0)),
            Self::Format => {
                ci(a.format_ext().unwrap_or_default()).cmp(&ci(b.format_ext().unwrap_or_default()))
            }
            Self::Bpm => a
                .bpm()
                .unwrap_or(0.0)
                .partial_cmp(&b.bpm().unwrap_or(0.0))
                .unwrap_or(Ordering::Equal),
            Self::DurationMs => a
                .duration_ms()
                .unwrap_or(0)
                .cmp(&b.duration_ms().unwrap_or(0)),
            Self::Key => key_notation::compare_keys(a.key(), b.key()),
        }
    }
}

/// A requested column sort: `sort_by` is one of `title` | `artist` | `album` |
/// `year` | `format` | `bpm` | `durationMs` | `key`, `sort_dir` is `asc` (default) or
/// `desc`. An absent or unknown column leaves the caller's natural order
/// (file path, playlist position or PDB entry order) untouched.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TrackSort<'a> {
    /// Trimmed request value; part of the page-cursor signature.
    pub(crate) by: &'a str,
    /// Trimmed request value; part of the page-cursor signature.
    pub(crate) dir: &'a str,
}

impl<'a> TrackSort<'a> {
    pub(crate) fn new(sort_by: Option<&'a str>, sort_dir: Option<&'a str>) -> Self {
        Self {
            by: sort_by.map(str::trim).unwrap_or_default(),
            dir: sort_dir.map(str::trim).unwrap_or_default(),
        }
    }

    /// Stable in-place sort, so rows with equal keys keep the caller's order.
    pub(crate) fn apply<T: ListTrack>(&self, items: &mut [T]) {
        let Some(column) = SortColumn::parse(self.by) else {
            return;
        };
        let desc = self.dir == "desc";
        items.sort_by(|a, b| {
            let ord = column.compare(a, b);
            if desc { ord.reverse() } else { ord }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, bpm: Option<f64>, key: Option<&str>) -> UsbTrack {
        UsbTrack {
            title: title.to_string(),
            artist: artist.to_string(),
            bpm,
            key: key.map(str::to_string),
            ..UsbTrack::default()
        }
    }

    fn titles(items: &[UsbTrack]) -> Vec<&str> {
        items.iter().map(|t| t.title.as_str()).collect()
    }

    #[test]
    fn query_matches_title_artist_album_case_insensitively_and_across_fields() {
        let mut with_album = track("Night Drive", "Example Artist", None, None);
        with_album.album = Some("Demo Album".into());
        with_album.genre = Some("Deep Techno".into());
        with_album.year = Some(2009);
        let mut items = vec![with_album, track("Other", "Someone Else", None, None)];
        for text in [
            "night",
            " EXAMPLE ",
            "demo album",
            "drive example",
            "techno",
            "2009",
            "techno 2009 night",
            "artist drive",
        ] {
            assert!(TrackQuery::new(text).matches(&items[0]), "{text}");
            assert!(!TrackQuery::new(text).matches(&items[1]), "{text}");
        }
        // Every word has to match somewhere.
        assert!(!TrackQuery::new("techno 1994").matches(&items[0]));
        TrackQuery::new("  ").retain(&mut items);
        assert_eq!(items.len(), 2);
        TrackQuery::new("someone").retain(&mut items);
        assert_eq!(titles(&items), ["Other"]);
        assert_eq!(TrackQuery::new(" Mixed Case ").as_str(), "mixed case");
    }

    #[test]
    fn absent_or_unknown_column_keeps_natural_order() {
        let original = vec![
            track("b", "", None, None),
            track("a", "", None, None),
            track("c", "", None, None),
        ];
        for sort in [
            TrackSort::new(None, Some("desc")),
            TrackSort::new(Some("  "), None),
            TrackSort::new(Some("rating"), Some("asc")),
        ] {
            let mut items = original.clone();
            sort.apply(&mut items);
            assert_eq!(titles(&items), ["b", "a", "c"]);
        }
    }

    #[test]
    fn title_is_case_insensitive_and_desc_reverses() {
        let mut items = vec![
            track("beta", "", None, None),
            track("Alpha", "", None, None),
            track("gamma", "", None, None),
        ];
        TrackSort::new(Some(" title "), None).apply(&mut items);
        assert_eq!(titles(&items), ["Alpha", "beta", "gamma"]);
        TrackSort::new(Some("title"), Some(" desc ")).apply(&mut items);
        assert_eq!(titles(&items), ["gamma", "beta", "Alpha"]);
    }

    #[test]
    fn artist_ties_break_by_title() {
        let mut items = vec![
            track("Second", "Example Artist", None, None),
            track("Only", "Another Artist", None, None),
            track("first", "example artist", None, None),
        ];
        TrackSort::new(Some("artist"), None).apply(&mut items);
        assert_eq!(titles(&items), ["Only", "first", "Second"]);
    }

    #[test]
    fn missing_bpm_sorts_as_zero() {
        let mut items = vec![
            track("fast", "", Some(174.0), None),
            track("unknown", "", None, None),
            track("slow", "", Some(90.0), None),
        ];
        TrackSort::new(Some("bpm"), None).apply(&mut items);
        assert_eq!(titles(&items), ["unknown", "slow", "fast"]);
    }

    #[test]
    fn missing_year_sorts_as_zero() {
        let with_year = |title: &str, year: Option<u32>| UsbTrack {
            year,
            ..track(title, "", None, None)
        };
        let mut items = vec![
            with_year("newer", Some(2021)),
            with_year("unknown", None),
            with_year("older", Some(1994)),
        ];
        TrackSort::new(Some("year"), None).apply(&mut items);
        assert_eq!(titles(&items), ["unknown", "older", "newer"]);
    }

    #[test]
    fn key_follows_the_wheel_with_unknown_last() {
        let mut items = vec![
            track("none", "", None, None),
            track("2A", "", None, Some("2A")),
            track("1A", "", None, Some("1A")),
        ];
        TrackSort::new(Some("key"), None).apply(&mut items);
        assert_eq!(titles(&items), ["1A", "2A", "none"]);
    }
}
