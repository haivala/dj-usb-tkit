//! The text catalog: every user-facing text, by key, in each language the app has.
//!
//! The catalogs are `locales/<language>.ftl` (Fluent), embedded in the binary. `build.rs`
//! turns each message of `locales/en.ftl` into a function in [`tr`], named after its key
//! (`status-warning-count` is `tr::status_warning_count(count)`), so a wrong key, a wrong argument
//! or a missing one fails the build.
//!
//! A [`Text`] is a key and its arguments. It becomes text only when shown: [`Text::render`]
//! in the process's current language, [`Text::render_english`] for logs. A `Text` passed as
//! another's argument stays a `Text` until the outer one renders, so it comes out in the same
//! language; for that, `Text` has no `Display` (see [`IntoArg`]). The
//! current language is one per process, readable from any thread; with nothing set, as in
//! tests and the CLI tools, it's English. A key missing from another language falls back to
//! English.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::types::{FluentNumber, FluentNumberOptions};
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;

use crate::models::{GetUiTextsData, UiLanguage, UiText};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/i18n_catalog.rs"));
}

pub use generated::tr;

/// The language every catalog falls back to, and the one logs are written in.
pub const ENGLISH: &str = "en";

/// The test language `build.rs` generates from English: every text bracketed, accented and
/// longer (`[Ŝéééţţîîñĝš]`). Debug builds only, so a release never lists it.
pub const TEST_LANGUAGE: &str = "en-XA";

/// A text not yet rendered: a catalog key and its arguments. Made by the functions in [`tr`].
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    key: &'static str,
    args: Vec<(&'static str, Arg)>,
}

/// One argument of a [`Text`]. Numbers stay numbers, so a translation can pick its plural form.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// Data, shown as it is: a name, a path, a reason from the OS.
    Text(String),
    Number(Num),
    /// A catalog text, rendered in the language the outer text is rendered in.
    Inner(Box<Text>),
}

/// What a text argument of a [`tr`] function takes: anything `Display` (data, shown as it
/// is), or a [`Text`] or catalog error, which render with the text they're passed into.
///
/// `Text` and `BackendError` have no `Display`, so neither can reach an argument (or a
/// `format!`) already rendered in one language.
pub trait IntoArg {
    fn into_arg(self) -> Arg;
}

impl<T: fmt::Display> IntoArg for T {
    fn into_arg(self) -> Arg {
        Arg::Text(self.to_string())
    }
}

impl IntoArg for Text {
    fn into_arg(self) -> Arg {
        Arg::Inner(Box::new(self))
    }
}

impl IntoArg for &Text {
    fn into_arg(self) -> Arg {
        Arg::Inner(Box::new(self.clone()))
    }
}

/// An argument made earlier and kept, such as an error an outcome holds.
impl IntoArg for Arg {
    fn into_arg(self) -> Arg {
        self
    }
}

impl Arg {
    /// The argument on its own, in the current language: for a value shown outside any text.
    pub fn render(&self) -> String {
        match self {
            Arg::Text(value) => value.clone(),
            Arg::Number(value) => value.fluent().as_string().into_owned(),
            Arg::Inner(text) => text.render(),
        }
    }
}

/// `value` as shown in the current language: data as it is, a [`Text`] rendered. For fields
/// that hold either, such as a diagnostics check's detail.
pub fn shown(value: impl IntoArg) -> String {
    value.into_arg().render()
}

/// A number argument: any integer or float. Counts are what plural forms select on.
///
/// Shown with the language's decimal separator (`format-decimal-separator`) and no digit
/// grouping, so "1234 tracks" and a track id read the same in every language. A float shows
/// its shortest form ("2.5"); [`Num::fixed`] gives it a set number of decimals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Num {
    value: f64,
    fraction_digits: Option<usize>,
}

impl Num {
    /// `value` rounded to exactly `digits` decimals: `Num::fixed(27.0, 1)` shows "27.0".
    pub fn fixed(value: f64, digits: usize) -> Self {
        Self {
            value,
            fraction_digits: Some(digits),
        }
    }

