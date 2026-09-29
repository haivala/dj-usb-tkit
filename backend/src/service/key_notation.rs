//! Display-only musical key notation (Classic `Am` vs Camelot `8A`).
//!
//! Stored and exported keys are never rewritten: the PDB/eDB key tables keep
//! whatever the track carries (normally classic notation, which every CDJ/XDJ
//! understands; newer players convert it on-device). This module only derives
//! the label, colour group and sort rank the frontend shows.

use rusqlite::OptionalExtension;
use rusqlite::params;

use super::SETTING_UI_KEY_NOTATION;
use crate::error::BackendResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyNotation {
    #[default]
    Classic,
    Camelot,
}

impl KeyNotation {
    /// Parse the persisted setting value; anything unknown is Classic.
    pub fn from_setting(value: &str) -> Self {
        match value.trim() {
            "camelot" => Self::Camelot,
            _ => Self::Classic,
        }
    }
}

/// Camelot wheel, index = wheel number − 1: (minor "A" spellings, major "B" spellings).
/// Enharmonic spellings are listed so flat output (essentia, eDB) maps too.
const WHEEL: [(&[&str], &[&str]); 12] = [
    (&["abm", "g#m"], &["b", "cb"]),
    (&["ebm", "d#m"], &["f#", "gb"]),
    (&["bbm", "a#m"], &["db", "c#"]),
    (&["fm"], &["ab", "g#"]),
    (&["cm"], &["eb", "d#"]),
    (&["gm"], &["bb", "a#"]),
    (&["dm"], &["f"]),
    (&["am"], &["c"]),
    (&["em"], &["g"]),
    (&["bm"], &["d"]),
    (&["f#m", "gbm"], &["a"]),
    (&["c#m", "dbm"], &["e"]),
];

/// Position on the Camelot wheel as `(number 1..=12, minor)`. Accepts classic
/// sharp or flat spellings (`F#m`, `Ebm`, `Db`, "A minor", "C major") and
/// strings that are already Camelot (`8A`, `12b`). `None` for anything else.
pub fn camelot_position(key: &str) -> Option<(u8, bool)> {
    let mut norm: String = key
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
        .replace('♯', "#")
        .replace('♭', "b");
    if norm.is_empty() {
        return None;
    }

    if let Some(letter) = norm.chars().last().filter(|c| *c == 'a' || *c == 'b') {
        let digits = &norm[..norm.len() - 1];
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            let number: u8 = digits.parse().ok()?;
            return (1..=12)
                .contains(&number)
                .then_some((number, letter == 'a'));
        }
    }

    for (suffix, replacement) in [("minor", "m"), ("min", "m"), ("major", ""), ("maj", "")] {
        if let Some(stripped) = norm.strip_suffix(suffix) {
            norm = format!("{stripped}{replacement}");
            break;
        }
    }

    WHEEL
        .iter()
        .enumerate()
        .find_map(|(idx, (minors, majors))| {
            let number = idx as u8 + 1;
            if minors.contains(&norm.as_str()) {
                Some((number, true))
            } else if majors.contains(&norm.as_str()) {
                Some((number, false))
            } else {
                None
            }
        })
}

/// The label the UI shows for `key`. Classic returns the stored string as-is;
/// Camelot converts when the key is recognised and otherwise falls back to
/// the stored string.
pub fn display_key(key: &str, notation: KeyNotation) -> String {
    match notation {
        KeyNotation::Classic => key.to_string(),
        KeyNotation::Camelot => match camelot_position(key) {
            Some((number, minor)) => format!("{number}{}", if minor { 'A' } else { 'B' }),
            None => key.to_string(),
        },
    }
}

/// Colour group 0..=11 (Camelot number − 1) for the key pill's `key-pill--hN`
/// class. Independent of the display notation so a key keeps its colour when
/// the user switches notation.
pub fn key_color_index(key: &str) -> Option<u8> {
    camelot_position(key).map(|(number, _)| number - 1)
}

/// Wheel order for sorting: 1A, 1B, 2A, … 12B. `None` for unrecognised keys.
pub fn key_sort_rank(key: &str) -> Option<u16> {
    camelot_position(key).map(|(number, minor)| u16::from(number) * 2 + u16::from(!minor))
}

/// Display label + colour group for an optional stored key, as sent on the
/// wire (`keyDisplay`, `keyColor`). Blank keys give `(None, None)`.
pub fn key_display_fields(
    key: Option<&str>,
    notation: KeyNotation,
) -> (Option<String>, Option<u8>) {
    match key.map(str::trim).filter(|key| !key.is_empty()) {
        Some(key) => (Some(display_key(key, notation)), key_color_index(key)),
        None => (None, None),
    }
}

/// Order two optional keys by wheel position; recognised keys sort before
/// unrecognised ones, which fall back to a case-insensitive string compare.
pub fn compare_keys(a: Option<&str>, b: Option<&str>) -> std::cmp::Ordering {
    let rank = |key: Option<&str>| key.and_then(key_sort_rank).unwrap_or(u16::MAX);
    rank(a).cmp(&rank(b)).then_with(|| {
        a.unwrap_or_default()
            .to_lowercase()
            .cmp(&b.unwrap_or_default().to_lowercase())
    })
}

