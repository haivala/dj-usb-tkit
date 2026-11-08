//! The text catalog: every user-facing text, by key, in each language the app has.
//!
//! The catalogs are `locales/<language>.ftl` (Fluent), embedded in the binary. `build.rs`
//! turns each message of `locales/en.ftl` into a function in [`tr`], named after its key
//! (`usb-warning-count` is `tr::usb_warning_count(count)`), so a wrong key, a wrong argument
//! or a missing one fails the build.
//!
//! A [`Text`] is a key and its arguments. It becomes text only when shown: [`Text::render`]
//! (or `Display`) in the process's current language, [`Text::render_english`] for logs. The
//! current language is one per process, readable from any thread; with nothing set, as in
//! tests and the CLI tools, it's English. A key missing from another language falls back to
//! English.

use std::fmt;
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

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
    Text(String),
    Number(f64),
}

/// A number argument: any integer or float. Counts are what plural forms select on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Num(pub f64);

macro_rules! num_from {
    ($($ty:ty)+) => {
        $(impl From<$ty> for Num {
            fn from(value: $ty) -> Self {
                Num(value as f64)
            }
        })+
    };
}
num_from!(u8 u16 u32 u64 usize i8 i16 i32 i64 isize f32 f64);

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

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// One language's parsed catalog.
struct Catalog {
    language: &'static str,
    bundle: FluentBundle<FluentResource>,
}

impl Catalog {
    fn parse(language: &'static str, source: &str) -> Result<Self, String> {
        let id: LanguageIdentifier = language
            .parse()
            .map_err(|err| format!("`{language}` isn't a language code: {err}"))?;
        let resource = FluentResource::try_new(source.to_string())
            .map_err(|(_, errors)| format!("the `{language}` catalog doesn't parse: {errors:?}"))?;
        let mut bundle = FluentBundle::new_concurrent(vec![id]);
        // Fluent puts invisible bidi isolation marks around every argument by default.
        bundle.set_use_isolating(false);
        bundle.add_resource(resource).map_err(|errors| {
            format!("the `{language}` catalog has a duplicate key: {errors:?}")
        })?;
        Ok(Self { language, bundle })
    }

    /// The text in this catalog's language, or `None` when the key isn't translated.
    fn format(&self, text: &Text) -> Option<String> {
        let pattern = self.bundle.get_message(text.key)?.value()?;
        let mut args = FluentArgs::new();
        for (name, arg) in &text.args {
            let value = match arg {
                Arg::Text(value) => FluentValue::from(value.as_str()),
                Arg::Number(value) => FluentValue::from(*value),
            };
            args.set(*name, value);
        }
        let mut errors = Vec::new();
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

#[cfg(test)]
mod tests {
    use super::*;

    const ISOLATION_MARKS: [char; 4] = ['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'];

    // Another language, for fallback and plural tests without changing the process's language
    // (other tests in this process assert English).
    const OTHER_SOURCE: &str = "\
fixture-hello = Hei { $name }!
usb-warning-count = { $count ->
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
        let english = |count: usize| tr::usb_warning_count(count).render_english();
        assert_eq!(english(0), "0 warnings");
        assert_eq!(english(1), "1 warning");
        assert_eq!(english(2), "2 warnings");
    }

    #[test]
    fn placeholders_are_filled_without_isolation_marks() {
        let catalog = other_catalog();
        let text = Text::new("fixture-hello", vec![("name", Arg::Text("Example".into()))]);
        let rendered = catalog.format(&text).expect("fixture key");
        assert_eq!(rendered, "Hei Example!");
        let plural = tr::usb_warning_count(3usize).render_in(&catalog);
        assert_eq!(plural, "3 varoitusta");
        for output in [
            rendered,
            plural,
            tr::usb_warning_count(2usize).render_english(),
        ] {
            assert!(!output.contains(ISOLATION_MARKS), "{output:?}");
        }
    }

    #[test]
    fn a_key_missing_from_another_language_falls_back_to_english() {
        let catalog = Catalog::parse("fi", "fixture-hello = Hei!\n").expect("fixture parses");
        assert_eq!(
            tr::usb_warning_count(1usize).render_in(&catalog),
            "1 warning"
        );
    }

    #[test]
    fn unset_language_is_english() {
        // Nothing in the unit tests sets a language; piece 2's own test file covers switching.
        assert_eq!(current_language(), ENGLISH);
        assert_eq!(tr::usb_warning_count(2usize).to_string(), "2 warnings");
    }

    #[test]
    fn unknown_language_is_refused() {
        assert!(set_language("xx").is_err());
        assert_eq!(current_language(), ENGLISH);
    }
}