    fn fluent(self) -> FluentNumber {
        let options = FluentNumberOptions {
            minimum_fraction_digits: self.fraction_digits,
            maximum_fraction_digits: self.fraction_digits,
            ..FluentNumberOptions::default()
        };
        FluentNumber::new(self.value, options)
    }
}

macro_rules! num_from {
    ($($ty:ty)+) => {
        $(impl From<$ty> for Num {
            fn from(value: $ty) -> Self {
                Num { value: value as f64, fraction_digits: None }
            }
        })+
    };
}
num_from!(u8 u16 u32 u64 usize i8 i16 i32 i64 isize f32 f64);

thread_local! {
    /// The decimal separator of the catalog formatting on this thread, for [`format_number`],
    /// which Fluent calls with no way to reach the catalog.
    static DECIMAL_SEPARATOR: Cell<char> = const { Cell::new('.') };
}

/// Every number Fluent shows: `maximumFractionDigits` decimals when set (rounded), else the
/// shortest form, then the language's decimal separator.
fn format_number<M>(value: &FluentValue, _intls: &M) -> Option<String> {
    let FluentValue::Number(number) = value else {
        return None;
    };
    let text = match number.options.maximum_fraction_digits {
        Some(digits) => format!("{:.*}", digits, number.value),
        None => number.as_string().into_owned(),
    };
    let separator = DECIMAL_SEPARATOR.get();
    Some(if separator == '.' {
        text
    } else {
        text.replace('.', &separator.to_string())
    })
}

impl Text {
    /// For the generated functions in [`tr`]; everything else gets a `Text` from those.
    fn new(key: &'static str, args: Vec<(&'static str, Arg)>) -> Self {
        Self { key, args }
    }

    pub fn key(&self) -> &'static str {
        self.key
    }

    /// The text in the process's current language.
    pub fn render(&self) -> String {
        self.render_in(&current())
    }

    /// The text in English, for the log file.
    pub fn render_english(&self) -> String {
        self.render_in(&ENGLISH_CATALOG)
    }

    fn render_in(&self, catalog: &Catalog) -> String {
        catalog
            .format(self)
            .or_else(|| ENGLISH_CATALOG.format(self))
            // build.rs makes a function only for a key en.ftl has, so this can't happen.
            .unwrap_or_else(|| self.key.to_string())
    }
}

/// The renderer for one of the two languages a line is written in: English for the log file,
/// else the current language. For composing a line of several texts with one renderer.
pub fn renderer(english: bool) -> fn(&Text) -> String {
    if english {
        Text::render_english
    } else {
        Text::render
    }
}

/// One language's parsed catalog.
struct Catalog {
    language: &'static str,
    /// The `.ftl` text, for `ui_texts`, which sends patterns rather than formatted text.
    source: &'static str,
    bundle: FluentBundle<FluentResource>,
    /// `format-decimal-separator`'s first character; `.` when the catalog has none.
    decimal_separator: char,
}

impl Catalog {
    fn parse(language: &'static str, source: &'static str) -> Result<Self, String> {
        let id: LanguageIdentifier = language
            .parse()
            .map_err(|err| format!("`{language}` isn't a language code: {err}"))?;
        let resource = FluentResource::try_new(source.to_string())
            .map_err(|(_, errors)| format!("the `{language}` catalog doesn't parse: {errors:?}"))?;
        let mut bundle = FluentBundle::new_concurrent(vec![id]);
        // Fluent puts invisible bidi isolation marks around every argument by default.
        bundle.set_use_isolating(false);
        bundle.set_formatter(Some(format_number));
        bundle
            .add_builtins()
            .map_err(|err| format!("adding Fluent's NUMBER(): {err:?}"))?;
        bundle.add_resource(resource).map_err(|errors| {
            format!("the `{language}` catalog has a duplicate key: {errors:?}")
        })?;
        let decimal_separator = bundle
            .get_message("format-decimal-separator")
            .and_then(|message| message.value())
            .and_then(|pattern| {
                let mut errors = Vec::new();
                bundle
                    .format_pattern(pattern, None, &mut errors)
                    .chars()
                    .next()
            })
            .unwrap_or('.');
        Ok(Self {
            language,
            source,
            bundle,
            decimal_separator,
        })
    }

