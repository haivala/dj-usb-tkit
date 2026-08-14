//! Local HDD staging for the USB-resident `export.pdb` / `exportLibrary.db`
//! files: read through a local working copy (re-synced only when the USB
//! source's size/mtime changes), and write back to USB only when the local
//! copy actually changed, via an atomic (temp-file + rename) commit.
//!
//! This module has no opinion about backups -- callers that want a
//! pre-write backup must take it themselves, before calling into any write
//! path here (see `commit_local_to_usb`'s doc comment for why: by the time
//! a write-back runs, in some call shapes the write has already happened).
//!
//! Staging is opt-in per process via [`init_cache_root`]. Until that is
//! called (CLI dev tools, tests that don't construct a [`super::BackendService`]),
//! every function here degrades to operating directly on the USB path --
//! today's behavior, unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::error::{BackendError, BackendResult};

use super::normalize_source_root_for_matching;
use super::usb_vendor_compat::{USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR};

static CACHE_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Guards the read-modify-write critical sections below (stat, copy/write,
/// sidecar update) so concurrent staging/write-back calls -- even across
/// different USB devices -- can't interleave and corrupt a sidecar file.
static STATE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DbKind {
    Pdb,
    Edb,
}

impl DbKind {
    fn filename(self) -> &'static str {
        match self {
            DbKind::Pdb => "export.pdb",
            DbKind::Edb => "exportLibrary.db",
        }
    }

    fn state_key(self) -> &'static str {
        self.filename()
    }
}

/// Enables local HDD staging by pointing it at a directory (created lazily,
/// on first use, alongside `PIONEER/rekordbox/...` subpaths per USB device).
/// `CACHE_ROOT` is a process-wide static, so this must be called at most
/// once per real, long-lived process -- **only the desktop app** does this,
/// from `desktop/src-tauri/src/main.rs` right after constructing its one
/// `BackendCommands`. It is deliberately NOT called from
/// `BackendService::new`/`BackendCommands::new` themselves: those
/// constructors are shared by the real app, `cargo test --lib`'s unit
/// tests, and `backend/tests/*.rs` integration tests alike, and the latter
/// two construct many short-lived `BackendService` instances per process
/// (often concurrently, since `cargo test` runs tests in parallel by
/// default) -- auto-initializing there would have each such instance
/// silently repoint (and, once its own tempdir is dropped, invalidate)
/// global staging state for every other test in the same binary. Tests that
/// specifically want to exercise staging opt in explicitly and locally via
/// the `_with_root`/`_for_test` functions below, or -- only where routing
/// through the real global is itself what's being verified -- via
/// [`set_cache_root_for_test`] under [`TEST_LOCK`].
pub fn init_cache_root(data_dir: &Path) {
    *CACHE_ROOT.lock().unwrap() = Some(data_dir.join("usb_cache"));
}

// `cargo test`'s default harness runs each test on a worker thread from a
// pool, potentially reusing a thread across multiple tests over the run, but
// never runs two tests *concurrently* on the same thread. A per-thread
// override therefore gives a test exclusive, contention-free control over
// what `cache_root()` reports *for that thread* -- unrelated tests running
// concurrently on other threads are structurally unaffected, no locking
// needed. `CacheRootOverrideGuard`'s `Drop` clears the override
// unconditionally (including on panic/early-return), so a leftover override
// can never bleed into whatever test the harness schedules next on the same
// reused thread.
#[cfg(test)]
thread_local! {
    static CACHE_ROOT_OVERRIDE: std::cell::RefCell<Option<Option<PathBuf>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct CacheRootOverrideGuard {
    _private: (),
}

#[cfg(test)]
impl Drop for CacheRootOverrideGuard {
    fn drop(&mut self) {
        CACHE_ROOT_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
    }
}

#[cfg(test)]
pub(crate) fn set_cache_root_for_test(dir: Option<PathBuf>) -> CacheRootOverrideGuard {
    CACHE_ROOT_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(dir));
    CacheRootOverrideGuard { _private: () }
}

