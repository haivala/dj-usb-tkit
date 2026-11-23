//! Text read from the stick's databases: cleaning it for display, undoing
//! double-decoded UTF-8, and the key playlist names are matched by.

pub(crate) fn canonicalize_playlist_name(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
}

pub(crate) fn repair_utf8_mojibake(value: &str) -> String {
    fn suspicious_score(value: &str) -> usize {
        value
            .chars()
            .filter(|c| {
                matches!(*c, 'Ã' | 'Â' | '¤' | '�') || ('\u{0080}'..='\u{009f}').contains(c)
            })
            .count()
    }

    fn latin1_bytes(value: &str) -> Option<Vec<u8>> {
        let mut bytes = Vec::with_capacity(value.len());
        for ch in value.chars() {
            let code = ch as u32;
            if code > 0xff {
                return None;
            }
            bytes.push(code as u8);
        }
        Some(bytes)
    }

    let mut current = value.to_string();
    for _ in 0..2 {
        let Some(bytes) = latin1_bytes(&current) else {
            break;
        };
        let Ok(decoded) = String::from_utf8(bytes) else {
            break;
        };
        if suspicious_score(&decoded) < suspicious_score(&current) {
            current = decoded;
        } else {
            break;
        }
    }
    current
}

pub(crate) fn sanitize_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control()
                && *c != '\u{fffd}'
                && (c.is_ascii_alphanumeric()
                    || c.is_ascii_punctuation()
                    || c.is_ascii_whitespace())
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_playlist_name_preserves_unicode_letters() {
        assert_eq!(canonicalize_playlist_name("星空レコード"), "星空レコード");
        assert_eq!(canonicalize_playlist_name("月影歩"), "月影歩");
        assert_eq!(canonicalize_playlist_name("Night Signal"), "nightsignal");
    }

    #[test]
    fn canonicalize_playlist_name_distinguishes_unicode_artist_names() {
        assert_ne!(
            canonicalize_playlist_name("星空レコード"),
            canonicalize_playlist_name("月影歩")
        );
        assert_ne!(
            canonicalize_playlist_name("月影歩"),
            canonicalize_playlist_name("ひかりの")
        );
    }

    #[test]
    fn repair_utf8_mojibake_recovers_double_decoded_utf8() {
        fn latin1_mojibake(value: &str) -> String {
            value
                .as_bytes()
                .iter()
                .map(|byte| char::from(*byte))
                .collect()
        }

        let original = "/Contents/星空ドライバー feat. ねこまち/Sample Game 31 ORIGINAL SOUNDTRACK/02 - 星空ドライバー feat. ねこまち - あおビー!.flac";
        let mojibake = latin1_mojibake(&latin1_mojibake(original));
        assert_eq!(repair_utf8_mojibake(&mojibake), original);
    }

    #[test]
    fn sanitize_text_removes_control_chars() {
        assert_eq!(sanitize_text("hello\x00world\x01!"), "helloworld!");
    }

    #[test]
    fn sanitize_text_removes_replacement_char() {
        assert_eq!(sanitize_text("good\u{FFFD}text"), "goodtext");
    }

    #[test]
    fn sanitize_text_preserves_ascii() {
        assert_eq!(sanitize_text("Hello World! 123"), "Hello World! 123");
    }

    #[test]
    fn sanitize_text_trims_whitespace() {
        assert_eq!(sanitize_text("  hello  "), "hello");
    }

    #[test]
    fn sanitize_text_removes_non_ascii_non_punctuation() {
        // Non-ASCII chars that aren't alphanumeric/punctuation/whitespace are removed
        let result = sanitize_text("café");
        // 'c', 'a', 'f' are ascii alphanum; 'é' is not ascii_alphanumeric
        assert_eq!(result, "caf");
    }
}