    /// The text in this catalog's language, or `None` when the key isn't translated.
    fn format(&self, text: &Text) -> Option<String> {
        let pattern = self.bundle.get_message(text.key)?.value()?;
        let mut args = FluentArgs::new();
        for (name, arg) in &text.args {
            let value = match arg {
                Arg::Text(value) => FluentValue::from(value.as_str()),
                Arg::Number(value) => FluentValue::Number(value.fluent()),
                Arg::Inner(inner) => FluentValue::from(inner.render_in(self)),
            };
            args.set(*name, value);
        }
        let mut errors = Vec::new();
        DECIMAL_SEPARATOR.set(self.decimal_separator);
        let rendered = self
            .bundle
            .format_pattern(pattern, Some(&args), &mut errors);
        // build.rs checks every catalog only uses arguments the key's function passes.
        debug_assert!(errors.is_empty(), "formatting `{}`: {errors:?}", text.key);
        Some(rendered.into_owned())
    }
}

static ENGLISH_CATALOG: LazyLock<Arc<Catalog>> = LazyLock::new(|| {
    let (language, source) = generated::LANGUAGE_SOURCES[0];
    Arc::new(Catalog::parse(language, source).expect("build.rs checked en.ftl"))
});

static CURRENT: LazyLock<RwLock<Arc<Catalog>>> =
    LazyLock::new(|| RwLock::new(Arc::clone(&ENGLISH_CATALOG)));

fn current() -> Arc<Catalog> {
    Arc::clone(&CURRENT.read().unwrap_or_else(PoisonError::into_inner))
}

/// The language codes the app has a catalog for, English first.
pub fn languages() -> Vec<&'static str> {
    generated::LANGUAGE_SOURCES
        .iter()
        .map(|(language, _)| *language)
        .collect()
}

/// The process's current language code.
pub fn current_language() -> &'static str {
    current().language
}

/// Makes `language` the process's current language, for every thread. The catalog is parsed
/// before the switch, so no thread ever sees a half-loaded language.
pub fn set_language(language: &str) -> Result<(), String> {
    let catalog = if language == ENGLISH {
        Arc::clone(&ENGLISH_CATALOG)
    } else {
        let (code, source) = generated::LANGUAGE_SOURCES
            .iter()
            .find(|(code, _)| *code == language)
            .ok_or_else(|| format!("no catalog for language `{language}`"))?;
        Arc::new(Catalog::parse(code, source)?)
    };
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = catalog;
    Ok(())
}

/// The language for the computer's languages (most preferred first), until one is chosen:
/// see [`matching_language`]. `None` leaves the app in English.
pub fn system_language() -> Option<&'static str> {
    let locales: Vec<String> = sys_locale::get_locales().collect();
    matching_language(locales.iter().map(String::as_str), &languages())
}

/// The first of `preferred` (locale names, most preferred first) the app has a catalog for:
/// by the whole code (`pt-BR`), else by its language alone (`fr-CA` is `fr`). Case, `_` for
/// `-` and a POSIX `.UTF-8` or `@euro` ending don't matter. The test language never matches.
fn matching_language<'a>(
    preferred: impl IntoIterator<Item = &'a str>,
    available: &[&'static str],
) -> Option<&'static str> {
    let available: Vec<&'static str> = available
        .iter()
        .copied()
        .filter(|code| *code != TEST_LANGUAGE)
        .collect();
    preferred.into_iter().find_map(|locale| {
        let locale = locale.split(['.', '@']).next().unwrap_or_default().replace('_', "-");
        let base = locale.split('-').next().unwrap_or_default();
        available
            .iter()
            .find(|code| code.eq_ignore_ascii_case(&locale))
            .or_else(|| available.iter().find(|code| code.eq_ignore_ascii_case(base)))
            .copied()
    })
}

/// Keys with this prefix are the text JS and `index.html` show, sent by `get_ui_texts`.
pub const UI_KEY_PREFIX: &str = "ui-";

/// Every `ui-` text in the current language, English where it has none: `get_ui_texts`.
pub fn ui_texts() -> GetUiTextsData {
    ui_texts_in(&current())
}

