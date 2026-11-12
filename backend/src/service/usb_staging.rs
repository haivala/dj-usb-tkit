//! Local staging for the USB's player databases. Readers use the connected
//! drive's working copies. Writers hold the operation's lock, edit through a
//! session, and commit only after a complete eDB checkpoint and conflict check.
//! A failed second write restores the first; this is not crash atomicity.
//!
//! Staging is enabled by the desktop app through [`init_cache_root`]. Without
//! it (standalone CLI tools and most tests), sessions snapshot and restore the
//! directly edited files instead. Backups are an optional session setting.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
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
static STANDALONE_WRITE_LOCK: Mutex<()> = Mutex::new(());
/// Cache keys (see `cache_key_for_root`) of devices whose local staging copy
/// has already been verified fresh for the current connect. While a
/// device's key is present here, `stage_file_with_root` skips its per-call
/// USB `stat()` entirely and trusts the local copy -- staging is meant to
/// happen once per USB connect (`validate_usb_root`, which force-rechecks),
/// not on every read. A plain `Vec` is fine: at most a handful of devices
/// are ever seen per process.
static VERIFIED_THIS_CONNECT: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn is_verified_this_connect(cache_key: &str) -> bool {
    VERIFIED_THIS_CONNECT
        .lock()
        .unwrap()
        .iter()
        .any(|k| k == cache_key)
}

fn mark_verified_this_connect(cache_key: &str) {
    let mut verified = VERIFIED_THIS_CONNECT.lock().unwrap();
    if !verified.iter().any(|k| k == cache_key) {
        verified.push(cache_key.to_string());
    }
}

/// Resolved `cache_key_for_root` results, keyed by the same normalized root
/// string used for device-identity matching elsewhere (`root_path_key` in
/// `usb.rs`), so a hot passive call never has to read the drive-name marker
/// off the USB device just to know which device it's talking to -- only the
/// first call for a given root (of any kind, passive or connect) pays that
/// cost. Invalidated by `forget_cache_key_for_root` whenever the marker is
/// (re)written (naming/renaming a drive), since that changes what
/// `cache_key_for_root` would compute.
static RESOLVED_CACHE_KEYS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn resolve_cache_key(usb_root: &Path) -> String {
    let root_key = normalize_source_root_for_matching(&usb_root.to_string_lossy());
    if let Some((_, cache_key)) = RESOLVED_CACHE_KEYS
        .lock()
        .unwrap()
        .iter()
        .find(|(root, _)| *root == root_key)
    {
        return cache_key.clone();
    }
    let cache_key = cache_key_for_root(usb_root);
    let mut resolved = RESOLVED_CACHE_KEYS.lock().unwrap();
    if !resolved.iter().any(|(root, _)| *root == root_key) {
        resolved.push((root_key, cache_key.clone()));
    }
    cache_key
}

