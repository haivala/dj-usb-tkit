//! Turns the text catalog (`locales/<language>.ftl`) into Rust: one function per message of
//! the English catalog, in `i18n::tr`, returning an `i18n::Text`. A wrong key, a wrong argument
//! name or a missing argument then fails the build, at the call site.
//!
//! Also checks every catalog: it must parse cleanly, and another language may only have keys
//! English has and only use the arguments English gives the same key.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

use fluent_syntax::ast;

const ENGLISH: &str = "en";

/// Plural categories (CLDR) and the selector keys that mean a number is selected on.
const PLURAL_CATEGORIES: &[&str] = &["zero", "one", "two", "few", "many", "other"];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let locales_dir = manifest_dir.join("locales");
    println!("cargo::rerun-if-changed={}", locales_dir.display());

    let catalogs = read_catalogs(&locales_dir);
    let english = catalogs
        .iter()
        .find(|catalog| catalog.language == ENGLISH)
        .expect("locales/en.ftl is missing");
    let english_messages = messages(&english.resource);

    for catalog in catalogs
        .iter()
        .filter(|catalog| catalog.language != ENGLISH)
    {
        check_translation(catalog, &english_messages);
    }

    let mut out = String::new();
    write_languages(&mut out, &catalogs);
    write_functions(&mut out, &english_messages);

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_dir.join("i18n_catalog.rs"), out).expect("write i18n_catalog.rs");
}

struct Catalog {
    language: String,
    path: PathBuf,
    resource: ast::Resource<String>,
}

fn read_catalogs(locales_dir: &Path) -> Vec<Catalog> {
    let mut paths: Vec<PathBuf> = fs::read_dir(locales_dir)
        .expect("read locales/")
        .map(|entry| entry.expect("read locales/ entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ftl"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            println!("cargo::rerun-if-changed={}", path.display());
            let language = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("catalog file name")
                .to_string();
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
            let resource = fluent_syntax::parser::parse(source).unwrap_or_else(|(_, errors)| {
                panic!("{} doesn't parse: {errors:#?}", path.display());
            });
            if let Some(ast::Entry::Junk { content }) = resource
                .body
                .iter()
                .find(|entry| matches!(entry, ast::Entry::Junk { .. }))
            {
                panic!("{} has a line that isn't Fluent: {content}", path.display());
            }
            Catalog {
                language,
                path,
                resource,
            }
        })
        .collect()
}

/// A message with a value (attribute-only messages have none), and what its pattern needs.
struct MessageInfo<'a> {
    message: &'a ast::Message<String>,
    /// Variables in first-use order, with whether each is selected on as a number.
    variables: Vec<(String, bool)>,
}

fn messages(resource: &ast::Resource<String>) -> BTreeMap<String, MessageInfo<'_>> {
    let by_id: BTreeMap<&str, &ast::Message<String>> = resource
        .body
        .iter()
        .filter_map(|entry| match entry {
            ast::Entry::Message(message) => Some((message.id.name.as_str(), message)),
            _ => None,
        })
        .collect();
    by_id
        .iter()
        .filter(|(_, message)| message.value.is_some())
        .map(|(id, message)| {
            let mut variables = Vec::new();
            let mut seen = BTreeSet::new();
            collect_message(message, &by_id, &mut variables, &mut seen);
            (id.to_string(), MessageInfo { message, variables })
        })
        .collect()
}

fn collect_message<'a>(
    message: &'a ast::Message<String>,
    by_id: &BTreeMap<&str, &'a ast::Message<String>>,
    variables: &mut Vec<(String, bool)>,
    seen: &mut BTreeSet<&'a str>,
) {
    if !seen.insert(message.id.name.as_str()) {
        return;
    }
    if let Some(pattern) = &message.value {
        collect_pattern(pattern, by_id, variables, seen);
    }
}

fn collect_pattern<'a>(
    pattern: &'a ast::Pattern<String>,
    by_id: &BTreeMap<&str, &'a ast::Message<String>>,
    variables: &mut Vec<(String, bool)>,
    seen: &mut BTreeSet<&'a str>,
) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            collect_expression(expression, by_id, variables, seen);
        }
    }
}

fn collect_expression<'a>(
    expression: &'a ast::Expression<String>,
    by_id: &BTreeMap<&str, &'a ast::Message<String>>,
    variables: &mut Vec<(String, bool)>,
    seen: &mut BTreeSet<&'a str>,
) {
    match expression {
        ast::Expression::Select { selector, variants } => {
            let numeric = variants.iter().all(|variant| match &variant.key {
                ast::VariantKey::Identifier { name } => PLURAL_CATEGORIES.contains(&name.as_str()),
                ast::VariantKey::NumberLiteral { .. } => true,
            });
            if let ast::InlineExpression::VariableReference { id } = selector {
                add_variable(variables, &id.name, numeric);
            } else {
                collect_inline(selector, by_id, variables, seen);
            }
            for variant in variants {
                collect_pattern(&variant.value, by_id, variables, seen);
            }
        }
        ast::Expression::Inline(inline) => collect_inline(inline, by_id, variables, seen),
    }
}

