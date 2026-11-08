//! The JS inventory: every English string in the UI's JS, so none is added unnoticed while the
//! text moves to the catalog (USB_TEXT_CATALOG_PLAN.md, pieces 5 to 8).
//!
//! Each string literal and template literal in `vanilla-ui/` (not its tests, scripts or
//! build output) that looks like text must be listed in `vanilla-ui/tests/ui_text_inventory.txt`,
//! and each entry listed there must still be in its file. So new English fails here (new UI
//! text goes through `tr::` or `t`), and a piece that moves a string deletes its entry.
//!
//! The file's lines are `<to>\t<file>:<line>\t<text as a JSON string>`, under a `## piece N`
//! heading (`<to>` is `response` or `t`), `## not text` (`ignore`) or `## stays English`
//! (`english`). Matching is on file and text; the line only helps a reader.
//! `UPDATE_UI_TEXT_INVENTORY=1 cargo test -p backend --test ui_text_inventory` rewrites it:
//! gone entries dropped, lines refreshed, new strings added as `?`, which fails the check until
//! someone classifies them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A string or template literal: the line it starts on and its text. A template's
/// placeholders are `${…}`.
#[derive(Debug, Clone, PartialEq)]
struct Literal {
    line: usize,
    text: String,
    template: bool,
    /// The property it's assigned to (`textContent = "…"`, `title: "…"`), else the function
    /// whose call it's in (`counted(n, "track")`).
    context: String,
}

/// Just enough of a JS lexer to find string and template literals: skips comments and regular
/// expressions, and follows `${}` inside templates.
struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    out: &'a mut Vec<Literal>,
    /// The function name of each open `(`, innermost last ("" for a bracket that isn't a call).
    calls: Vec<String>,
    /// The last name, while nothing but whitespace followed it.
    prev_word: Option<String>,
    /// The property being assigned (`name =`, `name:`), until a `,`, `;` or closing bracket.
    property: Option<String>,
}

#[derive(Clone, PartialEq)]
enum Last {
    /// Start of input, an operator or an opening bracket: a `/` here starts a regex.
    Operator,
    /// A value, a name or a closing bracket: a `/` here divides.
    Value,
}

const REGEX_AFTER_WORDS: &[&str] = &[
    "return",
    "typeof",
    "case",
    "do",
    "else",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "yield",
    "await",
    "instanceof",
];