/// Must be called whenever a drive's name marker is written (initial naming
/// or rename) so the next `resolve_cache_key` call re-derives the cache key
/// instead of serving a now-stale cached value from before the drive had
/// (or had a different) name.
pub(crate) fn forget_cache_key_for_root(usb_root: &Path) {
    let root_key = normalize_source_root_for_matching(&usb_root.to_string_lossy());
    RESOLVED_CACHE_KEYS
        .lock()
        .unwrap()
        .retain(|(root, _)| *root != root_key);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbKind {
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
        if let Some(override_value) = CACHE_ROOT_OVERRIDE.with(|cell| cell.borrow().clone()) {
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
    if let Some(name) = super::usb_identity::drive_name(usb_root) {
        return super::usb_identity::slug(&name);
    }

    use std::hash::{Hash, Hasher};
    let normalized = normalize_source_root_for_matching(&usb_root.to_string_lossy());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    format!("unnamed-{:016x}", hasher.finish())
}

/// Moves a named drive's cache dir to its new name's key when the drive is
/// renamed, so its staged DBs and archived backups stay with it. Does
/// nothing when staging is off, the drive has no cache dir yet, or the new
/// key's dir already exists (left behind by a removed drive that had that
/// name -- merging two drives' archives would be worse than starting fresh).
pub(crate) fn rename_cache_dir(old_name: &str, new_name: &str) -> std::io::Result<()> {
    rename_cache_dir_with_root(cache_root().as_deref(), old_name, new_name)
}

fn rename_cache_dir_with_root(
    cache_root: Option<&Path>,
    old_name: &str,
    new_name: &str,
) -> std::io::Result<()> {
    let Some(cache_root) = cache_root else {
        return Ok(());
    };
    let old_dir = cache_root.join(super::usb_identity::slug(old_name));
    let new_dir = cache_root.join(super::usb_identity::slug(new_name));
    if old_dir == new_dir || !old_dir.is_dir() || new_dir.exists() {
        return Ok(());
    }
    let _guard = STATE_LOCK.lock().unwrap();
    std::fs::rename(old_dir, new_dir)
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
    super::usb_identity::drive_name(usb_root)?;
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
/// concurrently with the rest of the suite. [`UsbDatabases`] is the
/// global-reading entry point production code actually calls.
///
/// `force_recheck` distinguishes a connect-time call
/// ([`UsbDatabases::on_connect`], `true`) from every
/// other, passive read/write call site (`false`): only a `true` call is
/// allowed to *establish* the "verified this connect" fast path
/// (`VERIFIED_THIS_CONNECT`) on success, and only a `false` call is allowed
/// to *use* it (skip the `stat()` below entirely). A passive call that
/// hasn't been preceded by a connect-time call always does the full
/// stat/compare/copy check itself, exactly as before this fast path
/// existed -- it just never earns the shortcut for later calls on its own.
fn stage_file_with_root(
    cache_root: Option<&Path>,
    usb_root: &Path,
    source: &Path,
    kind: DbKind,
    force_recheck: bool,
) -> BackendResult<PathBuf> {
    let Some(cache_root) = cache_root else {
        return Ok(source.to_path_buf());
    };
    // Computed once via the cached resolver and reused below (for
    // `device_dir` and the verified-marker lookup) rather than via
    // `device_dir_for`, which would call the uncached `cache_key_for_root`
    // again -- that reads a small marker file directly off the USB device
    // for named drives, so recomputing it on every call (including on the
    // fast path below, meant to avoid touching the device at all) would
    // defeat the point.
    let cache_key = resolve_cache_key(usb_root);
    let device_dir = cache_root.join(&cache_key);
    let local_dir = local_vendor_db_dir(&device_dir);
    let local_path = local_dir.join(kind.filename());

    if force_recheck {
        VERIFIED_THIS_CONNECT
            .lock()
            .unwrap()
            .retain(|key| key != &cache_key);
    }
    if !force_recheck && local_path.is_file() && is_verified_this_connect(&cache_key) {
        return Ok(local_path);
    }

    if optional_stat(source)?.is_none() {
        // Nothing to stage (e.g. fresh USB, PDB not created). Callers
        // already handle "file not present" via their own existence checks.
        // A connect-time call also drops the local copy: one left from an
        // earlier session isn't this USB's database. A passive call keeps
        // it -- it may be one this session created and hasn't written back.
        if force_recheck {
            let _guard = STATE_LOCK.lock().unwrap();
            remove_local_copy(&device_dir, kind, &local_path)?;
        }
        return Ok(local_path);
    }

    let _guard = STATE_LOCK.lock().unwrap();
    let device_dir_is_new = !device_dir.is_dir();
    std::fs::create_dir_all(&local_dir)?;
    if device_dir_is_new {
        crate::logging::debug(
            "usb-staging",
            format!("USB local staging folder: {}", device_dir.display()),
        );
    }

    let source_stat = stat(source)?;
    let state = load_state(&device_dir);
    let recorded = state.files.get(kind.state_key());
    // A reconnect must not reuse edits left by an interrupted session. WAL
    // bytes also count as dirty, even if the main SQLite file is unchanged.
    let wal = local_path.with_file_name(format!("{}-wal", kind.filename()));
    let clean = recorded.is_some_and(|s| stat(&local_path).ok() == Some(s.local))
        && optional_stat(&wal)?.is_none_or(|s| s.size == 0);
    let reuse = local_path.is_file()
        && recorded.is_some_and(|s| s.source == source_stat)
        && (!force_recheck || clean);

    if reuse {
        if force_recheck {
            mark_verified_this_connect(&cache_key);
        }
        return Ok(local_path);
    }

    if force_recheck {
        remove_local_copy(&device_dir, kind, &local_path)?;
    } else if local_path.is_file() && !clean && recorded.is_some() {
        return Err(external_change(source));
    }
    std::fs::copy(source, &local_path)?;
    crate::logging::debug(
        "usb-staging",
        format!(
            "Staged {} locally: {}",
            kind.filename(),
            local_path.display()
        ),
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
    if force_recheck {
        mark_verified_this_connect(&cache_key);
    }

    Ok(local_path)
}

#[cfg(test)]
pub(crate) fn stage_pdb(usb_root: &Path) -> BackendResult<PathBuf> {
    stage_file_with_root(
        cache_root().as_deref(),
        usb_root,
        &super::usb_vendor_compat::vendor_pdb_path(usb_root),
        DbKind::Pdb,
        false,
    )
}

#[cfg(test)]
pub(crate) fn stage_edb(usb_root: &Path) -> BackendResult<PathBuf> {
    stage_file_with_root(
        cache_root().as_deref(),
        usb_root,
        &super::usb_vendor_compat::vendor_edb_path(usb_root),
        DbKind::Edb,
        false,
    )
}

/// Force-recheck variants used to (re-)stage exactly once per USB connect
/// (see `BackendService::validate_usb_root`), regardless of whether this
/// device was already marked verified earlier in the process -- unlike
/// `stage_pdb`/`stage_edb`, which trust an existing verified mark and skip
/// the USB `stat()` entirely.
#[cfg(test)]
pub(crate) fn stage_pdb_on_connect(usb_root: &Path) -> BackendResult<PathBuf> {
    stage_file_with_root(
        cache_root().as_deref(),
        usb_root,
        &super::usb_vendor_compat::vendor_pdb_path(usb_root),
        DbKind::Pdb,
        true,
    )
}

#[cfg(test)]
pub(crate) fn stage_edb_with_root(
    cache_root: Option<&Path>,
    usb_root: &Path,
) -> BackendResult<PathBuf> {
    stage_file_with_root(
        cache_root,
        usb_root,
        &super::usb_vendor_compat::vendor_edb_path(usb_root),
        DbKind::Edb,
        false,
    )
}

#[cfg(test)]
pub(crate) fn stage_edb_on_connect_with_root(
    cache_root: Option<&Path>,
    usb_root: &Path,
) -> BackendResult<PathBuf> {
    stage_file_with_root(
        cache_root,
        usb_root,
        &super::usb_vendor_compat::vendor_edb_path(usb_root),
        DbKind::Edb,
        true,
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
    ensure_usb_copy_unchanged(device_dir, kind, &dest)?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = write_back_temp_path(&dest);
    std::fs::copy(local_path, &tmp)?;
    std::fs::rename(&tmp, &dest)?;
    crate::logging::debug(
        "usb-staging",
        format!("Wrote {} back to USB: {}", kind.filename(), dest.display()),
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

/// The file a write-back copies to before renaming it over `dest`.
fn write_back_temp_path(dest: &Path) -> PathBuf {
    dest.with_extension(format!("tmp-{}", std::process::id()))
}

/// Where the write-back of `kind` puts its temporary copy on the USB; a test
/// blocks that path to make the write-back fail.
#[cfg(test)]
pub(crate) fn write_back_temp_path_for_test(usb_root: &Path, kind: DbKind) -> PathBuf {
    write_back_temp_path(&vendor_path(usb_root, kind))
}

/// External-modification guard: if the USB source has moved since we last
/// synced with it, don't silently clobber whatever changed it.
fn optional_stat(path: &Path) -> std::io::Result<Option<FileStat>> {
    match stat(path) {
        Ok(stat) => Ok(Some(stat)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn external_change(path: &Path) -> BackendError {
    BackendError::Internal(format!(
        "USB database changed since it was last read ({}) -- re-open and retry to avoid discarding the external change",
        path.display()
    ))
}

fn ensure_usb_copy_unchanged(device_dir: &Path, kind: DbKind, dest: &Path) -> BackendResult<()> {
    let expected = load_state(device_dir)
        .files
        .get(kind.state_key())
        .map(|s| s.source);
    if optional_stat(dest)? != expected {
        return Err(external_change(dest));
    }
    Ok(())
}

/// Fails when the USB copy of `kind` changed since it was staged, so a
/// caller writing back several databases can find out before writing any.
fn check_write_back(usb_root: &Path, kind: DbKind) -> BackendResult<()> {
    let Some(device_dir) = cache_root().map(|root| root.join(resolve_cache_key(usb_root))) else {
        return Ok(());
    };
    let _guard = STATE_LOCK.lock().unwrap();
    ensure_usb_copy_unchanged(&device_dir, kind, &vendor_path(usb_root, kind))
}

/// Deletes the local copy of `kind` (with the eDB's journal files) and its
/// synced state. The caller holds `STATE_LOCK`.
fn remove_local_copy(device_dir: &Path, kind: DbKind, local_path: &Path) -> BackendResult<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut path = local_path.as_os_str().to_owned();
        path.push(suffix);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    let mut state = load_state(device_dir);
    if state.files.remove(kind.state_key()).is_some() {
        let _ = save_state(device_dir, &state);
    }
    Ok(())
}

/// Drops the local changes to a staged database and stages it again from
/// the USB, so the local copy matches the drive.
pub(crate) fn restage_from_usb(usb_root: &Path, kind: DbKind) -> BackendResult<PathBuf> {
    let cache_root = cache_root();
    if let Some(device_dir) = cache_root
        .as_ref()
        .map(|root| root.join(resolve_cache_key(usb_root)))
    {
        let local_path = local_vendor_db_dir(&device_dir).join(kind.filename());
        let _guard = STATE_LOCK.lock().unwrap();
        remove_local_copy(&device_dir, kind, &local_path)?;
    }
    stage_file_with_root(
        cache_root.as_deref(),
        usb_root,
        &vendor_path(usb_root, kind),
        kind,
        true,
    )
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
fn commit_and_write_back(usb_root: &Path, kind: DbKind, local_bytes: &[u8]) -> BackendResult<bool> {
    commit_and_write_back_with_root(cache_root().as_deref(), usb_root, kind, local_bytes)
}

fn commit_and_write_back_with_root(
    cache_root: Option<&Path>,
    usb_root: &Path,
    kind: DbKind,
    local_bytes: &[u8],
) -> BackendResult<bool> {
    let Some(device_dir) = cache_root.map(|root| root.join(resolve_cache_key(usb_root))) else {
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
fn write_changed_database(usb_root: &Path, kind: DbKind) -> BackendResult<bool> {
    write_back_if_changed_with_root(cache_root().as_deref(), usb_root, kind)
}

#[cfg(test)]
pub(crate) fn write_back_if_changed(usb_root: &Path, kind: DbKind) -> BackendResult<bool> {
    write_changed_database(usb_root, kind)
}

fn write_back_if_changed_with_root(
    cache_root: Option<&Path>,
    usb_root: &Path,
    kind: DbKind,
) -> BackendResult<bool> {
    let Some(device_dir) = cache_root.map(|root| root.join(resolve_cache_key(usb_root))) else {
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

// ── Which databases a stick has ──────────────────────────────────────────

/// The player databases a stick has, as staged: the PDB, the eDB or both.
/// Staging decides this once per call from its own per-device copies (which
/// a connect re-syncs with the USB), so nothing downstream checks files.
/// Read from whichever is present, PDB first; write to each present one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbDatabases {
    Both { pdb: PathBuf, edb: PathBuf },
    PdbOnly { pdb: PathBuf },
    EdbOnly { edb: PathBuf },
}

impl UsbDatabases {
    fn from_staged(pdb: PathBuf, edb: PathBuf) -> Option<Self> {
        match (pdb.is_file(), edb.is_file()) {
            (true, true) => Some(Self::Both { pdb, edb }),
            (true, false) => Some(Self::PdbOnly { pdb }),
            (false, true) => Some(Self::EdbOnly { edb }),
            (false, false) => None,
        }
    }

    /// The databases `usb_root` has. Fails when it has neither.
    pub(crate) fn of(usb_root: &Path) -> BackendResult<Self> {
        Self::present(usb_root)?
            .ok_or_else(|| BackendError::NotFound(crate::i18n::tr::usb_error_no_player_database()))
    }

    /// The databases `usb_root` has, `None` when it has neither (a fresh
    /// stick, before export or initialize creates them).
    pub(crate) fn present(usb_root: &Path) -> BackendResult<Option<Self>> {
        Ok(Self::from_staged(
            staged_path(usb_root, DbKind::Pdb, false)?,
            staged_path(usb_root, DbKind::Edb, false)?,
        ))
    }

    /// Stages both databases afresh, once per USB connect: a database the
    /// USB lacks is dropped from staging. Best-effort: a staging hiccup is
    /// logged, and that database reads as missing.
    pub(crate) fn on_connect(usb_root: &Path) -> Option<Self> {
        let stage = |kind: DbKind| {
            staged_path(usb_root, kind, true)
                .inspect_err(|err| {
                    crate::logging::debug(
                        "usb-import",
                        format!(
                            "Failed to stage {} locally: {err}",
                            kind.label(),
                            err = err.english()
                        ),
                    );
                })
                .unwrap_or_default()
        };
        let pdb = stage(DbKind::Pdb);
        let edb = stage(DbKind::Edb);
        Self::from_staged(pdb, edb)
    }

    pub(crate) fn pdb(&self) -> Option<&Path> {
        match self {
            Self::Both { pdb, .. } | Self::PdbOnly { pdb } => Some(pdb),
            Self::EdbOnly { .. } => None,
        }
    }

    pub(crate) fn edb(&self) -> Option<&Path> {
        match self {
            Self::Both { edb, .. } | Self::EdbOnly { edb } => Some(edb),
            Self::PdbOnly { .. } => None,
        }
    }

    /// The staged PDB, for a reader that needs it: an eDB-only stick fails.
    pub(crate) fn require_pdb(&self) -> BackendResult<&Path> {
        self.pdb()
            .ok_or_else(|| BackendError::NotFound(crate::i18n::tr::usb_error_no_pdb()))
    }

    pub(crate) fn path(&self, kind: DbKind) -> Option<&Path> {
        match kind {
            DbKind::Pdb => self.pdb(),
            DbKind::Edb => self.edb(),
        }
    }

    pub(crate) fn has(&self, kind: DbKind) -> bool {
        self.path(kind).is_some()
    }

    /// The database the stick lacks while it has the other one.
    pub(crate) fn missing(&self) -> Option<DbKind> {
        match self {
            Self::Both { .. } => None,
            Self::PdbOnly { .. } => Some(DbKind::Edb),
            Self::EdbOnly { .. } => Some(DbKind::Pdb),
        }
    }

    /// A read-only connection to the staged eDB; `None` without one, or when
    /// it can't be opened (that adds a warning).
    pub(crate) fn open_edb(
        &self,
        warnings: &mut Vec<crate::models::WarningEntry>,
    ) -> Option<rusqlite::Connection> {
        crate::edb::open_edb(self.edb()?, warnings)
    }

    /// A read-write connection to the staged eDB; `None` without one.
    pub(crate) fn open_edb_writable(&self) -> Option<rusqlite::Connection> {
        crate::edb::open_edb_rw_at(self.edb()?)
    }

    /// Where `media_path`'s track is in each database: its PDB row (from
    /// `parsed`, the parse of [`Self::pdb`]) and its eDB `content_id` (through
    /// `edb`, a connection to [`Self::edb`]). The per-database decision of a
    /// track edit comes from this: see [`TrackLocation::require`].
    pub(crate) fn locate_track<'p>(
        &self,
        parsed: Option<&'p crate::pdb_reader::ParsedPdb>,
        edb: Option<&rusqlite::Connection>,
        media_path: &str,
    ) -> BackendResult<TrackLocation<'p>> {
        let media_path = media_path.trim();
        let pdb_row = match parsed.filter(|_| self.pdb().is_some()) {
            Some(parsed) => parsed
                .tracks
                .iter()
                .find(|t| t.track_file_path == media_path)
                .or_else(|| {
                    parsed
                        .tracks
                        .iter()
                        .find(|t| t.track_file_path.eq_ignore_ascii_case(media_path))
                }),
            None => None,
        };
        let edb_content_id = match edb.filter(|_| self.edb().is_some()) {
            Some(conn) => crate::edb::find_content_id_by_path(conn, media_path)?,
            None => None,
        };
        Ok(TrackLocation {
            pdb_row,
            edb_content_id,
            pdb_present: self.pdb().is_some(),
            edb_present: self.edb().is_some(),
        })
    }
}

/// The staged PDB of `usb_root`, for a reader that needs it.
pub(crate) fn required_pdb(usb_root: &Path) -> BackendResult<PathBuf> {
    Ok(UsbDatabases::of(usb_root)?.require_pdb()?.to_path_buf())
}

/// [`UsbDatabases::pdb`] / [`UsbDatabases::edb`] for a stick that may have
/// neither ([`UsbDatabases::present`]).
pub trait UsbDatabasesExt {
    fn pdb(&self) -> Option<&Path>;
    fn edb(&self) -> Option<&Path>;
}

impl UsbDatabasesExt for Option<UsbDatabases> {
    fn pdb(&self) -> Option<&Path> {
        self.as_ref().and_then(UsbDatabases::pdb)
    }
    fn edb(&self) -> Option<&Path> {
        self.as_ref().and_then(UsbDatabases::edb)
    }
}

/// Where a track is in a stick's databases (see [`UsbDatabases::locate_track`]).
#[derive(Debug, Clone, Copy)]
pub struct TrackLocation<'p> {
    pub(crate) pdb_row: Option<&'p crate::pdb_reader::PdbTrackRow>,
    pub(crate) edb_content_id: Option<i64>,
    pdb_present: bool,
    edb_present: bool,
}

impl TrackLocation<'_> {
    /// The databases a present database lacks this track in.
    pub(crate) fn gaps(&self) -> Vec<DbKind> {
        let mut gaps = Vec::new();
        if self.pdb_present && self.pdb_row.is_none() {
            gaps.push(DbKind::Pdb);
        }
        if self.edb_present && self.edb_content_id.is_none() {
            gaps.push(DbKind::Edb);
        }
        gaps
    }

    /// In at least one database. A track in neither is `not_found`.
    pub(crate) fn require(self, not_found: impl FnOnce() -> BackendError) -> BackendResult<Self> {
        if self.pdb_row.is_none() && self.edb_content_id.is_none() {
            return Err(not_found());
        }
        Ok(self)
    }
}

/// The staged path of `kind` (the USB path when staging is off), present on
/// the stick or not.
fn staged_path(usb_root: &Path, kind: DbKind, on_connect: bool) -> BackendResult<PathBuf> {
    stage_file_with_root(
        cache_root().as_deref(),
        usb_root,
        &vendor_path(usb_root, kind),
        kind,
        on_connect,
    )
}

/// Where `kind` of `usb_root` is staged, present on the stick or not: for a
/// writer that may create it (in a write session).
pub(crate) fn staged_write_path(usb_root: &Path, kind: DbKind) -> BackendResult<PathBuf> {
    staged_path(usb_root, kind, false)
}

/// A read-write connection to the staged eDB of `usb_root`; `None` without one.
pub fn open_edb_rw(usb_root: &Path) -> Option<rusqlite::Connection> {
    UsbDatabases::present(usb_root)
        .ok()
        .flatten()
        .and_then(|dbs| dbs.open_edb_writable())
}

/// The caller's shared eDB connection, else one of its own opened through
/// the layer: what a repair step that can run alone or inside a repair run
/// writes through.
pub(crate) enum EdbRw<'a> {
    Shared(&'a mut rusqlite::Connection),
    Owned(rusqlite::Connection),
}

impl<'a> EdbRw<'a> {
    pub(crate) fn shared_or_open(
        shared: Option<&'a mut rusqlite::Connection>,
        usb_root: &Path,
    ) -> Option<Self> {
        match shared {
            Some(conn) => Some(Self::Shared(conn)),
            None => open_edb_rw(usb_root).map(Self::Owned),
        }
    }

    /// Like [`Self::shared_or_open`], opening a connection of its own with
    /// `open` instead.
    pub(crate) fn shared_or(
        shared: Option<&'a mut rusqlite::Connection>,
        open: impl FnOnce() -> Option<rusqlite::Connection>,
    ) -> Option<Self> {
        match shared {
            Some(conn) => Some(Self::Shared(conn)),
            None => open().map(Self::Owned),
        }
    }

    pub(crate) fn conn(&mut self) -> &mut rusqlite::Connection {
        match self {
            Self::Shared(conn) => conn,
            Self::Owned(conn) => conn,
        }
    }
}

/// Checkpoints the eDB's write-ahead log into its main file: the staging
/// write-back copies only that file. An incomplete checkpoint fails the save.
fn checkpoint_edb(conn: &rusqlite::Connection) -> BackendResult<()> {
    // A reader still on an older snapshot gets this long to finish.
    conn.busy_timeout(std::time::Duration::from_secs(1))?;
    let (busy, log_frames, checkpointed) =
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
    if busy != 0 || (log_frames >= 0 && checkpointed != log_frames) {
        return Err(BackendError::Validation(
            crate::i18n::tr::usb_edb_checkpoint_failed(),
        ));
    }
    Ok(())
}

// ── Write session ────────────────────────────────────────────────────────

/// A backup a write session takes when it opens.
#[derive(Debug, Clone, Copy)]
pub enum SessionBackup<'r> {
    None,
    /// `backup_usb_databases_with_retention`: `reason` and a fresh playlist
    /// count, when the write can change it.
    WithRetention {
        reason: &'r str,
        playlist_count: Option<usize>,
    },
    /// The same, with the playlist count from the staged PDB as it is now.
    CountingPlaylists {
        reason: &'r str,
    },
    /// Taking the place of a newest backup of the same reason.
    ReplacingSameReason {
        reason: &'r str,
    },
}

/// One write to a stick's databases. On open it takes the backup asked for;
/// the writer then changes the staged copies -- the PDB through
/// [`Self::write_pdb`], the eDB through the one shared connection
/// [`Self::edb`] -- and [`Self::commit`] writes each changed database back
/// (both or neither) and drops the parse caches. Dropped without a commit
/// (an error), it puts the staged copies back as they were and writes
/// nothing to the USB.
pub struct UsbWriteSession<'s> {
    service: Option<&'s super::BackendService>,
    // Borrowing the operation's guard keeps the entire read/modify/write
    // sequence locked, including post-commit work done by the caller.
    _write_guard: Option<&'s UsbWriteGuard<'s>>,
    _standalone_guard: Option<MutexGuard<'static, ()>>,
    usb_root: PathBuf,
    edb: Option<rusqlite::Connection>,
    /// The databases this session changed (or may have, through its
    /// connection), PDB then eDB.
    touched: [bool; 2],
    /// The staged PDB before this session changed it, `None` when the stick
    /// had none: for undoing a PDB write-back when the eDB's then fails.
    pdb_original: Option<Option<Vec<u8>>>,
    /// Staging off: the USB files as they were, put back on a drop.
    direct_snapshots: [Option<Option<Vec<u8>>>; 2],
    done: bool,
}

fn kind_index(kind: DbKind) -> usize {
    match kind {
        DbKind::Pdb => 0,
        DbKind::Edb => 1,
    }
}

impl DbKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            DbKind::Pdb => "PDB",
            DbKind::Edb => "eDB",
        }
    }
}

/// Proof that the caller holds the service's USB write lock: only
/// [`super::BackendService::lock_usb_writes`] and
/// [`super::BackendService::try_lock_usb_writes`] make one, and a write
/// session can't be opened without it.
pub(crate) struct UsbWriteGuard<'a> {
    _held: MutexGuard<'a, ()>,
}

impl super::BackendService {
    /// Takes the USB write lock, waiting for a running write to finish.
    pub(crate) fn lock_usb_writes(&self) -> UsbWriteGuard<'_> {
        UsbWriteGuard {
            _held: self
                .usb_write_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        }
    }

    /// Takes the USB write lock, or `None` while another write holds it.
    pub(crate) fn try_lock_usb_writes(&self) -> Option<UsbWriteGuard<'_>> {
        match self.usb_write_lock.try_lock() {
            Ok(guard) => Some(UsbWriteGuard { _held: guard }),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(UsbWriteGuard {
                _held: poisoned.into_inner(),
            }),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    /// Opens a [`UsbWriteSession`] on `usb_root`, taking `backup` first; the
    /// backup's warnings go to `warnings`.
    pub(crate) fn usb_write_session<'s>(
        &'s self,
        guard: &'s UsbWriteGuard<'s>,
        usb_root: &Path,
        backup: SessionBackup<'_>,
        warnings: &mut Vec<crate::models::WarningEntry>,
    ) -> UsbWriteSession<'s> {
        match backup {
            SessionBackup::None => {}
            SessionBackup::WithRetention {
                reason,
                playlist_count,
            } => warnings.extend(self.backup_usb_databases_with_retention(
                usb_root,
                reason,
                playlist_count,
            )),
            SessionBackup::CountingPlaylists { reason } => {
                let playlist_count = UsbDatabases::present(usb_root)
                    .ok()
                    .flatten()
                    .pdb()
                    .and_then(crate::pdb_reader::count_pdb_playlists);
                warnings.extend(self.backup_usb_databases_with_retention(
                    usb_root,
                    reason,
                    playlist_count,
                ));
            }
            SessionBackup::ReplacingSameReason { reason } => {
                warnings.extend(self.backup_usb_databases_replacing_same_reason(usb_root, reason));
            }
        }
        let mut session = UsbWriteSession::open(Some(self), usb_root);
        session._write_guard = Some(guard);
        session
    }
}

impl<'s> UsbWriteSession<'s> {
    /// A session with no service: no backup, and no parse caches to drop.
    pub(crate) fn without_service(usb_root: &Path) -> UsbWriteSession<'static> {
        let guard = STANDALONE_WRITE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut session = UsbWriteSession::open(None, usb_root);
        session._standalone_guard = Some(guard);
        session
    }

    /// A session without a backup; `service` (when given) has its parse
    /// caches dropped on commit.
    fn open(service: Option<&'s super::BackendService>, usb_root: &Path) -> Self {
        Self {
            service,
            _write_guard: None,
            _standalone_guard: None,
            usb_root: usb_root.to_path_buf(),
            edb: None,
            touched: [false; 2],
            pdb_original: None,
            direct_snapshots: [None, None],
            done: false,
        }
    }

    pub(crate) fn usb_root(&self) -> &Path {
        &self.usb_root
    }

    /// The databases the stick has now, as this session has staged them.
    pub(crate) fn databases(&self) -> BackendResult<Option<UsbDatabases>> {
        UsbDatabases::present(&self.usb_root)
    }

    /// Marks `kind` as changed by this session, keeping what's needed to
    /// undo that first.
    fn touch(&mut self, kind: DbKind) -> BackendResult<()> {
        let index = kind_index(kind);
        if self.touched[index] {
            return Ok(());
        }
        let path = staged_path(&self.usb_root, kind, false)?;
        let current = || -> BackendResult<Option<Vec<u8>>> {
            Ok(if path.is_file() {
                Some(std::fs::read(&path)?)
            } else {
                None
            })
        };
        if cache_root().is_none() {
            self.direct_snapshots[index] = Some(current()?);
        }
        if kind == DbKind::Pdb {
            self.pdb_original = Some(if vendor_path(&self.usb_root, kind).is_file() {
                current()?
            } else {
                None
            });
        }
        self.touched[index] = true;
        Ok(())
    }

    /// The shared read-write connection to the staged eDB, opened on first
    /// use; `None` while the stick has no eDB.
    pub(crate) fn edb(&mut self) -> BackendResult<Option<&mut rusqlite::Connection>> {
        if self.edb.is_none() {
            let Some(path) = UsbDatabases::present(&self.usb_root)?
                .edb()
                .map(Path::to_path_buf)
            else {
                return Ok(None);
            };
            self.touch(DbKind::Edb)?;
            self.edb = Some(crate::edb::open_edb_rw_at(&path).ok_or_else(|| {
                BackendError::NotFound(crate::i18n::tr::error_usb_edb_unreadable())
            })?);
        }
        Ok(self.edb.as_mut())
    }

    /// Closes the shared eDB connection, its WAL checkpointed into the file
    /// first: a write-back copies the file, not through SQLite.
    pub(crate) fn close_edb(&mut self) -> BackendResult<()> {
        if let Some(conn) = self.edb.as_ref() {
            checkpoint_edb(conn)?;
        }
        self.edb.take();
        Ok(())
    }

    /// The staged PDB's path, for a writer that changes the file itself;
    /// whatever it writes there is this session's change. `None` while the
    /// stick has no PDB.
    pub(crate) fn pdb_path(&mut self) -> BackendResult<Option<PathBuf>> {
        let Some(path) = UsbDatabases::present(&self.usb_root)?
            .pdb()
            .map(Path::to_path_buf)
        else {
            return Ok(None);
        };
        self.touch(DbKind::Pdb)?;
        Ok(Some(path))
    }

    /// Puts `bytes` into the staged PDB, creating it when the stick has none.
    pub(crate) fn write_pdb(&mut self, bytes: &[u8]) -> BackendResult<()> {
        self.touch(DbKind::Pdb)?;
        let path = staged_path(&self.usb_root, DbKind::Pdb, false)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if cache_root().is_some() {
            std::fs::write(&path, bytes)?;
        } else {
            super::anlz::atomic_write_bytes(&path, bytes)?;
        }
        Ok(())
    }

    /// Creates `kind`, missing from the stick, as `initialize_usb` does:
    /// `created_date` is the day it was made. A local copy left from an
    /// earlier session isn't this stick's database, so it goes first.
    /// Returns the staged path.
    pub(crate) fn create_database(
        &mut self,
        kind: DbKind,
        created_date: &str,
    ) -> BackendResult<PathBuf> {
        if kind == DbKind::Edb {
            self.close_edb()?;
        }
        let usb_path = vendor_path(&self.usb_root, kind);
        if usb_path.exists() {
            return Err(BackendError::Internal(format!(
                "{} already exists: {}",
                kind.label(),
                usb_path.display()
            )));
        }
        let path = restage_from_usb(&self.usb_root, kind)?;
        self.touch(kind)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match kind {
            DbKind::Pdb => {
                std::fs::write(
                    &path,
                    super::usb_utils::build_fullshape_pdb_bytes(created_date),
                )?;
            }
            DbKind::Edb => super::usb_utils::initialize_fullshape_edb(&path, created_date)?,
        }
        Ok(path)
    }

    /// Drops `kind`, which [`Self::create_database`] created, again: a failed
    /// rebuild leaves nothing to write back.
    pub(crate) fn drop_created_database(&mut self, kind: DbKind) {
        if kind == DbKind::Edb {
            self.edb.take();
        }
        if cache_root().is_some() {
            let _ = restage_from_usb(&self.usb_root, kind);
        } else {
            let path = vendor_path(&self.usb_root, kind);
            for suffix in ["", "-wal", "-shm"] {
                let mut file = path.as_os_str().to_owned();
                file.push(suffix);
                let _ = std::fs::remove_file(file);
            }
        }
    }

    /// Marks `kind` as one this session writes, for a writer that changes
    /// its staged copy on its own (the repair's in-memory PDB).
    pub(crate) fn will_write(&mut self, kind: DbKind) -> BackendResult<()> {
        self.touch(kind)
    }

    /// [`Self::edb`], with a failure to stage logged and read as no eDB: for
    /// steps that open a connection of their own without one.
    pub(crate) fn edb_conn(&mut self) -> Option<&mut rusqlite::Connection> {
        match self.edb() {
            Ok(conn) => conn,
            Err(err) => {
                crate::logging::debug(
                    "usb-staging",
                    format!("could not stage the eDB: {err}", err = err.english()),
                );
                None
            }
        }
    }

    /// Replaces `kind` with the file at `source` (a backup): what the stick
    /// holds is staged again first, so the copy lands on it as it is now.
    pub(crate) fn replace_database(&mut self, kind: DbKind, source: &Path) -> BackendResult<()> {
        if kind == DbKind::Edb {
            self.close_edb()?;
        }
        if cache_root().is_some() {
            restage_from_usb(&self.usb_root, kind)?;
        }
        self.touch(kind)?;
        let path = staged_path(&self.usb_root, kind, false)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source, &path)?;
        Ok(())
    }

    /// Writes each changed database back to the USB, both or neither, and
    /// drops the parse caches.
    pub(crate) fn commit(mut self) -> BackendResult<()> {
        // Helpers may have written through a separate connection. Checkpoint
        // a remaining WAL too. A restored byte snapshot has no WAL to flush.
        if self.touched[1] && self.edb.is_none() {
            let path = staged_path(&self.usb_root, DbKind::Edb, false)?;
            let wal = path.with_file_name("exportLibrary.db-wal");
            if optional_stat(&wal)?.is_some_and(|s| s.size > 0) {
                self.edb()?;
            }
        }
        self.close_edb()?;
        let original = self.pdb_original.take();
        let result = write_back_both_or_neither(
            &self.usb_root,
            original.as_ref().map(|original| original.as_deref()),
            self.touched,
        );
        self.done = true;
        if let Some(service) = self.service {
            service.invalidate_usb_parse_cache();
        }
        result
    }
}