fn collect_inline<'a>(
    inline: &'a ast::InlineExpression<String>,
    by_id: &BTreeMap<&str, &'a ast::Message<String>>,
    variables: &mut Vec<(String, bool)>,
    seen: &mut BTreeSet<&'a str>,
) {
    match inline {
        ast::InlineExpression::VariableReference { id } => add_variable(variables, &id.name, false),
        ast::InlineExpression::FunctionReference { id, arguments } => {
            let numeric = id.name == "NUMBER";
            for argument in &arguments.positional {
                if let ast::InlineExpression::VariableReference { id } = argument {
                    add_variable(variables, &id.name, numeric);
                } else {
                    collect_inline(argument, by_id, variables, seen);
                }
            }
        }
        // A referenced message is formatted with the same arguments.
        ast::InlineExpression::MessageReference {
            id,
            attribute: None,
        } => {
            if let Some(message) = by_id.get(id.name.as_str()) {
                collect_message(message, by_id, variables, seen);
            }
        }
        ast::InlineExpression::Placeable { expression } => {
            collect_expression(expression, by_id, variables, seen)
        }
        // Terms get their own arguments; literals and attribute references need none.
        _ => {}
    }
}

fn add_variable(variables: &mut Vec<(String, bool)>, name: &str, numeric: bool) {
    match variables.iter_mut().find(|(existing, _)| existing == name) {
        Some((_, existing_numeric)) => *existing_numeric |= numeric,
        None => variables.push((name.to_string(), numeric)),
    }
}

fn check_translation(catalog: &Catalog, english: &BTreeMap<String, MessageInfo<'_>>) {
    for (id, info) in messages(&catalog.resource) {
        let Some(english_info) = english.get(&id) else {
            panic!("{}: `{id}` isn't in en.ftl", catalog.path.display());
        };
        for (variable, _) in &info.variables {
            if !english_info
                .variables
                .iter()
                .any(|(name, _)| name == variable)
            {
                panic!(
                    "{}: `{id}` uses `${variable}`, which en.ftl doesn't give it",
                    catalog.path.display()
                );
            }
        }
    }
}

fn write_languages(out: &mut String, catalogs: &[Catalog]) {
    out.push_str("/// Each language's code and its catalog's source, English first.\n");
    out.push_str("pub(super) const LANGUAGE_SOURCES: &[(&str, &str)] = &[\n");
    let english_first = catalogs
        .iter()
        .filter(|catalog| catalog.language == ENGLISH)
        .chain(
            catalogs
                .iter()
                .filter(|catalog| catalog.language != ENGLISH),
        );
    for catalog in english_first {
        writeln!(
            out,
            "    ({:?}, include_str!({:?})),",
            catalog.language,
            catalog.path.display().to_string()
        )
        .expect("write");
    }
    out.push_str("];\n\n");
}

fn write_functions(out: &mut String, english: &BTreeMap<String, MessageInfo<'_>>) {
    out.push_str("/// One function per message in `locales/en.ftl`, named after its key.\n");
    out.push_str("pub mod tr {\n    use super::super::{Num, Text};\n");
    for (id, info) in english {
        out.push('\n');
        if let Some(comment) = &info.message.comment {
            for line in &comment.content {
                writeln!(out, "    /// {line}").expect("write");
            }
            out.push_str("    ///\n");
        }
        writeln!(out, "    /// English: `{}`", english_preview(info.message)).expect("write");
        let params: Vec<String> = info
            .variables
            .iter()
            .map(|(name, numeric)| {
                let ty = if *numeric {
                    "impl Into<Num>"
                } else {
                    "impl std::fmt::Display"
                };
                format!("{}: {ty}", rust_ident(name))
            })
            .collect();
        writeln!(
            out,
            "    pub fn {}({}) -> Text {{",
            rust_ident(&id.replace('-', "_")),
            params.join(", ")
        )
        .expect("write");
        let args: Vec<String> = info
            .variables
            .iter()
            .map(|(name, numeric)| {
                let value = if *numeric {
                    format!("super::super::Arg::Number({}.into().0)", rust_ident(name))
                } else {
                    format!("super::super::Arg::Text({}.to_string())", rust_ident(name))
                };
                format!("({name:?}, {value})")
            })
            .collect();
        writeln!(out, "        Text::new({id:?}, vec![{}])", args.join(", ")).expect("write");
        out.push_str("    }\n");
    }
    out.push_str("}\n");
}

/// The English text on one line, for the function's doc comment.
fn english_preview(message: &ast::Message<String>) -> String {
    let source = fluent_syntax::serializer::serialize(&ast::Resource {
        body: vec![ast::Entry::Message(ast::Message {
            id: message.id.clone(),
            value: message.value.clone(),
            attributes: Vec::new(),
            comment: None,
        })],
    });
    let value = source
        .split_once('=')
        .map_or(source.as_str(), |(_, value)| value);
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('`', "'")
}

fn rust_ident(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod",
        "move", "mut", "pub", "ref", "return", "static", "struct", "trait", "true", "type",
        "unsafe", "use", "where", "while", "yield",
    ];
    let name = name.replace('-', "_");
    if KEYWORDS.contains(&name.as_str()) {
        format!("r#{name}")
    } else {
        name
    }
}