impl Lexer<'_> {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
        }
        Some(ch)
    }

    /// Lexes code until the input ends or, inside a template's `${`, its closing `}`.
    fn code(&mut self, in_placeholder: bool) {
        let mut last = Last::Operator;
        let mut depth = 0usize;
        while let Some(ch) = self.peek(0) {
            match ch {
                c if c.is_whitespace() => {
                    self.bump();
                }
                '/' if self.peek(1) == Some('/') => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                '/' if self.peek(1) == Some('*') => {
                    self.bump();
                    self.bump();
                    while self.peek(0).is_some()
                        && !(self.peek(0) == Some('*') && self.peek(1) == Some('/'))
                    {
                        self.bump();
                    }
                    self.bump();
                    self.bump();
                }
                '/' if last == Last::Operator => {
                    self.regex();
                    last = Last::Value;
                }
                '"' | '\'' => {
                    let line = self.line;
                    let context = self.context();
                    let text = self.string(ch);
                    self.out.push(Literal {
                        line,
                        text,
                        template: false,
                        context,
                    });
                    last = Last::Value;
                }
                '`' => {
                    self.template();
                    last = Last::Value;
                }
                '{' | '(' | '[' => {
                    if ch == '{' {
                        depth += 1;
                        self.property = None;
                    }
                    if ch == '(' {
                        let callee = self.prev_word.take().unwrap_or_default();
                        self.calls.push(callee);
                    }
                    self.prev_word = None;
                    self.bump();
                    last = Last::Operator;
                }
                '}' => {
                    if depth == 0 && in_placeholder {
                        self.bump();
                        return;
                    }
                    depth = depth.saturating_sub(1);
                    self.property = None;
                    self.prev_word = None;
                    self.bump();
                    // A block's end; a `/` after it starts a regex more often than it divides.
                    last = Last::Operator;
                }
                ')' | ']' => {
                    if ch == ')' {
                        self.calls.pop();
                    }
                    self.property = None;
                    self.prev_word = None;
                    self.bump();
                    last = Last::Value;
                }
                c if c.is_alphanumeric() || c == '_' || c == '$' => {
                    let mut word = String::new();
                    while let Some(c) = self
                        .peek(0)
                        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    {
                        word.push(c);
                        self.bump();
                    }
                    last = if REGEX_AFTER_WORDS.contains(&word.as_str()) {
                        Last::Operator
                    } else {
                        Last::Value
                    };
                    self.prev_word = Some(word);
                }
                _ => {
                    let assigns =
                        (ch == '=' && !matches!(self.peek(1), Some('=' | '>'))) || ch == ':';
                    if assigns && let Some(word) = self.prev_word.take() {
                        self.property = Some(word);
                    } else if matches!(ch, ',' | ';') {
                        self.property = None;
                    }
                    self.prev_word = None;
                    self.bump();
                    last = Last::Operator;
                }
            }
        }
    }

    fn escape(&mut self, out: &mut String) {
        match self.bump() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let mut hex = String::new();
                if self.peek(0) == Some('{') {
                    self.bump();
                    while let Some(c) = self.bump().filter(|c| *c != '}') {
                        hex.push(c);
                    }
                } else {
                    for _ in 0..4 {
                        hex.extend(self.bump());
                    }
                }
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some('\n') => {}
            Some(c) => out.push(c),
            None => {}
        }
    }

    fn string(&mut self, quote: char) -> String {
        self.bump();
        let mut text = String::new();
        while let Some(c) = self.bump() {
            match c {
                '\\' => self.escape(&mut text),
                c if c == quote => break,
                c => text.push(c),
            }
        }
        text
    }

    fn context(&self) -> String {
        self.property
            .clone()
            .or_else(|| self.calls.last().cloned())
            .unwrap_or_default()
    }

    fn template(&mut self) {
        let line = self.line;
        let context = self.context();
        self.bump();
        let mut text = String::new();
        while let Some(c) = self.bump() {
            match c {
                '\\' => self.escape(&mut text),
                '`' => break,
                '$' if self.peek(0) == Some('{') => {
                    self.bump();
                    text.push_str("${…}");
                    self.code(true);
                }
                c => text.push(c),
            }
        }
        self.out.push(Literal {
            line,
            text,
            template: true,
            context,
        });
    }

    fn regex(&mut self) {
        self.bump();
        let mut in_class = false;
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    self.bump();
                }
                '[' => in_class = true,
                ']' => in_class = false,
                '/' if !in_class => break,
                '\n' => break,
                _ => {}
            }
        }
        while self.peek(0).is_some_and(|c| c.is_ascii_alphabetic()) {
            self.bump();
        }
    }
}

fn literals(source: &str) -> Vec<Literal> {
    let mut out = Vec::new();
    Lexer {
        chars: source.chars().collect(),
        pos: 0,
        line: 1,
        out: &mut out,
        calls: Vec::new(),
        prev_word: None,
        property: None,
    }
    .code(false);
    out
}

/// Where any string with a word in it is text: the properties and functions that show it.
const TEXT_CONTEXTS: &[&str] = &[
    "textContent",
    "innerText",
    "title",
    "placeholder",
    "ariaLabel",
    "message",
    "label",
    "tooltip",
    "counted",
    "plural",
    "emitStatus",
    "setStatusText",
    "confirm",
    "alert",
];

/// Whether a literal reads as text a user sees, rather than code: a CSS class or selector, a
/// key, an event name, a path. Errs towards text; what isn't goes to the `not text` list.
fn looks_like_text(literal: &Literal) -> bool {
    // A catalog key, for `t` and `tCount`.
    if literal.text.starts_with("ui-")
        && literal
            .text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return false;
    }
    let text = literal.text.replace("${…}", " ");
    let trimmed = text.trim();
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let letter_words = words
        .iter()
        .filter(|word| word.chars().filter(char::is_ascii_alphabetic).count() >= 2)
        .count();
    if letter_words == 0 {
        return false;
    }
    let code_marks = [
        "#", "{", "}", "=", ";", "<", ">", "://", "--", "./", "[data-", "var(",
    ];
    if code_marks.iter().any(|mark| trimmed.contains(mark)) {
        return false;
    }
    // `name(` is a call or a CSS function; text puts a space before a bracket.
    if trimmed
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0].is_ascii_alphanumeric() && pair[1] == b'(')
    {
        return false;
    }
    if TEXT_CONTEXTS.contains(&literal.context.as_str()) {
        return true;
    }
    let sentence_end = ['.', '…', '!', '?', ':'];
    // A word beside a placeholder: `${count} selected`.
    let beside_placeholder = literal.template
        && literal.text.contains("${…}")
        && literal.text.contains(char::is_whitespace);
    if words.len() == 1 && !beside_placeholder {
        let word = words[0];
        let mut chars = word.chars();
        let capitalized = chars.next().is_some_and(|c| c.is_ascii_uppercase())
            && chars.clone().next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || sentence_end.contains(&c) || c == '\'');
        return capitalized || word.ends_with('…') || word.ends_with("...");
    }
    // Several words: CSS classes and selectors are lowercase, with a `-`, `_` or `.` somewhere.
    let lowercase_code = words.iter().all(|word| {
        word.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.,:*>+~%".contains(c))
    }) && trimmed.contains(['-', '_', '.']);
    !lowercase_code
}