impl Drop for UsbWriteSession<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // Discarding needs no checkpoint. Close the handle before removing
        // staging and its journals (also after an incomplete checkpoint).
        self.edb.take();
        for kind in [DbKind::Pdb, DbKind::Edb] {
            let index = kind_index(kind);
            if !self.touched[index] {
                continue;
            }
            match self.direct_snapshots[index].take() {
                Some(snapshot) => {
                    let path = vendor_path(&self.usb_root, kind);
                    let _ = match snapshot {
                        Some(bytes) => std::fs::write(&path, bytes),
                        None => std::fs::remove_file(&path),
                    };
                }
                None => {
                    let _ = restage_from_usb(&self.usb_root, kind);
                }
            }
        }
        if let Some(service) = self.service {
            service.invalidate_usb_parse_cache();
        }
    }
}

/// Writes both staged databases back to the USB, or neither: before writing,
/// both USB copies are checked for outside changes. When the eDB write fails
/// after the PDB was written, `pdb_original` (what the USB held; `Some(None)`
/// when it had no PDB; `None` when unknown, as for a PDB no writer said it
/// changes) is put back. On failure the staged copies are staged again from
/// the USB, so they match the drive.
pub(crate) fn write_back_both_or_neither(
    usb_root: &Path,
    pdb_original: Option<Option<&[u8]>>,
    touched: [bool; 2],
) -> BackendResult<()> {
    let restage_both = || {
        let _ = restage_from_usb(usb_root, DbKind::Pdb);
        let _ = restage_from_usb(usb_root, DbKind::Edb);
    };
    let pdb_written = check_write_back(usb_root, DbKind::Pdb)
        .and_then(|()| check_write_back(usb_root, DbKind::Edb))
        .and_then(|()| {
            if touched[0] {
                write_changed_database(usb_root, DbKind::Pdb)
            } else {
                Ok(false)
            }
        })
        .inspect_err(|_| restage_both())?;
    if let Err(err) = if touched[1] {
        write_changed_database(usb_root, DbKind::Edb)
    } else {
        Ok(false)
    } {
        let undo = || match pdb_original {
            Some(Some(original)) => {
                commit_and_write_back(usb_root, DbKind::Pdb, original).map(|_| ())
            }
            Some(None) => {
                std::fs::remove_file(vendor_path(usb_root, DbKind::Pdb)).map_err(Into::into)
            }
            None => Err(BackendError::Internal(
                "the previous PDB wasn't kept".to_string(),
            )),
        };
        if pdb_written && let Err(undo_err) = undo() {
            restage_both();
            return Err(BackendError::Internal(format!(
                "{err}; putting the previous PDB back also failed ({undo_err}), so the USB has the \
                 new PDB with the previous eDB -- restore the backup taken before this change",
                err = err.english(),
                undo_err = undo_err.english()
            )));
        }
        restage_both();
        return Err(err);
    }
    Ok(())
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

    #[test]
    fn rename_cache_dir_moves_the_dir_but_never_onto_an_existing_one() {
        let cache = tempdir().unwrap();
        std::fs::create_dir_all(cache.path().join("club-stick/backups")).unwrap();
        rename_cache_dir_with_root(Some(cache.path()), "Club Stick", "Booth Stick").unwrap();
        assert!(!cache.path().join("club-stick").exists());
        assert!(cache.path().join("booth-stick/backups").is_dir());

        std::fs::create_dir_all(cache.path().join("old-stick")).unwrap();
        rename_cache_dir_with_root(Some(cache.path()), "Booth Stick", "Old Stick").unwrap();
        assert!(
            cache.path().join("booth-stick/backups").is_dir(),
            "an existing dir for the new name is left alone"
        );

        rename_cache_dir_with_root(None, "Booth Stick", "New Stick").unwrap();
        rename_cache_dir_with_root(Some(cache.path()), "Missing", "New Stick").unwrap();
        assert!(!cache.path().join("new-stick").exists());
    }

    #[test]
    fn resolve_cache_key_is_cached_until_forgotten() {
        let usb = tempdir().unwrap();
        let usb_root = usb.path().to_path_buf();

        let key1 = resolve_cache_key(&usb_root);
        assert!(key1.starts_with("unnamed-"));

        // Naming the drive without invalidating the resolver's cache must
        // still serve the stale (pre-name) key -- this is what makes the
        // resolver actually skip the marker-file read on repeat calls.
        crate::service::usb_identity::write_drive_name(&usb_root, "Club Stick").unwrap();
        let key2 = resolve_cache_key(&usb_root);
        assert_eq!(
            key1, key2,
            "resolve_cache_key must reuse its cached value until explicitly forgotten"
        );

        forget_cache_key_for_root(&usb_root);
        let key3 = resolve_cache_key(&usb_root);
        assert_eq!(
            key3, "club-stick",
            "after forget_cache_key_for_root, resolve_cache_key must reflect the new name"
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
        assert_eq!(
            staged,
            super::super::usb_vendor_compat::vendor_edb_path(&usb_root)
        );
    }

    #[test]
    fn stage_edb_copies_to_local_cache_and_skips_recopy_when_unchanged() {
        let cache = tempdir().unwrap();
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");

        let staged1 = stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();
        assert_ne!(
            staged1,
            super::super::usb_vendor_compat::vendor_edb_path(&usb_root)
        );
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
    fn stage_edb_skips_recheck_after_verified_on_connect() {
        let cache = tempdir().unwrap();
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");

        // Connect-time staging: marks this device verified for the session.
        let staged1 = stage_edb_on_connect_with_root(Some(cache.path()), &usb_root).unwrap();
        assert_eq!(std::fs::read(&staged1).unwrap(), b"hello");

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            super::super::usb_vendor_compat::vendor_edb_path(&usb_root),
            b"changed after connect",
        )
        .unwrap();

        // A passive call after the device was verified must trust the local
        // copy and skip its own stat() -- the USB-side change must not be
        // picked up until the next connect-time (force-recheck) call.
        let staged2 = stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();
        assert_eq!(staged1, staged2);
        assert_eq!(
            std::fs::read(&staged2).unwrap(),
            b"hello",
            "a passive call must not re-stat once verified this connect"
        );
    }

    #[test]
    fn stage_edb_on_connect_always_resyncs_even_when_already_verified() {
        let cache = tempdir().unwrap();
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");

        stage_edb_on_connect_with_root(Some(cache.path()), &usb_root).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            super::super::usb_vendor_compat::vendor_edb_path(&usb_root),
            b"changed externally, reconnected",
        )
        .unwrap();

        // A second connect-time call must force a real recheck regardless of
        // the existing verified mark (e.g. drive was replugged/modified
        // externally and the user re-picked it).
        let staged = stage_edb_on_connect_with_root(Some(cache.path()), &usb_root).unwrap();
        assert_eq!(
            std::fs::read(&staged).unwrap(),
            b"changed externally, reconnected",
            "connect-time staging must always re-verify, never trust a stale mark"
        );
    }

    #[test]
    fn stage_edb_on_connect_drops_the_local_copy_when_the_usb_has_none() {
        let cache = tempdir().unwrap();
        let usb = tempdir().unwrap();
        let usb_root = usb_root_with_edb(usb.path(), b"hello");
        let staged = stage_edb_on_connect_with_root(Some(cache.path()), &usb_root).unwrap();
        std::fs::write(staged.with_extension("db-wal"), b"wal").unwrap();
        std::fs::remove_file(super::super::usb_vendor_compat::vendor_edb_path(&usb_root)).unwrap();

        // A passive call keeps it: it may be a copy this session hasn't
        // written back yet.
        let passive = stage_edb_with_root(Some(cache.path()), &usb_root).unwrap();
        assert!(passive.is_file());

        let staged = stage_edb_on_connect_with_root(Some(cache.path()), &usb_root).unwrap();
        assert!(
            !staged.exists(),
            "a stale local eDB must not stand in for the USB's"
        );
        assert!(!staged.with_extension("db-wal").exists());
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
        assert!(
            !leftover_tmp,
            "atomic rename must not leave a temp file behind"
        );
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

        let err =
            write_back_if_changed_for_test(Some(cache.path()), &usb_root, DbKind::Edb).unwrap_err();
        assert!(matches!(err, BackendError::Internal(_)));
        assert_eq!(
            std::fs::read(super::super::usb_vendor_compat::vendor_edb_path(&usb_root)).unwrap(),
            b"changed externally",
            "external change must not be silently clobbered"
        );
    }
}

