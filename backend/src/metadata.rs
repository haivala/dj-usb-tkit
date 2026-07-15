//! Shared metadata cleanup used before writing player database text fields.

use std::borrow::Cow;

use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// Total exported string length cap, in Unicode characters.
const MAX_METADATA_CHARS: usize = 255;

/// Max codepoints kept per grapheme cluster (base character + combining marks).
///
/// Real-world clusters — accented letters, Hangul jamo, emoji ZWJ/skin-tone
/// sequences — essentially never exceed this. "Zalgo" text abuses combining
/// marks (Unicode category M) to stack dozens to hundreds of marks onto a
/// single base character; CDJ hardware text rendering has no bound on this
/// and hangs when asked to composite a cluster that deep. Capping here keeps
/// the visual glyph recognizable while bounding the renderer's work.
pub(crate) const MAX_GRAPHEME_CLUSTER_CHARS: usize = 8;

/// Max distinct (non-Common/Inherited/Unknown) Unicode scripts kept in a
/// single exported string; characters from further scripts are dropped.
///
/// Real names rarely mix more than a couple of scripts (e.g. a romanized
/// title next to its native-script form). A string that hops through many
/// unrelated scripts in a handful of characters — Braille, Yi, Tibetan,
/// Bengali, and Arabic marks all in one artist name, observed in the wild —
/// has hung CDJ text rendering independent of string length or combining-mark
/// depth, most likely in per-script font/bidi handling. This caps diversity
/// per string without curating a list of which scripts are "safe": the
/// allowed set is just whichever scripts a string touches first.
pub(crate) const MAX_DISTINCT_SCRIPTS: usize = 3;

fn counts_toward_script_diversity(script: Script) -> bool {
    !matches!(script, Script::Common | Script::Inherited | Script::Unknown)
}

/// Cap every grapheme cluster in `s` to at most `max_cluster_chars` codepoints.
///
/// Used both for exported metadata text ([`sanitize_metadata`]) and for
/// on-disk file/folder names built from that metadata, so a pathological
/// combining-mark stack can't reach the player from either path.
pub(crate) fn cap_grapheme_clusters(s: &str, max_cluster_chars: usize) -> Cow<'_, str> {
    if s.graphemes(true)
        .all(|g| g.chars().count() <= max_cluster_chars)
    {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for grapheme in s.graphemes(true) {
        out.extend(grapheme.chars().take(max_cluster_chars));
    }
    Cow::Owned(out)
}

/// Cap the number of distinct scripts touched by `s`. Once
/// [`MAX_DISTINCT_SCRIPTS`] distinct scripts have appeared, whole grapheme
/// clusters (base character plus its combining marks) whose base belongs to
/// any further script are dropped together; clusters whose base is already
/// in an allowed script (plus script-neutral bases: punctuation, digits,
/// stray combining marks) always pass through.
///
/// Clusters are dropped as a unit rather than character-by-character:
/// dropping only an over-budget base while letting its combining marks
/// through (they're `Script::Inherited`, which is script-neutral) would
/// leave the marks to reattach onto whatever base character precedes them
/// in the output, reconstituting an oversized cluster — the exact "zalgo"
/// shape [`cap_grapheme_clusters`] exists to prevent.
///
/// Used both for exported metadata text ([`sanitize_metadata`]) and for
/// on-disk file/folder names built from that metadata.
pub(crate) fn cap_script_diversity(s: &str) -> Cow<'_, str> {
    let mut seen = Vec::with_capacity(MAX_DISTINCT_SCRIPTS);
    let mut overflow = false;
    for grapheme in s.graphemes(true) {
        let Some(script) = grapheme.chars().next().map(|c| c.script()) else {
            continue;
        };
        if !counts_toward_script_diversity(script) || seen.contains(&script) {
            continue;
        }
        if seen.len() >= MAX_DISTINCT_SCRIPTS {
            overflow = true;
            break;
        }
        seen.push(script);
    }
    if !overflow {
        return Cow::Borrowed(s);
    }

    let mut allowed: Vec<Script> = Vec::with_capacity(MAX_DISTINCT_SCRIPTS);
    let mut out = String::with_capacity(s.len());
    for grapheme in s.graphemes(true) {
        let base_script = grapheme.chars().next().map(|c| c.script());
        let keep = match base_script {
            Some(script) if counts_toward_script_diversity(script) => {
                if allowed.contains(&script) {
                    true
                } else if allowed.len() < MAX_DISTINCT_SCRIPTS {
                    allowed.push(script);
                    true
                } else {
                    false
                }
            }
            _ => true,
        };
        if keep {
            out.push_str(grapheme);
        }
        // else: cluster's base belongs to a script beyond the budget — drop
        // the whole cluster, combining marks included.
    }
    Cow::Owned(out)
}