fn cache_root() -> Option<PathBuf> {
    #[cfg(test)]
    {
        if let Some(override_value) =
            CACHE_ROOT_OVERRIDE.with(|cell| cell.borrow().clone())
        {
            return override_value;
        }
    }
    CACHE_ROOT.lock().unwrap().clone()
}

/// A named drive's cache key is its slugged name, so the same physical drive
/// reuses the same cache dir regardless of which port/mount path the OS
/// assigns it this time. Unnamed drives (not yet through the naming prompt,
/// or a test's arbitrary temp dir) fall back to the old mount-path hash,
/// prefixed so the two key spaces can never collide.
fn cache_key_for_root(usb_root: &Path) -> String {
    if let Some(name) = super::usb_identity::read_drive_name(usb_root) {
        return super::usb_identity::slug(&name);
    }

    use std::hash::{Hash, Hasher};
    let normalized = normalize_source_root_for_matching(&usb_root.to_string_lossy());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    format!("unnamed-{:016x}", hasher.finish())
}

fn device_dir_for(cache_root: Option<&Path>, usb_root: &Path) -> Option<PathBuf> {
    cache_root.map(|root| root.join(cache_key_for_root(usb_root)))
}

fn local_vendor_db_dir(device_dir: &Path) -> PathBuf {
    device_dir.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)
}