// Failure contracts of the shared write session.
#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn failed_initialization_removes_the_new_pdb() {
        let usb = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let _override = set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let blocker = write_back_temp_path_for_test(usb.path(), DbKind::Edb);
        std::fs::create_dir_all(&blocker).unwrap();
        crate::service::initialize_usb(usb.path()).expect_err("the eDB write fails");
        assert!(!vendor_path(usb.path(), DbKind::Pdb).exists());
        assert!(!vendor_path(usb.path(), DbKind::Edb).exists());
        std::fs::remove_dir(blocker).unwrap();
        crate::service::initialize_usb(usb.path()).expect("retry initialization");
        assert!(vendor_path(usb.path(), DbKind::Pdb).exists());
        assert!(vendor_path(usb.path(), DbKind::Edb).exists());
    }

    #[test]
    fn session_writes_only_touched_databases() {
        let usb = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let _override = set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let pdb = vendor_path(usb.path(), DbKind::Pdb);
        let edb = vendor_path(usb.path(), DbKind::Edb);
        std::fs::create_dir_all(pdb.parent().unwrap()).unwrap();
        std::fs::write(&pdb, b"original pdb").unwrap();
        std::fs::write(&edb, b"original edb").unwrap();
        let databases = UsbDatabases::on_connect(usb.path()).unwrap();
        std::fs::write(databases.edb().unwrap(), b"unrelated dirty staging").unwrap();
        let mut session = UsbWriteSession::without_service(usb.path());
        session.write_pdb(b"edited pdb").unwrap();
        session.commit().unwrap();
        assert_eq!(std::fs::read(pdb).unwrap(), b"edited pdb");
        assert_eq!(std::fs::read(edb).unwrap(), b"original edb");
    }

    #[test]
    fn reconnect_discards_leftover_wal_even_when_main_file_is_clean() {
        let usb = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let _override = set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let edb = vendor_path(usb.path(), DbKind::Edb);
        std::fs::create_dir_all(edb.parent().unwrap()).unwrap();
        std::fs::write(&edb, b"original edb").unwrap();
        let databases = UsbDatabases::on_connect(usb.path()).unwrap();
        let wal = databases
            .edb()
            .unwrap()
            .with_file_name("exportLibrary.db-wal");
        std::fs::write(&wal, b"uncommitted frames").unwrap();
        let reconnected = UsbDatabases::on_connect(usb.path()).unwrap();
        assert!(!wal.exists());
        assert_eq!(
            std::fs::read(reconnected.edb().unwrap()).unwrap(),
            b"original edb"
        );
    }

    #[test]
    fn externally_created_database_is_not_overwritten() {
        let usb = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let _override = set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let mut session = UsbWriteSession::without_service(usb.path());
        session.write_pdb(b"new pdb").unwrap();
        let raw = vendor_path(usb.path(), DbKind::Pdb);
        std::fs::create_dir_all(raw.parent().unwrap()).unwrap();
        std::fs::write(&raw, b"externally created").unwrap();
        session
            .commit()
            .expect_err("the destination appeared after staging");
        assert_eq!(std::fs::read(raw).unwrap(), b"externally created");
    }

    #[test]
    fn session_commit_and_drop_work_with_and_without_staging() {
        for staged in [true, false] {
            let usb = tempfile::tempdir().unwrap();
            let cache = tempfile::tempdir().unwrap();
            let _override = set_cache_root_for_test(staged.then(|| cache.path().to_path_buf()));
            let pdb = vendor_path(usb.path(), DbKind::Pdb);
            let edb = vendor_path(usb.path(), DbKind::Edb);
            std::fs::create_dir_all(pdb.parent().unwrap()).unwrap();
            std::fs::write(&pdb, b"original pdb").unwrap();
            {
                let conn = rusqlite::Connection::open(&edb).unwrap();
                conn.execute_batch(
                    "CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(1);",
                )
                .unwrap();
            }
            UsbDatabases::on_connect(usb.path()).unwrap();
            for commit in [false, true] {
                let mut session = UsbWriteSession::without_service(usb.path());
                session.write_pdb(b"edited pdb").unwrap();
                session
                    .edb()
                    .unwrap()
                    .unwrap()
                    .execute_batch("UPDATE probe SET value=2;")
                    .unwrap();
                if staged {
                    assert_eq!(std::fs::read(&pdb).unwrap(), b"original pdb");
                    let conn = rusqlite::Connection::open(&edb).unwrap();
                    assert_eq!(
                        conn.query_row("SELECT value FROM probe", [], |r| r.get::<_, i64>(0))
                            .unwrap(),
                        1
                    );
                }
                if commit {
                    session.commit().unwrap();
                } else {
                    drop(session);
                }
                assert_eq!(
                    std::fs::read(&pdb).unwrap(),
                    if commit {
                        b"edited pdb".as_slice()
                    } else {
                        b"original pdb".as_slice()
                    }
                );
                let conn = rusqlite::Connection::open(&edb).unwrap();
                assert_eq!(
                    conn.query_row("SELECT value FROM probe", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    if commit { 2 } else { 1 }
                );
            }
        }
    }

    #[test]
    fn incomplete_checkpoint_refuses_both_database_writes() {
        let usb = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let _override = set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let raw = vendor_path(usb.path(), DbKind::Edb);
        std::fs::create_dir_all(raw.parent().unwrap()).unwrap();
        {
            let conn = rusqlite::Connection::open(&raw).unwrap();
            conn.execute_batch("CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(1);")
                .unwrap();
        }
        let pdb = vendor_path(usb.path(), DbKind::Pdb);
        std::fs::write(&pdb, b"original pdb").unwrap();
        UsbDatabases::on_connect(usb.path()).unwrap();
        let before = std::fs::read(&raw).unwrap();
        let mut session = UsbWriteSession::without_service(usb.path());
        session.write_pdb(b"changed pdb").unwrap();
        let conn = session.edb().unwrap().unwrap();
        conn.busy_timeout(std::time::Duration::from_millis(10))
            .unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        let reader = open_edb_rw(usb.path()).unwrap();
        reader.execute_batch("BEGIN").unwrap();
        let old: i64 = reader
            .query_row("SELECT value FROM probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(old, 1);
        conn.execute_batch("UPDATE probe SET value=2;").unwrap();
        let (busy, frames, checkpointed): (i64, i64, i64) = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_ne!(busy, 0);
        assert!(frames > checkpointed);
        let err = session
            .commit()
            .expect_err("the checkpoint must finish before either write");
        assert!(err.english().contains("couldn't finish writing"));
        assert_eq!(std::fs::read(&pdb).unwrap(), b"original pdb");
        assert_eq!(std::fs::read(&raw).unwrap(), before);
        let on_usb = rusqlite::Connection::open(&raw).unwrap();
        let actual: i64 = on_usb
            .query_row("SELECT value FROM probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(actual, 1, "the acknowledged edit was not copied to USB");
        drop(reader);
        let local = open_edb_rw(usb.path()).unwrap();
        let staged: i64 = local
            .query_row("SELECT value FROM probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            staged, 1,
            "failed changes cannot leak into the next session"
        );
    }

    #[test]
    fn reconnect_discards_uncommitted_staged_bytes() {
        let usb = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let _override = set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let raw = vendor_path(usb.path(), DbKind::Pdb);
        std::fs::create_dir_all(raw.parent().unwrap()).unwrap();
        std::fs::write(&raw, b"original").unwrap();
        let dbs = UsbDatabases::on_connect(usb.path()).unwrap();
        std::fs::write(
            dbs.pdb().unwrap(),
            b"uncommitted change left by an interrupted process",
        )
        .unwrap();
        let reconnect = UsbDatabases::on_connect(usb.path()).unwrap();
        assert_eq!(
            std::fs::read(reconnect.pdb().unwrap()).unwrap(),
            b"original"
        );
        UsbWriteSession::without_service(usb.path())
            .commit()
            .unwrap();
        assert_eq!(std::fs::read(raw).unwrap(), b"original");
    }

    #[test]
    fn external_deletion_is_a_write_conflict() {
        let usb = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let _override = set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let raw = vendor_path(usb.path(), DbKind::Pdb);
        std::fs::create_dir_all(raw.parent().unwrap()).unwrap();
        std::fs::write(&raw, b"original").unwrap();
        UsbDatabases::on_connect(usb.path()).unwrap();
        let mut session = UsbWriteSession::without_service(usb.path());
        session.write_pdb(b"changed").unwrap();
        std::fs::remove_file(&raw).unwrap();
        let error = session.commit().expect_err("deletion changed the source");
        assert!(error.english().contains("changed since it was last read"));
        assert!(!raw.exists());
    }
}