fn ui_texts_in(catalog: &Catalog) -> GetUiTextsData {
    let translated = ui_patterns(catalog.source);
    let texts = ui_patterns(ENGLISH_CATALOG.source)
        .into_iter()
        .map(|(key, english)| {
            let text = translated.get(&key).cloned().unwrap_or(english);
            (key, text)
        })
        .collect();
    GetUiTextsData {
        language: catalog.language.to_string(),
        decimal_separator: catalog.decimal_separator.to_string(),
        texts,
        languages: LANGUAGE_NAMES.clone(),
        language_choice: LANGUAGE_NAMES.len() > 1,
    }
}

/// Every language the app has, each named in itself (`settings-language-name` in its own
/// catalog, its code when it has none).
static LANGUAGE_NAMES: LazyLock<Vec<UiLanguage>> = LazyLock::new(|| {
    generated::LANGUAGE_SOURCES
        .iter()
        .map(|(code, source)| {
            let name = Catalog::parse(code, source)
                .ok()
                .and_then(|catalog| catalog.format(&tr::settings_language_name()))
                .unwrap_or_else(|| code.to_string());
            UiLanguage {
                code: code.to_string(),
                name,
            }
        })
        .collect()
});

/// The `ui-` messages of one catalog, as JS fills them in (see `ui_text`).
fn ui_patterns(source: &str) -> BTreeMap<String, UiText> {
    // build.rs checked every catalog parses.
    let resource = fluent_syntax::parser::parse(source).unwrap_or_else(|(resource, _)| resource);
    resource
        .body
        .iter()
        .filter_map(|entry| match entry {
            ast::Entry::Message(message) if message.id.name.starts_with(UI_KEY_PREFIX) => Some((
                message.id.name.to_string(),
                ui_text(message.value.as_ref()?),
            )),
            _ => None,
        })
        .collect()
}

/// A `ui-` message for JS: its text with each variable as `{name}`, which `t` fills in. A
/// message with a plural (build.rs allows one, on `$count`, with plural categories only) is
/// its forms by category, the text around the plural repeated in each; `tCount` picks one.
fn ui_text(pattern: &ast::Pattern<&str>) -> UiText {
    let select = pattern.elements.iter().position(|element| {
        matches!(
            element,
            ast::PatternElement::Placeable {
                expression: ast::Expression::Select { .. }
            }
        )
    });
    let Some(index) = select else {
        return UiText::Text(js_text(&pattern.elements));
    };
    let ast::PatternElement::Placeable {
        expression: ast::Expression::Select { variants, .. },
    } = &pattern.elements[index]
    else {
        unreachable!("found as a select above");
    };
    let before = js_text(&pattern.elements[..index]);
    let after = js_text(&pattern.elements[index + 1..]);
    UiText::Counted(
        variants
            .iter()
            .map(|variant| {
                let category = match &variant.key {
                    ast::VariantKey::Identifier { name }
                    | ast::VariantKey::NumberLiteral { value: name } => name.to_string(),
                };
                (
                    category,
                    format!("{before}{}{after}", js_text(&variant.value.elements)),
                )
            })
            .collect(),
    )
}

fn js_text(elements: &[ast::PatternElement<&str>]) -> String {
    let mut out = String::new();
    for element in elements {
        match element {
            ast::PatternElement::TextElement { value } => out.push_str(value),
            ast::PatternElement::Placeable { expression } => js_expression(expression, &mut out),
        }
    }
    out
}

