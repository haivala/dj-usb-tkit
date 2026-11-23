//! The Tauri commands the desktop app registers and the ones the UI calls must match. A
//! command the UI stopped calling stays registered (and callable) unnoticed otherwise, and a
//! call to a name that isn't registered only fails at runtime.
//!
//! Registered: every entry of `generate_handler![…]` in `desktop/src-tauri/src/main.rs`.
//! Called: a registered name counts as used when the UI's JS has it as a quoted string (some
//! calls pass the name through a variable); every `command("…")`/`invoke("…")` literal must be
//! registered. Plugin commands (`plugin:…`) belong to their plugins and are skipped.

mod common;

use common::ui_files;
use std::collections::BTreeSet;
use std::path::Path;

fn registered_commands() -> BTreeSet<String> {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("../desktop/src-tauri/src/main.rs");
    let source = std::fs::read_to_string(&main).expect("read desktop main.rs");
    let start = source
        .find("generate_handler![")
        .expect("generate_handler! in main.rs")
        + "generate_handler![".len();
    let end = start + source[start..].find(']').expect("end of generate_handler!");
    source[start..end]
        .split(',')
        .map(|entry| entry.trim().rsplit("::").next().unwrap_or_default())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// Names in `command("…")` / `invoke("…")` calls with a literal first argument.
fn literal_calls(source: &str, out: &mut BTreeSet<String>) {
    for callee in ["command(", "invoke("] {
        for (at, _) in source.match_indices(callee) {
            let before = source[..at].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$') {
                continue;
            }
            let rest = source[at + callee.len()..].trim_start();
            let Some(quote) = rest
                .chars()
                .next()
                .filter(|c| matches!(c, '"' | '\'' | '`'))
            else {
                continue;
            };
            let Some(len) = rest[1..].find(quote) else {
                continue;
            };
            let name = &rest[1..1 + len];
            if !name.starts_with("plugin:") {
                out.insert(name.to_string());
            }
        }
    }
}

#[test]
fn registered_tauri_commands_match_the_ui_calls() {
    let registered = registered_commands();
    assert!(
        !registered.is_empty(),
        "no commands parsed from generate_handler!"
    );

    let ui_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../vanilla-ui");
    let mut sources = String::new();
    let mut called = BTreeSet::new();
    for file in ui_files(&ui_dir) {
        let source = std::fs::read_to_string(ui_dir.join(&file)).expect("read UI file");
        literal_calls(&source, &mut called);
        sources.push_str(&source);
        sources.push('\n');
    }

    let unused: Vec<_> = registered
        .iter()
        .filter(|name| {
            !['"', '\'', '`']
                .iter()
                .any(|q| sources.contains(&format!("{q}{name}{q}")))
        })
        .collect();
    let unregistered: Vec<_> = called.difference(&registered).collect();
    assert!(
        unused.is_empty() && unregistered.is_empty(),
        "Registered in generate_handler! but never called from the UI (remove the command and \
         its registration, or call it):\n{unused:#?}\nCalled from the UI but not registered in \
         generate_handler! (register the command or fix the name):\n{unregistered:#?}"
    );
}

/// Every registered command is classified in `job_conflicts::command_block`: either a running
/// job can block it (its action group) or none ever does. A new command must be added there,
/// so the UI and the invoke handler know what blocks it.
#[test]
fn every_registered_command_says_what_blocks_it() {
    let unclassified: Vec<_> = registered_commands()
        .into_iter()
        .filter(|name| backend::job_conflicts::command_block(name).is_none())
        .collect();
    assert!(
        unclassified.is_empty(),
        "Commands missing from job_conflicts::command_block (add each to its action group, \
         or to the commands no job blocks):\n{unclassified:#?}"
    );
}