/// The UI's JS files, relative to `vanilla-ui/`, sorted.
fn ui_files(ui_dir: &Path) -> Vec<PathBuf> {
    const SKIP: &[&str] = &[
        "node_modules",
        "dist",
        "tests",
        "scripts",
        "test-results",
        "assets",
        "playwright.config.mjs",
    ];
    let mut files = Vec::new();
    let mut dirs = vec![ui_dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("read vanilla-ui") {
            let path = entry.expect("dir entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if path.is_dir() {
                if !SKIP.contains(&name) && !name.starts_with('.') {
                    dirs.push(path);
                }
            } else if (name.ends_with(".mjs") || name.ends_with(".js")) && !SKIP.contains(&name) {
                files.push(path.strip_prefix(ui_dir).expect("inside").to_path_buf());
            }
        }
    }
    files.sort();
    files
}

/// Which piece of the plan moves a file's text.
fn piece(file: &str) -> &'static str {
    let starts = |prefixes: &[&str]| prefixes.iter().any(|p| file.starts_with(p));
    if starts(&["components/usb/", "components/backups/"]) {
        "5"
    } else if starts(&[
        "components/library/",
        "components/playlist/",
        "components/shared/",
        "track_table.mjs",
    ]) {
        "6"
    } else if starts(&[
        "components/track-detail/",
        "components/playback/",
        "waveform",
        "playback_ui_state.mjs",
    ]) {
        "7"
    } else {
        "8"
    }
}

/// One English string found in a file: (file, text) → first line.
type Hits = BTreeMap<(String, String), usize>;