fn js_expression(expression: &ast::Expression<&str>, out: &mut String) {
    match expression {
        ast::Expression::Inline(ast::InlineExpression::VariableReference { id }) => {
            out.push('{');
            out.push_str(id.name);
            out.push('}');
        }
        ast::Expression::Inline(
            ast::InlineExpression::StringLiteral { value }
            | ast::InlineExpression::NumberLiteral { value },
        ) => out.push_str(value),
        ast::Expression::Inline(ast::InlineExpression::Placeable { expression }) => {
            js_expression(expression, out)
        }
        // build.rs allows nothing else in a `ui-` message.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISOLATION_MARKS: [char; 4] = ['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'];

    // Another language, for fallback and plural tests without changing the process's language
    // (other tests in this process assert English).
    const OTHER_SOURCE: &str = "\
fixture-hello = Hei { $name }!
status-warning-count = { $count ->
    [one] { $count } varoitus
   *[other] { $count } varoitusta
}
";

    fn other_catalog() -> Catalog {
        Catalog::parse("fi", OTHER_SOURCE).expect("fixture parses")
    }

    #[test]
    fn every_catalog_parses() {
        for (language, source) in generated::LANGUAGE_SOURCES {
            if let Err(err) = Catalog::parse(language, source) {
                panic!("{err}");
            }
        }
        assert_eq!(languages().first(), Some(&ENGLISH));
    }

    #[test]
    fn english_plural_for_zero_one_and_two() {
        let english = |count: usize| tr::status_warning_count(count).render_english();
        assert_eq!(english(0), "(0 warnings)");
        assert_eq!(english(1), "(1 warning)");
        assert_eq!(english(2), "(2 warnings)");
    }

    #[test]
    fn placeholders_are_filled_without_isolation_marks() {
        let catalog = other_catalog();
        let text = Text::new("fixture-hello", vec![("name", Arg::Text("Example".into()))]);
        let rendered = catalog.format(&text).expect("fixture key");
        assert_eq!(rendered, "Hei Example!");
        let plural = tr::status_warning_count(3usize).render_in(&catalog);
        assert_eq!(plural, "3 varoitusta");
        for output in [
            rendered,
            plural,
            tr::status_warning_count(2usize).render_english(),
        ] {
            assert!(!output.contains(ISOLATION_MARKS), "{output:?}");
        }
    }

    #[test]
    fn a_text_argument_renders_in_the_outer_texts_language() {
        let outer = Text::new(
            "fixture-hello",
            vec![("name", tr::status_warning_count(2usize).into_arg())],
        );
        assert_eq!(outer.render_in(&other_catalog()), "Hei 2 varoitusta!");
        let logged = tr::repair_outcome_failed("Fix X", tr::status_warning_count(2usize));
        assert_eq!(logged.render_english(), "Fix X failed: (2 warnings)");
        // An outer text another language lacks falls back to English, inner text included,
        // so the line stays in one language.
        assert_eq!(
            logged.render_in(&other_catalog()),
            "Fix X failed: (2 warnings)"
        );
    }

    #[test]
    fn a_key_missing_from_another_language_falls_back_to_english() {
        let catalog = Catalog::parse("fi", "fixture-hello = Hei!\n").expect("fixture parses");
        assert_eq!(
            tr::status_warning_count(1usize).render_in(&catalog),
            "(1 warning)"
        );
    }

    #[test]
    fn unset_language_is_english() {
        // Nothing in the unit tests sets a language; piece 2's own test file covers switching.
        assert_eq!(current_language(), ENGLISH);
        assert_eq!(tr::status_warning_count(2usize).render(), "(2 warnings)");
    }

    #[test]
    fn ui_texts_send_placeholders_and_plural_forms() {
        let texts = ui_patterns(
            "\
ui-test-hello = Hello { $name }!
ui-test-count = Adding { $count ->
    [one] { $count } track
   *[other] { $count } tracks
} to { $playlist }
usb-not-for-ui = Not sent
",
        );
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert_eq!(texts["ui-test-hello"], UiText::Text("Hello {name}!".into()));
        assert_eq!(
            texts["ui-test-count"],
            UiText::Counted(BTreeMap::from([
                ("one".into(), "Adding {count} track to {playlist}".into()),
                ("other".into(), "Adding {count} tracks to {playlist}".into()),
            ]))
        );
    }

    #[test]
    fn ui_texts_fall_back_to_english_per_key() {
        let catalog = Catalog::parse("fi", "ui-header-usb-name-badge-unnamed = Nimetön\n")
            .expect("fixture parses");
        let data = ui_texts_in(&catalog);
        assert_eq!(data.language, "fi");
        assert_eq!(
            data.texts["ui-header-usb-name-badge-unnamed"],
            UiText::Text("Nimetön".into())
        );
        assert_eq!(
            data.texts["ui-header-usb-name-badge-not-connected"],
            UiText::Text("Not connected".into())
        );
        assert!(data.texts.keys().all(|key| key.starts_with(UI_KEY_PREFIX)));
    }

    #[test]
    fn ui_texts_send_the_languages_decimal_separator() {
        let catalog =
            Catalog::parse("fi", "format-decimal-separator = ,\n").expect("fixture parses");
        assert_eq!(ui_texts_in(&catalog).decimal_separator, ",");
        assert_eq!(ui_texts_in(&ENGLISH_CATALOG).decimal_separator, ".");
    }

    /// One English text, one key: a text with the same role in several places is one
    /// `ui-common-*` key (docs/TRANSLATING.md), so a translator translates it once. These
    /// share their English but not their role, and may translate differently.
    const SAME_ENGLISH_KEPT_APART: &[&[&str]] = &[
        // The sidebar's heading, the Add to… list, the import dialog's group, a diagnostics area.
        &[
            "diagnostics-area-playlists",
            "ui-add-to-list-aria-label",
            "ui-playlist-import-group-playlists",
            "ui-sidebar-playlists",
        ],
        // The database's name: a backup's file, a diagnostics section and check, a table column.
        &[
            "backups-files-edb",
            "diagnostics-section-edb",
            "diagnostics-check-edb",
            "ui-usb-edb",
        ],
        &["backups-files-pdb", "diagnostics-section-pdb", "ui-usb-pdb"],
        // A diagnostics group, where playback came from, the sidebar's page.
        &[
            "diagnostics-group-library",
            "playback-source-library",
            "ui-sidebar-library",
        ],
        // Where playback came from vs the USB page.
        &["playback-source-usb", "ui-common-usb"],
        // The header's badge, the playlist page's title, a parity table's column.
        &[
            "ui-header-playlist-badge",
            "ui-playlist-title",
            "ui-usb-playlist",
        ],
        // A diagnostics section vs a Settings section.
        &["diagnostics-section-analysis", "ui-settings-analysis"],
        // A narrow column header (may be abbreviated, like "Fmt") vs a field's label.
        &["ui-track-detail-bpm", "ui-track-table-bpm"],
        &["ui-track-detail-key", "ui-track-table-key"],
        &["ui-common-genre", "ui-track-table-genre"],
        // A status word vs a button.
        &["ui-job-done", "ui-track-columns-done"],
        // A library's history lists vs the USB History page.
        &["ui-playlist-import-group-history", "ui-sidebar-history"],
        // A text field's placeholder vs a button.
        &["ui-add-to-new-name-placeholder", "ui-sidebar-new-playlist"],
        // A button that opens a dialog ("…") vs a button that acts, or vs words before a
        // playlist's name ("+ Add to Set 1"), or vs a Settings section.
        &["ui-selection-bar-add-to", "ui-selection-bar-add-to-target"],
        &["ui-convert-apply", "ui-selection-bar-convert"],
        &["ui-settings-export", "ui-tracklist-export-ok"],
        // The variable is a different thing: why it failed vs the playlist that wasn't deleted.
        &["backups-delete-failed", "playlist-delete-failed"],
        // A diagnostics check's result vs a sentence in the playlist's heading.
        &["diagnostics-check-engine-missing", "playlist-export-not-on-usb"],
        // The sidebar's short mark vs a sentence in the playlist's heading.
        &["export-mark-on-usb", "playlist-export-on-usb"],
        // Only variables around a colon: a word order each language picks per message.
        &[
            "analysis-track-failed",
            "analysis-track-incomplete",
            "repair-outcome-applied",
            "transcode-failed",
        ],
    ];

    /// A message's English as the duplicate check compares it: each variable is `{$}`
    /// whatever its name, and a trailing `.`, `...` or `…` is dropped, so "Not saved." and
    /// "Not saved", or "{ $error }" and "{ $reason }", are the same text.
    fn comparable_text(pattern: &ast::Pattern<&str>) -> String {
        fn expression(expression: &ast::Expression<&str>) -> String {
            match expression {
                ast::Expression::Inline(ast::InlineExpression::VariableReference { .. }) => {
                    "{$}".to_string()
                }
                ast::Expression::Select { variants, .. } => {
                    let variants: Vec<String> = variants
                        .iter()
                        .map(|variant| {
                            format!("{:?}={}", variant.key, comparable_text(&variant.value))
                        })
                        .collect();
                    format!("{{$ -> {}}}", variants.join("|"))
                }
                other => format!("{{{other:?}}}"),
            }
        }
        let mut text = String::new();
        for element in &pattern.elements {
            match element {
                ast::PatternElement::TextElement { value } => text.push_str(value),
                ast::PatternElement::Placeable { expression: inner } => {
                    text.push_str(&expression(inner))
                }
            }
        }
        let text = text.trim_end();
        ["...", "…", "."]
            .iter()
            .find_map(|end| text.strip_suffix(end))
            .unwrap_or(text)
            .to_string()
    }

    #[test]
    fn no_two_keys_share_their_english_text() {
        let resource = fluent_syntax::parser::parse(ENGLISH_CATALOG.source)
            .unwrap_or_else(|(resource, _)| resource);
        let mut keys_by_text: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for entry in &resource.body {
            if let ast::Entry::Message(message) = entry
                && let Some(value) = &message.value
            {
                keys_by_text
                    .entry(comparable_text(value))
                    .or_default()
                    .push(message.id.name);
            }
        }
        let shared: Vec<Vec<&str>> = keys_by_text
            .into_values()
            .filter(|keys| keys.len() > 1)
            .filter(|keys| {
                !SAME_ENGLISH_KEPT_APART.iter().any(|kept| {
                    let mut kept = kept.to_vec();
                    let mut keys = keys.clone();
                    kept.sort_unstable();
                    keys.sort_unstable();
                    kept == keys
                })
            })
            .collect();
        assert!(
            shared.is_empty(),
            "keys with the same English text: make each one key (ui-common-* when it's used in \
             several areas), or list it in SAME_ENGLISH_KEPT_APART with why: {shared:?}"
        );
    }

    /// The e2e specs' labels come from `vanilla-ui/tests/e2e/ui_texts.en.json` (loaded by
    /// `coverage-fixture.mjs`), and the test-language check's from `ui_texts.en-XA.json`.
    /// Rewrite both with `UPDATE_UI_TEXTS=1 cargo test -p backend --lib i18n::`.
    #[test]
    fn e2e_ui_texts_files_are_current() {
        let (test_language, test_source) = generated::LANGUAGE_SOURCES
            .iter()
            .find(|(language, _)| *language == TEST_LANGUAGE)
            .expect("debug builds have the test language");
        let test_catalog = Catalog::parse(test_language, test_source).expect("generated");
        for catalog in [&ENGLISH_CATALOG as &Catalog, &test_catalog] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../vanilla-ui/tests/e2e/ui_texts.{}.json",
                catalog.language
            ));
            let expected =
                serde_json::to_string_pretty(&ui_texts_in(catalog)).expect("serialize") + "\n";
            if std::env::var_os("UPDATE_UI_TEXTS").is_some() {
                std::fs::write(&path, &expected).expect("write the e2e texts file");
                continue;
            }
            let actual = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                actual == expected,
                "{} is out of date: run `UPDATE_UI_TEXTS=1 cargo test -p backend --lib i18n::`",
                path.display()
            );
        }
    }

    #[test]
    fn the_first_preferred_language_with_a_catalog_wins() {
        let available = ["en", "fr", "pt-BR", TEST_LANGUAGE];
        assert_eq!(matching_language(["de-DE", "fr-CA", "en-US"], &available), Some("fr"));
        assert_eq!(matching_language(["en-GB", "fr-FR"], &available), Some("en"));
        assert_eq!(matching_language(["pt-BR"], &available), Some("pt-BR"));
        assert_eq!(matching_language(["pt_br.UTF-8"], &available), Some("pt-BR"));
        assert_eq!(matching_language(["fr_FR@euro"], &available), Some("fr"));
        assert_eq!(matching_language(["de-DE", "ja"], &available), None);
        assert_eq!(matching_language([], &available), None);
    }

    #[test]
    fn the_test_language_is_never_the_systems() {
        assert_eq!(matching_language(["en-XA"], &["en", TEST_LANGUAGE]), Some("en"));
    }

    #[test]
    fn unknown_language_is_refused() {
        assert!(set_language("xx").is_err());
        assert_eq!(current_language(), ENGLISH);
    }
}