/// The user's key notation, read from `app_settings`. Missing → Classic.
pub(crate) fn key_notation_setting(conn: &rusqlite::Connection) -> BackendResult<KeyNotation> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_KEY_NOTATION],
            |row| row.get(0),
        )
        .optional()?;
    Ok(raw
        .map(|value| KeyNotation::from_setting(&value))
        .unwrap_or_default())
}

impl super::BackendService {
    /// [`key_notation_setting`] on a fresh connection.
    pub(crate) fn key_notation(&self) -> BackendResult<KeyNotation> {
        let conn = self.db.connect()?;
        key_notation_setting(&conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::cues::KEY_OPTIONS;

    #[test]
    fn every_key_option_maps_to_a_unique_wheel_position() {
        let mut seen = std::collections::HashSet::new();
        for key in KEY_OPTIONS {
            let pos = camelot_position(key).unwrap_or_else(|| panic!("{key} unmapped"));
            assert_eq!(pos.1, key.ends_with('m'), "{key} minor flag");
            assert!(seen.insert(pos), "{key} duplicates {pos:?}");
        }
        assert_eq!(seen.len(), 24);
    }

    #[test]
    fn classic_to_camelot_reference_points() {
        for (classic, camelot) in [
            ("Am", "8A"),
            ("C", "8B"),
            ("G#m", "1A"),
            ("B", "1B"),
            ("F#", "2B"),
            ("C#m", "12A"),
            ("E", "12B"),
            ("Dm", "7A"),
        ] {
            assert_eq!(
                display_key(classic, KeyNotation::Camelot),
                camelot,
                "{classic}"
            );
        }
    }

    #[test]
    fn flat_and_long_spellings_are_recognised() {
        assert_eq!(camelot_position("Ebm"), Some((2, true)));
        assert_eq!(camelot_position("Db"), Some((3, false)));
        assert_eq!(camelot_position("Gbm"), Some((11, true)));
        assert_eq!(camelot_position(" A minor "), Some((8, true)));
        assert_eq!(camelot_position("C major"), Some((8, false)));
        assert_eq!(camelot_position("Bb"), Some((6, false)));
        assert_eq!(camelot_position("B♭"), Some((6, false)));
    }

    #[test]
    fn camelot_input_passes_through() {
        assert_eq!(camelot_position("8A"), Some((8, true)));
        assert_eq!(camelot_position("12b"), Some((12, false)));
        assert_eq!(display_key("8a", KeyNotation::Camelot), "8A");
        assert_eq!(camelot_position("13A"), None);
        assert_eq!(camelot_position("0B"), None);
    }

    #[test]
    fn classic_notation_keeps_the_stored_string() {
        assert_eq!(display_key("Eb", KeyNotation::Classic), "Eb");
        assert_eq!(display_key("8A", KeyNotation::Classic), "8A");
    }

    #[test]
    fn unrecognised_keys_fall_back_to_raw_label_and_no_colour() {
        assert_eq!(camelot_position("H"), None);
        assert_eq!(camelot_position(""), None);
        assert_eq!(display_key("???", KeyNotation::Camelot), "???");
        assert_eq!(
            key_display_fields(Some("???"), KeyNotation::Camelot),
            (Some("???".into()), None)
        );
        assert_eq!(
            key_display_fields(Some("  "), KeyNotation::Camelot),
            (None, None)
        );
        assert_eq!(key_display_fields(None, KeyNotation::Classic), (None, None));
    }

    #[test]
    fn colour_index_matches_previous_frontend_hue_table() {
        // key_hue.mjs hue per wheel number, bucketed as round(hue / 30) % 12.
        let hues = [0, 30, 55, 85, 120, 155, 180, 200, 230, 260, 290, 325];
        for (idx, hue) in hues.iter().enumerate() {
            let bucket = ((f64::from(*hue) / 30.0).round() as u8) % 12;
            assert_eq!(bucket, idx as u8, "wheel {}", idx + 1);
        }
        assert_eq!(key_color_index("Am"), Some(7));
        assert_eq!(key_color_index("C"), Some(7));
        assert_eq!(key_color_index("B"), Some(0));
    }

    #[test]
    fn sort_follows_the_wheel_with_unknown_last() {
        let mut keys = vec![
            Some("8B"),
            None,
            Some("Am"),
            Some("zzz"),
            Some("G#m"),
            Some("B"),
        ];
        keys.sort_by(|a, b| compare_keys(*a, *b));
        assert_eq!(
            keys,
            vec![
                Some("G#m"),
                Some("B"),
                Some("Am"),
                Some("8B"),
                None,
                Some("zzz")
            ]
        );
    }

    #[test]
    fn setting_parse_defaults_to_classic() {
        assert_eq!(KeyNotation::from_setting("camelot"), KeyNotation::Camelot);
        assert_eq!(KeyNotation::from_setting("classic"), KeyNotation::Classic);
        assert_eq!(KeyNotation::from_setting("bogus"), KeyNotation::Classic);
    }
}