/// Strip null bytes, cap runaway combining-mark stacks ("zalgo" text), cap
/// how many distinct scripts a string may mix, and truncate to 255 Unicode
/// characters.
///
/// Applied to exported metadata strings (titles, artist names, album names),
/// not media paths, analysis paths, or key/tonality values.
pub(crate) fn sanitize_metadata(s: &str) -> Cow<'_, str> {
    let has_nul = s.contains('\0');
    let over_length = s.chars().count() > MAX_METADATA_CHARS;
    let has_runaway_cluster = s
        .graphemes(true)
        .any(|g| g.chars().count() > MAX_GRAPHEME_CLUSTER_CHARS);
    let has_excess_script_diversity = matches!(cap_script_diversity(s), Cow::Owned(_));

    if !has_nul && !over_length && !has_runaway_cluster && !has_excess_script_diversity {
        return Cow::Borrowed(s);
    }

    let mut out = String::new();
    let mut total_chars = 0usize;
    let mut allowed_scripts: Vec<Script> = Vec::with_capacity(MAX_DISTINCT_SCRIPTS);
    'clusters: for grapheme in s.graphemes(true) {
        if total_chars >= MAX_METADATA_CHARS {
            break;
        }
        // Nul-strip and depth-cap first, then decide script diversity for
        // the cluster as a whole (base character decides, marks follow) —
        // never split a cluster's base from its own combining marks, or a
        // dropped base's marks would reattach onto the previous cluster and
        // reconstitute an oversized one.
        let capped: String = grapheme
            .chars()
            .filter(|&c| c != '\0')
            .take(MAX_GRAPHEME_CLUSTER_CHARS)
            .collect();
        let base_script = capped.chars().next().map(|c| c.script());
        if let Some(script) = base_script
            && counts_toward_script_diversity(script)
            && !allowed_scripts.contains(&script)
        {
            if allowed_scripts.len() >= MAX_DISTINCT_SCRIPTS {
                continue 'clusters;
            }
            allowed_scripts.push(script);
        }
        for ch in capped.chars() {
            if total_chars >= MAX_METADATA_CHARS {
                break 'clusters;
            }
            out.push(ch);
            total_chars += 1;
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_metadata_passthrough_when_clean() {
        let s = "Artist Name";
        let result = sanitize_metadata(s);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result.as_ref(), s);
    }

    #[test]
    fn sanitize_metadata_strips_null_bytes() {
        let s = "Art\0ist";
        assert_eq!(sanitize_metadata(s).as_ref(), "Artist");
    }

    #[test]
    fn sanitize_metadata_truncates_to_255_chars() {
        let long: String = "a".repeat(300);
        let result = sanitize_metadata(&long);
        assert_eq!(result.chars().count(), 255);
    }

    #[test]
    fn sanitize_metadata_truncates_unicode_by_char_not_byte() {
        // 200 × 3-byte UTF-8 char → should be truncated at 255 chars, not 255 bytes
        let long: String = "ä".repeat(300);
        let result = sanitize_metadata(&long);
        assert_eq!(result.chars().count(), 255);
    }

    #[test]
    fn sanitize_metadata_null_and_long_combined() {
        let mut s = "x\0".repeat(200);
        s.push_str("tail");
        let result = sanitize_metadata(&s);
        // After stripping nulls: 204 'x' chars + "tail" = 204+4 = 208, fits in 255
        assert!(!result.contains('\0'));
        assert!(result.chars().count() <= 255);
    }

    #[test]
    fn sanitize_metadata_caps_zalgo_combining_marks() {
        // A single base char with 60 stacked combining marks (category Mn),
        // as produced by "zalgo text" generators.
        let base = 'e';
        let mark = '\u{0301}'; // COMBINING ACUTE ACCENT
        let zalgo: String = std::iter::once(base).chain(std::iter::repeat(mark).take(60)).collect();
        let result = sanitize_metadata(&zalgo);
        assert_eq!(
            result.chars().count(),
            MAX_GRAPHEME_CLUSTER_CHARS,
            "cluster must be capped, not passed through whole"
        );
        assert_eq!(result.chars().next(), Some('e'), "base character is preserved");
    }

    #[test]
    fn sanitize_metadata_caps_zalgo_across_many_clusters_and_total_length() {
        // Real-world case: dozens of base characters, each with a long
        // combining-mark tail, exceeding both the per-cluster cap and the
        // total 255-character budget.
        let mark = '\u{0301}';
        let cluster = |base: char| -> String {
            std::iter::once(base)
                .chain(std::iter::repeat(mark).take(20))
                .collect()
        };
        let zalgo: String = "the quick brown fox jumps over the lazy dog and more text after"
            .chars()
            .map(cluster)
            .collect();
        let result = sanitize_metadata(&zalgo);
        assert!(result.chars().count() <= MAX_METADATA_CHARS);
        assert!(
            result
                .graphemes(true)
                .all(|g| g.chars().count() <= MAX_GRAPHEME_CLUSTER_CHARS)
        );
    }

    #[test]
    fn cap_grapheme_clusters_leaves_normal_text_untouched() {
        let s = "Café del Mar — Niño de Elche";
        let result = cap_grapheme_clusters(s, MAX_GRAPHEME_CLUSTER_CHARS);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result.as_ref(), s);
    }

    #[test]
    fn cap_grapheme_clusters_bounds_pathological_input() {
        let mark = '\u{0301}';
        let s: String = std::iter::once('e').chain(std::iter::repeat(mark).take(100)).collect();
        let result = cap_grapheme_clusters(&s, MAX_GRAPHEME_CLUSTER_CHARS);
        assert_eq!(result.chars().count(), MAX_GRAPHEME_CLUSTER_CHARS);
    }

    #[test]
    fn sanitize_metadata_caps_synthetic_zalgo_release_style_tag() {
        // Shaped like the "zalgo" release tags this cap was built for
        // (fabricated, not pulled from any real file): a mix of short
        // clusters and a few base characters carrying dozens of stacked
        // combining marks, totalling well past 255 codepoints. Before the
        // grapheme-cluster cap, this passed sanitize_metadata's 255-char
        // check untouched and reportedly hung CDJ text rendering.
        let mark_a = '\u{0301}'; // COMBINING ACUTE ACCENT
        let mark_b = '\u{0362}'; // COMBINING DOUBLE RIGHTWARDS ARROW BELOW
        let heavy_cluster = |base: char, mark: char, count: usize| -> String {
            std::iter::once(base).chain(std::iter::repeat(mark).take(count)).collect::<String>()
        };
        let mut album = String::new();
        album.push_str(&heavy_cluster('x', mark_a, 40));
        album.push_str(" :: ");
        album.push_str(&heavy_cluster('y', mark_b, 70));
        album.push_str(" -- ");
        album.push_str(&"z".repeat(120));
        album.push_str(&heavy_cluster('w', mark_a, 30));

        assert!(album.chars().count() > MAX_METADATA_CHARS);
        let max_cluster_before = album
            .graphemes(true)
            .map(|g| g.chars().count())
            .max()
            .unwrap_or(0);
        assert!(
            max_cluster_before > MAX_GRAPHEME_CLUSTER_CHARS,
            "fixture should contain an oversized cluster; got max {max_cluster_before}"
        );

        let result = sanitize_metadata(&album);

        assert!(result.chars().count() <= MAX_METADATA_CHARS);
        assert!(
            result
                .graphemes(true)
                .all(|g| g.chars().count() <= MAX_GRAPHEME_CLUSTER_CHARS),
            "no cluster in the sanitized output may exceed the cap"
        );
    }

    #[test]
    fn cap_script_diversity_leaves_single_script_text_untouched() {
        let s = "Café del Mar — Niño de Elche";
        let result = cap_script_diversity(s);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result.as_ref(), s);
    }

    #[test]
    fn cap_script_diversity_allows_two_scripts_mixed() {
        // A romanized title next to its native-script form is legitimate
        // and must survive untouched (Latin + Han is 2 scripts, under cap).
        let s = "Tokyo 東京";
        let result = cap_script_diversity(s);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result.as_ref(), s);
    }

    #[test]
    fn cap_script_diversity_drops_characters_beyond_budget() {
        // Latin, Han, Hiragana, Cyrillic, Greek: 5 distinct scripts.
        let s = "aあ中Бγ";
        let result = cap_script_diversity(s);
        let scripts_kept = result
            .chars()
            .map(|c| c.script())
            .filter(|&sc| counts_toward_script_diversity(sc))
            .collect::<std::collections::HashSet<_>>();
        assert!(
            scripts_kept.len() <= MAX_DISTINCT_SCRIPTS,
            "expected at most {MAX_DISTINCT_SCRIPTS} distinct scripts, kept {scripts_kept:?}"
        );
        assert!(result.starts_with('a'), "characters before the budget is hit are preserved");
    }

    #[test]
    fn sanitize_metadata_caps_synthetic_script_salad_name() {
        // Fabricated (not pulled from any real file), shaped like a name
        // observed in the wild that hung CDJ hardware (browsing the Artist
        // menu, and loading a track) despite no single grapheme cluster
        // being deep enough to trip the combining-mark cap on its own:
        // eight unrelated scripts packed into a short string. Codepoints
        // below are Cherokee, Runic, Glagolitic, Coptic, N'Ko, Vai,
        // Osmanya, and Deseret — chosen only because they're distinct
        // scripts, not for any resemblance to real text.
        let artist =
            "\u{13A0}\u{13A1} \u{16A0}\u{16A1} \u{2C00}\u{2C01} \u{2C80}\u{2C81} \u{07CA}\u{07CB} \u{A500}\u{A501} \u{10480}\u{10481} \u{10400}\u{10401}";
        let max_cluster_before = artist
            .graphemes(true)
            .map(|g| g.chars().count())
            .max()
            .unwrap_or(0);
        assert!(
            max_cluster_before <= MAX_GRAPHEME_CLUSTER_CHARS,
            "fixture should NOT trip the depth cap on its own; got max cluster {max_cluster_before}"
        );

        let result = sanitize_metadata(artist);
        let scripts_kept = result
            .chars()
            .map(|c| c.script())
            .filter(|&sc| counts_toward_script_diversity(sc))
            .collect::<std::collections::HashSet<_>>();
        assert!(
            scripts_kept.len() <= MAX_DISTINCT_SCRIPTS,
            "expected at most {MAX_DISTINCT_SCRIPTS} distinct scripts, kept {scripts_kept:?}"
        );
        assert!(!result.is_empty());
    }

    #[test]
    fn cap_script_diversity_drops_whole_cluster_not_just_base() {
        // A cluster whose base script is over budget must be dropped
        // entirely, including its combining marks — not just the base,
        // which would leave the (script-neutral, Inherited) marks to
        // reattach onto the previous surviving base character and
        // reconstitute an oversized cluster.
        let mark = '\u{0301}'; // COMBINING ACUTE ACCENT (Script::Inherited)
        let mut s = String::new();
        s.push('a'); // Latin (1/3)
        s.push('\u{3042}'); // Hiragana あ (2/3)
        s.push('\u{4e2d}'); // Han 中 (3/3, budget now full)
        // Two more clusters whose base script (Cyrillic, Greek) is over
        // budget, each dragging several combining marks along.
        s.push('\u{0411}'); // Cyrillic Б — over budget
        s.extend(std::iter::repeat(mark).take(7));
        s.push('\u{03a9}'); // Greek Ω — over budget
        s.extend(std::iter::repeat(mark).take(7));

        let result = cap_script_diversity(&s);
        assert_eq!(result.as_ref(), "a\u{3042}\u{4e2d}", "over-budget clusters must be dropped whole, including their marks");
        assert!(
            result
                .graphemes(true)
                .all(|g| g.chars().count() <= MAX_GRAPHEME_CLUSTER_CHARS),
            "no orphaned marks may reattach onto a surviving cluster"
        );
    }

    #[test]
    fn sanitize_metadata_drops_whole_cluster_not_just_base() {
        // Same scenario as cap_script_diversity_drops_whole_cluster_not_just_base,
        // exercised through the fused sanitize_metadata path.
        let mark = '\u{0301}';
        let mut s = String::new();
        s.push('a');
        s.push('\u{3042}');
        s.push('\u{4e2d}');
        s.push('\u{0411}');
        s.extend(std::iter::repeat(mark).take(7));
        s.push('\u{03a9}');
        s.extend(std::iter::repeat(mark).take(7));

        let result = sanitize_metadata(&s);
        assert_eq!(result.as_ref(), "a\u{3042}\u{4e2d}");
        assert!(
            result
                .graphemes(true)
                .all(|g| g.chars().count() <= MAX_GRAPHEME_CLUSTER_CHARS),
            "dropped clusters' marks must not reattach onto the last kept base"
        );
    }
}