fn hits_in(file: &str, source: &str, hits: &mut Hits) {
    for literal in literals(source) {
        if looks_like_text(&literal) {
            hits.entry((file.to_string(), literal.text))
                .or_insert(literal.line);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    to: String,
    file: String,
    line: usize,
    text: String,
}

const NOT_TEXT: &str = "not text";
const STAYS_ENGLISH: &str = "stays English";
/// Under a `## piece N` heading. `?` is only what update mode writes for a new string.
const TO_VALUES: &[&str] = &["response", "t", "?"];

/// Parses the inventory: `## <section>` headings, then `<to>\t<file>:<line>\t<json text>`.
fn parse_inventory(content: &str) -> Result<Vec<(String, Entry)>, String> {
    let mut section = String::new();
    let mut entries = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let at = || format!("inventory line {}", index + 1);
        if let Some(heading) = line.strip_prefix("## ") {
            section = heading.trim().to_string();
            continue;
        }
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(3, '\t');
        let (Some(to), Some(location), Some(text)) = (fields.next(), fields.next(), fields.next())
        else {
            return Err(format!("{}: expected three tab-separated fields", at()));
        };
        let (file, line_no) = location
            .rsplit_once(':')
            .ok_or_else(|| format!("{}: expected <file>:<line>", at()))?;
        let text: String = serde_json::from_str(text)
            .map_err(|err| format!("{}: text isn't a JSON string: {err}", at()))?;
        let allowed = match section.as_str() {
            NOT_TEXT => &["ignore"][..],
            STAYS_ENGLISH => &["english"][..],
            _ => TO_VALUES,
        };
        if !allowed.contains(&to) {
            return Err(format!(
                "{}: `{to}` isn't one of {allowed:?} under `## {section}`",
                at()
            ));
        }
        entries.push((
            section.clone(),
            Entry {
                to: to.to_string(),
                file: file.to_string(),
                line: line_no.parse().unwrap_or(0),
                text,
            },
        ));
    }
    Ok(entries)
}

/// What's wrong between the strings found and the inventory: new ones, and listed ones gone.
fn compare(hits: &Hits, inventory: &[(String, Entry)]) -> (Vec<String>, Vec<String>) {
    let listed: BTreeSet<(&str, &str)> = inventory
        .iter()
        .map(|(_, entry)| (entry.file.as_str(), entry.text.as_str()))
        .collect();
    let new = hits
        .iter()
        .filter(|((file, text), _)| !listed.contains(&(file.as_str(), text.as_str())))
        .map(|((file, text), line)| format!("{file}:{line}  {text:?}"))
        .collect();
    let gone = inventory
        .iter()
        .filter(|(_, entry)| !hits.contains_key(&(entry.file.clone(), entry.text.clone())))
        .map(|(_, entry)| format!("{}:{}  {:?}", entry.file, entry.line, entry.text))
        .collect();
    (new, gone)
}

const HEADER: &str = "\
# The JS inventory: every English string in the UI's JS. Checked by
# backend/tests/ui_text_inventory.rs; see that file for the format.
#
# to: `response` (composed or counted text, or text picked by a decision on response data:
# the response carries it, from a `tr::` function), `t` (a fixed label, or text shown before
# any response; `tCount` with a count). `?` is a new string nobody has classified yet.
# `## not text`: key names, codes, product names, CSS. `## stays English`: what a developer
# reads (console messages, internal errors, runtime errors in the Event Log) and data written
# to the stick. A piece that moves a string deletes its entry.
";

fn render_inventory(hits: &Hits, inventory: &[(String, Entry)]) -> String {
    let known: BTreeMap<(&str, &str), (&str, &str)> = inventory
        .iter()
        .map(|(section, entry)| {
            (
                (entry.file.as_str(), entry.text.as_str()),
                (section.as_str(), entry.to.as_str()),
            )
        })
        .collect();
    // By section, then file and line.
    let mut sections: BTreeMap<String, BTreeMap<(&str, usize, &str), String>> = BTreeMap::new();
    for ((file, text), line) in hits {
        let (section, to) = match known.get(&(file.as_str(), text.as_str())) {
            Some((section, to)) => (section.to_string(), to.to_string()),
            None => (format!("piece {}", piece(file)), "?".to_string()),
        };
        let json = serde_json::to_string(text).expect("serialize");
        sections
            .entry(section)
            .or_default()
            .insert((file, *line, text), format!("{to}\t{file}:{line}\t{json}"));
    }
    let mut out = HEADER.to_string();
    for (section, lines) in &sections {
        write!(out, "\n## {section}\n").expect("write");
        for line in lines.values() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn every_english_string_in_the_ui_js_is_in_the_inventory() {
    let ui_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../vanilla-ui");
    let inventory_path = ui_dir.join("tests/ui_text_inventory.txt");
    let mut hits = Hits::new();
    for file in ui_files(&ui_dir) {
        let source = std::fs::read_to_string(ui_dir.join(&file)).expect("read UI file");
        hits_in(
            &file.to_string_lossy().replace('\\', "/"),
            &source,
            &mut hits,
        );
    }
    let content = std::fs::read_to_string(&inventory_path).unwrap_or_default();
    let inventory = parse_inventory(&content).unwrap_or_else(|err| panic!("{err}"));
    if std::env::var_os("UPDATE_UI_TEXT_INVENTORY").is_some() {
        std::fs::write(&inventory_path, render_inventory(&hits, &inventory))
            .expect("write inventory");
        return;
    }
    let (new, gone) = compare(&hits, &inventory);
    assert!(
        new.is_empty(),
        "English in the UI's JS that isn't in {}: new UI text goes through the catalog (`t`, or a \
         response field from `tr::`). Not text? Add it under `## not text`.\n{}",
        inventory_path.display(),
        new.join("\n")
    );
    assert!(
        gone.is_empty(),
        "No longer in the UI's JS: delete from {} (or rewrite it with UPDATE_UI_TEXT_INVENTORY=1)\n{}",
        inventory_path.display(),
        gone.join("\n")
    );
    let unclassified: Vec<String> = inventory
        .iter()
        .filter(|(_, entry)| entry.to == "?")
        .map(|(_, entry)| format!("{}:{}  {:?}", entry.file, entry.line, entry.text))
        .collect();
    assert!(
        unclassified.is_empty(),
        "Not classified in {}: mark each `response` or `t`, or move it under `## not text` or \
         `## stays English`\n{}",
        inventory_path.display(),
        unclassified.join("\n")
    );
}

#[cfg(test)]
mod checks {
    use super::*;

    fn texts(source: &str) -> Vec<String> {
        literals(source)
            .into_iter()
            .map(|literal| literal.text)
            .collect()
    }

    #[test]
    fn finds_strings_and_templates_past_comments_and_regexes() {
        let source = r#"
// "in a comment"
/* 'also' */
const a = "Double" + 'single';
const b = s.replace(/["']/g, "") / 2;
const c = `Adding ${count} to ${`inner ${x}`}`;
return /a"b/.test(c);
"#;
        assert_eq!(
            texts(source),
            ["Double", "single", "", "inner ${…}", "Adding ${…} to ${…}"]
        );
        assert_eq!(literals("\n\nconst a = 'x';")[0].line, 3);
    }

    #[test]
    fn tells_text_from_code() {
        let literal = |s: &str| Literal {
            line: 1,
            text: s.to_string(),
            template: s.contains("${…}"),
            context: String::new(),
        };
        let text = |s: &str| looks_like_text(&literal(s));
        for s in [
            "Cancel",
            "Exporting ${…} to USB...",
            "${…} selected",
            "Please wait…",
            "drag to reorder",
            "Remove (2)",
        ] {
            assert!(text(s), "{s:?} is text");
        }
        // Lowercase single words, which are code anywhere else.
        let found = |source: &str| {
            literals(source)
                .into_iter()
                .filter(looks_like_text)
                .map(|literal| literal.text)
                .collect::<Vec<_>>()
        };
        assert_eq!(found("x(counted(n, 'track'), 'click')"), ["track"]);
        assert!(found("tCount('ui-common-selection-count', n)").is_empty());
        assert_eq!(
            found("el.textContent = 'none'; el.className = 'none'"),
            ["none"]
        );
        assert_eq!(found("f({ title: 'empty', kind: 'empty' })"), ["empty"]);
        for s in [
            "click",
            "nav-item active",
            "#usbRoot .x",
            "[data-text]",
            "translate(${…}px, 0)",
            "./ui.mjs",
            "https://example.com",
            "1px solid var(--line)",
            "ArrowUp",
            "a",
            "MP3",
            "${…}px",
        ] {
            assert!(!text(s), "{s:?} is code");
        }
    }

    fn inventory(lines: &str) -> Vec<(String, Entry)> {
        parse_inventory(lines).expect("parses")
    }

    #[test]
    fn a_new_string_fails_and_a_listed_one_passes() {
        let listed = inventory("## piece 5\nt\tusb.mjs:3\t\"Please wait…\"\n");
        let mut hits = Hits::new();
        hits_in("usb.mjs", "x = 'Please wait…';", &mut hits);
        assert_eq!(compare(&hits, &listed), (vec![], vec![]));

        hits_in("usb.mjs", "\ny = `Exporting ${name} to USB...`;", &mut hits);
        let (new, gone) = compare(&hits, &listed);
        assert_eq!(new, ["usb.mjs:2  \"Exporting ${…} to USB...\""]);
        assert!(gone.is_empty());
    }

    #[test]
    fn a_listed_string_that_is_gone_fails() {
        let listed = inventory(
            "## piece 5\nt\tusb.mjs:3\t\"Please wait…\"\n## not text\nignore\tusb.mjs:9\t\"Escape\"\n",
        );
        let (new, gone) = compare(&Hits::new(), &listed);
        assert!(new.is_empty());
        assert_eq!(gone.len(), 2);
    }

    #[test]
    fn the_inventory_format_is_checked() {
        assert!(parse_inventory("## not text\nt\ta.mjs:1\t\"x\"\n").is_err());
        assert!(parse_inventory("## stays English\nignore\ta.mjs:1\t\"x\"\n").is_err());
        assert!(parse_inventory("## stays English\nenglish\ta.mjs:1\t\"x\"\n").is_ok());
        assert!(parse_inventory("## piece 6\nmaybe\ta.mjs:1\t\"x\"\n").is_err());
        assert!(parse_inventory("## piece 6\nt\ta.mjs:1\tnot json\n").is_err());
    }
}
