//! Where node, the essentia analysis runner and its node modules are, as the app
//! resolves them at startup or after installing the engine. Kept in memory
//! instead of the process environment: writing the environment while other
//! threads read it is undefined behaviour on Linux (why `std::env::set_var` is
//! `unsafe`). The node process gets the modules path through its own
//! environment ([`apply_node_modules`]); the app's stays untouched.
//!
//! A value set in the environment before the app starts still counts when
//! nothing has been stored.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{PoisonError, RwLock};

const NODE_ENV: &str = "DJTKIT_ESSENTIA_NODE";
const RUNNER_ENV: &str = "DJTKIT_ESSENTIA_RUNNER";
/// Read by `desktop/scripts/essentia_runner.cjs`.
const NODE_MODULES_ENV: &str = "DJTKIT_ESSENTIA_NODE_MODULES";

static NODE: RwLock<Option<PathBuf>> = RwLock::new(None);
static RUNNER: RwLock<Option<PathBuf>> = RwLock::new(None);
static NODE_MODULES: RwLock<Option<PathBuf>> = RwLock::new(None);

fn stored_or_env(slot: &RwLock<Option<PathBuf>>, env: &str) -> Option<PathBuf> {
    slot.read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .or_else(|| std::env::var_os(env).map(PathBuf::from))
}

fn store(slot: &RwLock<Option<PathBuf>>, path: PathBuf) {
    *slot.write().unwrap_or_else(PoisonError::into_inner) = Some(path);
}

/// The node binary the app found (bundled runtime), if any.
pub fn node() -> Option<PathBuf> {
    stored_or_env(&NODE, NODE_ENV)
}

pub fn set_node(path: PathBuf) {
    store(&NODE, path);
}

/// The node binary to run the essentia runner with: the one found, else
/// `node` from `PATH`.
pub fn node_bin() -> PathBuf {
    node().unwrap_or_else(|| PathBuf::from("node"))
}

/// The essentia runner script (`essentia_runner.cjs`), if known.
pub fn runner() -> Option<PathBuf> {
    stored_or_env(&RUNNER, RUNNER_ENV)
}

pub fn set_runner(path: PathBuf) {
    store(&RUNNER, path);
}

/// The `node_modules` folder holding `essentia.js`, if known.
pub fn node_modules() -> Option<PathBuf> {
    stored_or_env(&NODE_MODULES, NODE_MODULES_ENV)
}

pub fn set_node_modules(path: PathBuf) {
    store(&NODE_MODULES, path);
}

/// Hand the runner `command` starts the node modules path, when one is known.
pub fn apply_node_modules(command: &mut Command) {
    if let Some(path) = node_modules() {
        command.env(NODE_MODULES_ENV, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn env_of<'a>(command: &'a Command, key: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(k, _)| *k == OsStr::new(key))
            .map(|(_, v)| v)
    }

    #[test]
    fn stored_paths_reach_the_runner_command() {
        // One test: the statics are process-wide.
        let mut command = Command::new("node");
        apply_node_modules(&mut command);
        if std::env::var_os(NODE_MODULES_ENV).is_none() {
            assert_eq!(env_of(&command, NODE_MODULES_ENV), None);
        }

        set_node_modules(PathBuf::from("/data/essentia/node_modules"));
        set_runner(PathBuf::from("/app/scripts/essentia_runner.cjs"));
        set_node(PathBuf::from("/app/bin/node"));
        assert_eq!(node_bin(), PathBuf::from("/app/bin/node"));
        assert_eq!(
            node_modules(),
            Some(PathBuf::from("/data/essentia/node_modules"))
        );
        assert_eq!(
            runner(),
            Some(PathBuf::from("/app/scripts/essentia_runner.cjs"))
        );
        let mut command = Command::new("node");
        apply_node_modules(&mut command);
        assert_eq!(
            env_of(&command, NODE_MODULES_ENV),
            Some(Some(OsStr::new("/data/essentia/node_modules")))
        );
    }
}