/// The HDD-cache-side `backups/` dir for `usb_root`'s device (mirrors the
/// USB-side `PIONEER/rekordbox/backups/` layout), or `None` when staging is
/// disabled (no cache root configured) or the drive has no stable cache
/// directory yet (see `cache_key_for_root`'s unnamed-drive fallback --
/// archiving into a key that will change once the drive is later named
/// would just orphan the archive, so callers should treat `None` here as
/// "don't archive yet").
pub(crate) fn backups_dir_for(usb_root: &Path) -> Option<PathBuf> {
    // Require a name, not just a configured cache root: an unnamed drive's
    // cache key is the unstable mount-path hash (see `cache_key_for_root`),
    // so archiving into it now would just orphan the archive once the drive
    // is later named and its cache key changes.
    super::usb_identity::read_drive_name(usb_root)?;
    device_dir_for(cache_root().as_deref(), usb_root)
        .map(|device_dir| local_vendor_db_dir(&device_dir).join("backups"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct FileStat {
    size: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
}

fn stat(path: &Path) -> std::io::Result<FileStat> {
    let meta = std::fs::metadata(path)?;
    let mtime = meta.modified()?;
    let dur = mtime.duration_since(UNIX_EPOCH).unwrap_or_default();
    Ok(FileStat {
        size: meta.len(),
        mtime_secs: dur.as_secs(),
        mtime_nanos: dur.subsec_nanos(),
    })
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SyncedState {
    source: FileStat,
    local: FileStat,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StagingState {
    #[serde(default)]
    files: HashMap<String, SyncedState>,
}

fn state_path(device_dir: &Path) -> PathBuf {
    local_vendor_db_dir(device_dir).join(".staging_state.json")
}

/// Sidecar corruption/absence is a cache miss, never a hard error -- staging
/// must never block a read/write just because its bookkeeping file is bad.
fn load_state(device_dir: &Path) -> StagingState {
    std::fs::read(state_path(device_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_state(device_dir: &Path, state: &StagingState) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state)?;
    std::fs::write(state_path(device_dir), bytes)
}

/// Stat-compare-and-copy-if-needed for one file. Returns the local path to
/// read/write through. When staging is disabled (`cache_root` is `None`)
/// returns `source` unchanged -- no copy, no sidecar, today's behavior.
///
/// Takes `cache_root` explicitly (rather than reading the process-global
/// directly) so tests can exercise the "enabled" path with a private,
/// per-test root -- never mutating global state, so they're safe to run
/// concurrently with the rest of the suite. [`stage_pdb`]/[`stage_edb`] are
/// the thin global-reading wrappers production code actually calls.
fn stage_file_with_root(
    cache_root: Option<&Path>,
    usb_root: &Path,
    source: &Path,
    kind: DbKind,
) -> BackendResult<PathBuf> {
    let Some(device_dir) = device_dir_for(cache_root, usb_root) else {
        return Ok(source.to_path_buf());
    };
    let local_dir = local_vendor_db_dir(&device_dir);
    let local_path = local_dir.join(kind.filename());

    if !source.exists() {
        // Nothing to stage yet (e.g. fresh USB, PDB not created). Callers
        // already handle "file not present" via their own existence checks.
        return Ok(local_path);
    }

    let _guard = STATE_LOCK.lock().unwrap();
    let device_dir_is_new = !device_dir.is_dir();
    std::fs::create_dir_all(&local_dir)?;
    if device_dir_is_new {
        crate::logging::emit(
            crate::logging::Level::Info,
            "usb-staging",
            &format!("USB local staging folder: {}", device_dir.display()),
        );
    }

    let source_stat = stat(source)?;
    let state = load_state(&device_dir);
    let reuse = local_path.is_file()
        && state
            .files
            .get(kind.state_key())
            .is_some_and(|s| s.source == source_stat);

    if reuse {
        return Ok(local_path);
    }

    std::fs::copy(source, &local_path)?;
    crate::logging::emit(
        crate::logging::Level::Info,
        "usb-staging",
        &format!("Staged {} locally: {}", kind.filename(), local_path.display()),
    );
    let local_stat = stat(&local_path)?;
    let mut state = state;
    state.files.insert(
        kind.state_key().to_string(),
        SyncedState {
            source: source_stat,
            local: local_stat,
        },
    );
    // Best-effort: if we can't persist the sidecar, the next call will just
    // treat this as a fresh miss and re-copy -- never a hard failure.
    let _ = save_state(&device_dir, &state);

    Ok(local_path)
}

pub(crate) fn stage_pdb(usb_root: &Path) -> BackendResult<PathBuf> {
    stage_file_with_root(
        cache_root().as_deref(),
        usb_root,
        &super::usb_vendor_compat::vendor_pdb_path(usb_root),
        DbKind::Pdb,
    )
}

pub(crate) fn stage_edb(usb_root: &Path) -> BackendResult<PathBuf> {
    stage_file_with_root(
        cache_root().as_deref(),
        usb_root,
        &super::usb_vendor_compat::vendor_edb_path(usb_root),
        DbKind::Edb,
    )
}

#[cfg(test)]
pub(crate) fn stage_edb_with_root(cache_root: Option<&Path>, usb_root: &Path) -> BackendResult<PathBuf> {
    stage_file_with_root(
        cache_root,
        usb_root,
        &super::usb_vendor_compat::vendor_edb_path(usb_root),
        DbKind::Edb,
    )
}

fn vendor_path(usb_root: &Path, kind: DbKind) -> PathBuf {
    match kind {
        DbKind::Pdb => super::usb_vendor_compat::vendor_pdb_path(usb_root),
        DbKind::Edb => super::usb_vendor_compat::vendor_edb_path(usb_root),
    }
}

/// Core write-back: given the local file's current bytes are the desired
/// new content, atomically commit to USB, then record the new synced state.
/// Called by both write-back entry points below once they've decided a
/// commit is actually needed.
///
/// Does NOT back up the USB file -- backup, if wanted, is the *caller's*
/// responsibility, taken before any write is attempted at all (including
/// the staging-disabled direct write below). It can't live here: for the
/// eDB path in particular, the write already happened (via a SQL
/// transaction commit through a connection opened earlier) by the time this
/// runs, so "back up right before committing" would capture the already-new
/// content, not the pre-write state a backup is supposed to preserve.
fn commit_local_to_usb(
    usb_root: &Path,
    device_dir: &Path,
    kind: DbKind,
    local_path: &Path,
) -> BackendResult<()> {
    let dest = vendor_path(usb_root, kind);

    // External-modification guard: if the USB source has moved since we last
    // synced with it, don't silently clobber whatever changed it.
    let state = load_state(device_dir);
    if let Some(recorded) = state.files.get(kind.state_key())
        && dest.exists()
        && let Ok(current) = stat(&dest)
        && current != recorded.source
    {
        return Err(BackendError::Internal(format!(
            "USB database changed since it was last read ({}) -- re-open and retry to avoid discarding the external change",
            dest.display()
        )));
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::copy(local_path, &tmp)?;
    std::fs::rename(&tmp, &dest)?;
    crate::logging::emit(
        crate::logging::Level::Info,
        "usb-staging",
        &format!("Wrote {} back to USB: {}", kind.filename(), dest.display()),
    );

    let local_stat = stat(local_path)?;
    let dest_stat = stat(&dest)?;
    let mut state = load_state(device_dir);
    state.files.insert(
        kind.state_key().to_string(),
        SyncedState {
            source: dest_stat,
            local: local_stat,
        },
    );
    let _ = save_state(device_dir, &state);

    Ok(())
}

/// Write-back for byte-buffer writers (PDB): `local_bytes` are the exact
/// bytes to commit. Always commits when staging is active -- callers
/// already gate whether to write at all (e.g. `preview_pdb` vs `write_pdb`)
/// before calling this.
///
/// When staging is disabled, writes `local_bytes` straight to the
/// USB-mounted file (today's pre-staging behavior).
///
/// Backup is the caller's responsibility -- see `commit_local_to_usb`'s doc
/// comment for why it can't live here.
pub(crate) fn commit_and_write_back(
    usb_root: &Path,
    kind: DbKind,
    local_bytes: &[u8],
) -> BackendResult<bool> {
    commit_and_write_back_with_root(cache_root().as_deref(), usb_root, kind, local_bytes)
}

fn commit_and_write_back_with_root(
    cache_root: Option<&Path>,
    usb_root: &Path,
    kind: DbKind,
    local_bytes: &[u8],
) -> BackendResult<bool> {
    let Some(device_dir) = device_dir_for(cache_root, usb_root) else {
        std::fs::write(vendor_path(usb_root, kind), local_bytes)?;
        return Ok(true);
    };
    let local_dir = local_vendor_db_dir(&device_dir);
    let local_path = local_dir.join(kind.filename());
    let _guard = STATE_LOCK.lock().unwrap();
    std::fs::create_dir_all(&local_dir)?;
    std::fs::write(&local_path, local_bytes)?;
    commit_local_to_usb(usb_root, &device_dir, kind, &local_path)?;
    Ok(true)
}

/// Write-back for connection-based writers (eDB): no single "new bytes"
/// buffer exists once a `rusqlite::Connection` commits, so this compares
/// the local file's current stat against the last-known synced state and
/// only commits back to USB if the local copy actually changed.
///
/// When staging is disabled, the write already landed directly on the USB
/// path (the connection was opened there in the first place), so this is a
/// no-op. Backup is the caller's responsibility -- see
/// `commit_local_to_usb`'s doc comment.
pub(crate) fn write_back_if_changed(usb_root: &Path, kind: DbKind) -> BackendResult<bool> {
    write_back_if_changed_with_root(cache_root().as_deref(), usb_root, kind)
}

fn write_back_if_changed_with_root(
    cache_root: Option<&Path>,
    usb_root: &Path,
    kind: DbKind,
) -> BackendResult<bool> {
    let Some(device_dir) = device_dir_for(cache_root, usb_root) else {
        return Ok(false);
    };
    let local_path = local_vendor_db_dir(&device_dir).join(kind.filename());
    if !local_path.is_file() {
        return Ok(false);
    }

    let _guard = STATE_LOCK.lock().unwrap();
    let local_stat = stat(&local_path)?;
    let state = load_state(&device_dir);
    let unchanged = state
        .files
        .get(kind.state_key())
        .is_some_and(|s| s.local == local_stat);
    if unchanged {
        return Ok(false);
    }

    commit_local_to_usb(usb_root, &device_dir, kind, &local_path)?;
    Ok(true)
}

#[cfg(test)]
pub(crate) fn write_back_if_changed_for_test(
    cache_root: Option<&Path>,
    usb_root: &Path,
    kind: DbKind,
) -> BackendResult<bool> {
    write_back_if_changed_with_root(cache_root, usb_root, kind)
}

#[cfg(test)]
pub(crate) fn commit_and_write_back_for_test(
    cache_root: Option<&Path>,
    usb_root: &Path,
    kind: DbKind,
    local_bytes: &[u8],
) -> BackendResult<bool> {
    commit_and_write_back_with_root(cache_root, usb_root, kind, local_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn usb_root_with_edb(dir: &Path, bytes: &[u8]) -> PathBuf {
        let db_dir = dir.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        std::fs::create_dir_all(&db_dir).unwrap();
        std::fs::write(db_dir.join("exportLibrary.db"), bytes).unwrap();
        dir.to_path_buf()
    }

    #[test]
    fn cache_key_for_root_uses_drive_name_when_named() {
        let usb = tempdir().unwrap();
        crate::service::usb_identity::write_drive_name(usb.path(), "Club Stick").unwrap();
        assert_eq!(cache_key_for_root(usb.path()), "club-stick");
    }

    #[test]
    fn cache_key_for_root_falls_back_to_unnamed_hash() {
        let usb = tempdir().unwrap();
        assert!(cache_key_for_root(usb.path()).starts_with("unnamed-"));
    }

    #[test]
    fn cache_key_for_root_is_stable_across_different_mount_paths_when_named() {
        // Two different "mount paths" (temp dirs) sharing a name must yield
        // the same cache key -- this is the whole point of naming: identity
        // survives a mount path the OS can reassign, unlike the unnamed hash
        // fallback which is keyed on that very path.
        let usb_a = tempdir().unwrap();
        let usb_b = tempdir().unwrap();
        crate::service::usb_identity::write_drive_name(usb_a.path(), "Club Stick").unwrap();
        crate::service::usb_identity::write_drive_name(usb_b.path(), "Club Stick").unwrap();
        assert_eq!(
            cache_key_for_root(usb_a.path()),
            cache_key_for_root(usb_b.path())
        );
    }

    // These tests exercise "staging enabled" behavior entirely through the
    // `_with_root`/`_for_test` entry points, which take the cache root as an
    // explicit parameter instead of reading the process-global `CACHE_ROOT`.
    // That means they never mutate global state, so they're safe to run
    // concurrently with the rest of the crate's test suite -- unlike an
    // earlier version of this module, which toggled `CACHE_ROOT` globally
    // via `set_cache_root_for_test` and, under `cargo test`'s default
    // parallel execution, could have another thread's unrelated
    // `write_pdb`/`open_edb_rw` call observe (and act on) a cache root
    // belonging to a different, already-torn-down test.
    //
    // `stage_pdb`/`stage_edb`/`commit_and_write_back`/`write_back_if_changed`
    // (the global-reading wrappers actually called by production code) are
    // thin one-line delegations to the `_with_root` functions tested here,
    // so this still covers their real logic.

    #[test]
    fn stage_edb_disabled_returns_usb_path_directly() {
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");
        let staged = stage_edb_with_root(None, &usb_root).unwrap();
        assert_eq!(staged, super::super::usb_vendor_compat::vendor_edb_path(&usb_root));
    }

    #[test]
    fn stage_edb_copies_to_local_cache_and_skips_recopy_when_unchanged() {
        let cache = tempdir().unwrap();
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");

        let staged1 = stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();
        assert_ne!(staged1, super::super::usb_vendor_compat::vendor_edb_path(&usb_root));
        assert_eq!(std::fs::read(&staged1).unwrap(), b"hello");

        let mtime_before = std::fs::metadata(&staged1).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let staged2 = stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();
        let mtime_after = std::fs::metadata(&staged2).unwrap().modified().unwrap();
        assert_eq!(staged1, staged2);
        assert_eq!(
            mtime_before, mtime_after,
            "unchanged USB source should not trigger a re-copy"
        );
    }

    #[test]
    fn stage_edb_recopies_when_usb_source_changes() {
        let cache = tempdir().unwrap();
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");

        let staged1 = stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();
        assert_eq!(std::fs::read(&staged1).unwrap(), b"hello");

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            super::super::usb_vendor_compat::vendor_edb_path(&usb_root),
            b"hello world, now longer",
        )
        .unwrap();

        let staged2 = stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();
        assert_eq!(staged1, staged2);
        assert_eq!(std::fs::read(&staged2).unwrap(), b"hello world, now longer");
    }

    #[test]
    fn write_back_if_changed_is_noop_when_local_copy_unchanged() {
        let cache = tempdir().unwrap();
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");
        stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();

        let committed =
            write_back_if_changed_for_test(Some(cache.path()), &usb_root, DbKind::Edb).unwrap();
        assert!(!committed);
        assert_eq!(
            std::fs::read(super::super::usb_vendor_compat::vendor_edb_path(&usb_root)).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn write_back_if_changed_atomically_commits_on_real_change() {
        let cache = tempdir().unwrap();
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");
        let staged = stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();

        std::fs::write(&staged, b"modified locally").unwrap();
        let committed =
            write_back_if_changed_for_test(Some(cache.path()), &usb_root, DbKind::Edb).unwrap();
        assert!(committed);

        assert_eq!(
            std::fs::read(super::super::usb_vendor_compat::vendor_edb_path(&usb_root)).unwrap(),
            b"modified locally"
        );
        // No leftover temp file beside the final destination.
        let db_dir = usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        let leftover_tmp = std::fs::read_dir(&db_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("tmp-"));
        assert!(!leftover_tmp, "atomic rename must not leave a temp file behind");
    }

    #[test]
    fn commit_and_write_back_disabled_staging_writes_usb_path_directly() {
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");
        let committed =
            commit_and_write_back_for_test(None, &usb_root, DbKind::Edb, b"new content").unwrap();
        assert!(committed);
        assert_eq!(
            std::fs::read(super::super::usb_vendor_compat::vendor_edb_path(&usb_root)).unwrap(),
            b"new content",
            "with staging disabled, commit_and_write_back must still write straight to the USB path"
        );
    }

    #[test]
    fn write_back_if_changed_aborts_when_usb_modified_externally_since_staging() {
        let cache = tempdir().unwrap();
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");
        let staged = stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();

        // Simulate external modification (e.g. Rekordbox writing to the
        // drive) after we staged our local copy.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            super::super::usb_vendor_compat::vendor_edb_path(&usb_root),
            b"changed externally",
        )
        .unwrap();

        // Our local edit, made against the now-stale staged copy.
        std::fs::write(&staged, b"our local edit").unwrap();

        let err = write_back_if_changed_for_test(Some(cache.path()), &usb_root, DbKind::Edb)
            .unwrap_err();
        assert!(matches!(err, BackendError::Internal(_)));
        assert_eq!(
            std::fs::read(super::super::usb_vendor_compat::vendor_edb_path(&usb_root)).unwrap(),
            b"changed externally",
            "external change must not be silently clobbered"
        );
    }
}
