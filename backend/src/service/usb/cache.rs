//! Caches of the selected USB's parsed databases: the staged PDB and eDB
//! parses, the resolved playlists and histories, file sizes and beat grids,
//! each keyed by the staged files' signatures so a changed stick re-reads.

use super::staging::{UsbDatabases, UsbDatabasesExt};
use crate::edb::open_edb_from_usb_root;
use crate::error::{BackendError, BackendResult};
use crate::models::{
    FetchUsbHistoriesRequest, FetchUsbPlaylistsRequest, UsbImportStats, UsbPlaylist, WarningEntry,
};
use crate::pdb_reader::{ParsedPdb, parse_pdb};
use crate::service::BackendService;
use crate::service::diagnostics::{OutdatedBeatGrid, detect_outdated_beat_grids};
use crate::service::track_matching::ContentFingerprint;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

/// Size + mtime of a file, a cheap change signal for the locally-staged
/// `export.pdb` / `exportLibrary.db` copies. `stage_pdb` / `stage_edb`
/// re-sync those from the device whenever the USB-side file changed, so a
/// matching staged signature means a cached parse is still current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedFileSig {
    size: u64,
    mtime_ns: i128,
}

impl StagedFileSig {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let dur = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Some(Self {
            size: meta.len(),
            mtime_ns: dur.as_nanos() as i128,
        })
    }
}

/// Cache key for one parsed + resolved USB stick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsbParseCacheKey {
    usb_root: std::path::PathBuf,
    pdb: Option<StagedFileSig>,
    edb: Option<StagedFileSig>,
}

/// The resolved-but-not-yet-materialized output of a USB playlist import:
/// everything that is a pure function of the staged PDB + eDB. The paginated
/// `fetch_usb_playlist_tracks` reuses this instead of re-parsing the whole
/// stick on every page / search / sort / scroll.
#[derive(Debug, Clone)]
pub(crate) struct CachedUsbPlaylists {
    pub(in crate::service) items: Vec<UsbPlaylist>,
    /// The stick's folders and playlists (`items` by id).
    pub(in crate::service) tree: Vec<super::playlist_tree::UsbTreeNode>,
    pub(super) stats: UsbImportStats,
    pub(super) warnings: Vec<WarningEntry>,
}

/// One-entry parse cache: `(key, resolved playlists)`, guarded for sharing
/// across `BackendService` clones.
pub(crate) type UsbPlaylistsCache =
    std::sync::Mutex<Option<(UsbParseCacheKey, Arc<CachedUsbPlaylists>)>>;

/// One-entry cache of a staged eDB's full read (`EdbRead::ALL`), keyed by its
/// path and signature.
pub(crate) type EdbLibraryCache = std::sync::Mutex<
    Option<(
        std::path::PathBuf,
        StagedFileSig,
        Arc<crate::edb::EdbLibrary>,
    )>,
>;

/// A staged PDB parsed, with the page statistics the diagnostics report.
#[derive(Debug)]
pub(crate) struct StagedPdbParse {
    pub(crate) parsed: Arc<crate::pdb_reader::ParsedPdb>,
    pub(crate) diagnostics: crate::pdb_reader::PdbDiagnostics,
}

impl StagedPdbParse {
    pub(crate) fn read(pdb_path: &Path) -> BackendResult<Self> {
        let (parsed, diagnostics) = crate::pdb_reader::parse_pdb_with_diagnostics(pdb_path)?;
        Ok(Self {
            parsed: Arc::new(parsed),
            diagnostics,
        })
    }
}

/// One-entry cache of a staged PDB's parse, keyed by its path and signature.
pub(crate) type ParsedPdbCache =
    std::sync::Mutex<Option<(std::path::PathBuf, StagedFileSig, Arc<StagedPdbParse>)>>;

/// One-entry parse cache for USB histories.
pub(crate) type UsbHistoriesCache =
    std::sync::Mutex<Option<(UsbParseCacheKey, Arc<crate::models::FetchUsbHistoriesData>)>>;

/// Every track either staged database lists (`UsbTrackFacts`).
#[derive(Debug, Default)]
pub(crate) struct UsbTrackSizes {
    /// By `content_fingerprint_key` (recorded size, title, artist), so a local
    /// track is found by its data wherever its file landed.
    pub by_fingerprint: HashSet<ContentFingerprint>,
    /// Recorded size by `usb_track_size_key` of the track file.
    pub by_path: HashMap<String, u64>,
}

impl UsbTrackSizes {
    /// Whether a local track of `size` bytes is already on the USB: by its
    /// fingerprint, or, for a track without one (no size or a blank title),
    /// by the USB-relative path it would be exported to.
    pub(crate) fn holds(
        &self,
        fingerprint: Option<&ContentFingerprint>,
        size: u64,
        target_relative: impl FnOnce() -> Option<String>,
    ) -> bool {
        match fingerprint {
            Some(fingerprint) => self.by_fingerprint.contains(fingerprint),
            None => {
                target_relative()
                    .and_then(|rel| self.by_path.get(&usb_track_size_key(&rel)).copied())
                    == Some(size)
            }
        }
    }
}

/// One-entry cache of `UsbTrackSizes`, keyed by the stick and both staged
/// databases' signatures, so the playlist heading's export estimate doesn't
/// re-read them on every page load.
pub(crate) type UsbTrackSizesCache = std::sync::Mutex<
    Option<(
        std::path::PathBuf,
        (Option<StagedFileSig>, Option<StagedFileSig>),
        Arc<UsbTrackSizes>,
    )>,
>;

/// One-entry cache of `detect_outdated_beat_grids`: keyed by the staged
/// PDB's path and signature and the export log's signature (which tracks are
/// candidates), so selecting a stick again doesn't re-read its analysis files.
/// A change to the files themselves comes with a write this app makes (which
/// clears it) or a rewritten PDB.
pub(crate) type BeatGridCache = std::sync::Mutex<
    Option<(
        std::path::PathBuf,
        (StagedFileSig, Option<StagedFileSig>),
        Arc<Option<Vec<OutdatedBeatGrid>>>,
    )>,
>;

/// Lookup key for a file under the USB root: its root-relative path with `/`
/// separators, lowercased (FAT32 and exFAT names are case-insensitive).
pub(crate) fn usb_track_size_key(relative_path: &str) -> String {
    relative_path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_lowercase()
}

impl BackendService {
    /// Staged-file signature key for the parse cache. Also forces the local
    /// staging copies to be in sync, so the subsequent `stage_pdb` /
    /// `open_edb_from_usb_root` calls in the resolve are no-ops.
    pub(crate) fn usb_parse_cache_key(&self, usb_root: &Path) -> UsbParseCacheKey {
        let databases = UsbDatabases::present(usb_root).ok().flatten();
        let pdb = databases.pdb().and_then(StagedFileSig::of);
        let edb = databases.edb().and_then(StagedFileSig::of);
        UsbParseCacheKey {
            usb_root: usb_root.to_path_buf(),
            pdb,
            edb,
        }
    }

    /// Drop the cached USB parse. Called by every command that writes back to
    /// the stick; the staged-signature key catches most changes on its own,
    /// this is the belt-and-suspenders.
    pub(crate) fn invalidate_usb_parse_cache(&self) {
        if let Ok(mut guard) = self.beat_grid_cache.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.usb_playlists_cache.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.usb_histories_cache.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.usb_track_sizes_cache.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.edb_library_cache.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.parsed_pdb_cache.lock() {
            *guard = None;
        }
    }

    /// The parse of the staged PDB at `pdb_path`, from the cache while the file is unchanged
    /// (same size and modification time); else parsed and kept. Every reader of a staged PDB
    /// that doesn't change it goes through here, so one stick is parsed once.
    pub(crate) fn parsed_staged_pdb(&self, pdb_path: &Path) -> BackendResult<Arc<StagedPdbParse>> {
        let sig = StagedFileSig::of(pdb_path);
        if let Some(sig) = &sig
            && let Ok(guard) = self.parsed_pdb_cache.lock()
            && let Some((path, cached_sig, parsed)) = guard.as_ref()
            && path == pdb_path
            && cached_sig == sig
        {
            return Ok(Arc::clone(parsed));
        }
        let parsed = Arc::new(StagedPdbParse::read(pdb_path)?);
        if let Some(sig) = sig
            && let Ok(mut guard) = self.parsed_pdb_cache.lock()
        {
            *guard = Some((pdb_path.to_path_buf(), sig, Arc::clone(&parsed)));
        }
        Ok(parsed)
    }

    /// The names of the playlists on `usb_root`, for the export status. From the parse
    /// caches while the staged databases are unchanged, so a playlist change doesn't read
    /// the USB again; else a names-only read of the staged PDB and eDB.
    pub(crate) fn usb_playlist_names(
        &self,
        usb_root: &Path,
    ) -> Arc<crate::service::export::UsbPlaylistNames> {
        let parsed = UsbDatabases::present(usb_root)
            .ok()
            .flatten()
            .pdb()
            .and_then(|path| self.parsed_staged_pdb(path).ok());
        let edb = self.staged_edb(usb_root, None, &mut Vec::new());
        Arc::new(
            crate::service::export::usb_playlist_names_for_export_compare(
                parsed.as_deref().map(|staged| &*staged.parsed),
                edb.iter().flat_map(|library| {
                    library
                        .playlists
                        .values()
                        .map(|playlist| playlist.path.clone())
                }),
            ),
        )
    }

    /// The staged eDB's tracks and playlists (`EdbRead::ALL`, paths as stored), from the
    /// cache while the file is unchanged; else read, through `conn` when the caller has it
    /// open (opening it adds its warnings to `warnings`). `None` without a readable eDB.
    /// Every reader of the staged eDB's tracks or playlists that doesn't change it goes
    /// through here, so one stick is read once.
    pub(crate) fn staged_edb(
        &self,
        usb_root: &Path,
        conn: Option<&rusqlite::Connection>,
        warnings: &mut Vec<WarningEntry>,
    ) -> Option<Arc<crate::edb::EdbLibrary>> {
        let edb_path = UsbDatabases::present(usb_root)
            .ok()
            .flatten()
            .edb()?
            .to_path_buf();
        let sig = StagedFileSig::of(&edb_path)?;
        if let Ok(guard) = self.edb_library_cache.lock()
            && let Some((path, cached_sig, library)) = guard.as_ref()
            && *path == edb_path
            && *cached_sig == sig
        {
            return Some(Arc::clone(library));
        }
        let owned_conn;
        let conn = match conn {
            Some(conn) => conn,
            None => {
                owned_conn = open_edb_from_usb_root(usb_root, warnings)?;
                &owned_conn
            }
        };
        let library = Arc::new(crate::edb::read_edb(
            conn,
            usb_root,
            crate::edb::EdbRead::ALL,
        )?);
        if let Ok(mut guard) = self.edb_library_cache.lock() {
            *guard = Some((edb_path, sig, Arc::clone(&library)));
        }
        Some(library)
    }

    /// [`detect_outdated_beat_grids`] for the stick whose staged PDB is
    /// `pdb_path`, from the cache while that PDB and the export log are
    /// unchanged.
    pub(crate) fn outdated_beat_grids(
        &self,
        usb_root: &Path,
        pdb_path: &Path,
        parsed: &ParsedPdb,
    ) -> Arc<Option<Vec<OutdatedBeatGrid>>> {
        let sig = StagedFileSig::of(pdb_path).map(|pdb| {
            (
                pdb,
                StagedFileSig::of(&crate::service::export_log::export_log_path(usb_root)),
            )
        });
        if let Some(sig) = sig.as_ref()
            && let Ok(guard) = self.beat_grid_cache.lock()
            && let Some((path, cached_sig, found)) = guard.as_ref()
            && path == pdb_path
            && cached_sig == sig
        {
            return Arc::clone(found);
        }
        let found = Arc::new(detect_outdated_beat_grids(usb_root, parsed));
        if let Some(sig) = sig
            && let Ok(mut guard) = self.beat_grid_cache.lock()
        {
            *guard = Some((pdb_path.to_path_buf(), sig, Arc::clone(&found)));
        }
        found
    }

    /// The staged databases' track files and sizes (see `UsbTrackSizes`).
    /// Reads only the local staged copies; staging touches the drive only
    /// when this stick's databases haven't been staged and verified yet.
    pub(crate) fn usb_track_sizes(&self, usb_root: &Path) -> BackendResult<Arc<UsbTrackSizes>> {
        let databases = UsbDatabases::of(usb_root)?;
        let sig = (
            databases.pdb().and_then(StagedFileSig::of),
            databases.edb().and_then(StagedFileSig::of),
        );
        if let Ok(guard) = self.usb_track_sizes_cache.lock()
            && let Some((path, cached_sig, sizes)) = guard.as_ref()
            && path == usb_root
            && *cached_sig == sig
        {
            return Ok(Arc::clone(sizes));
        }
        let staged = databases
            .pdb()
            .map(|path| self.parsed_staged_pdb(path))
            .transpose()?;
        // Its open warnings are the USB view's to report, not the estimate's.
        let edb = databases.open_edb(&mut Vec::new());
        let facts = crate::service::usb::track_facts::UsbTrackFacts::read(
            staged.as_deref().map(|staged| &*staged.parsed),
            edb.as_ref(),
        );
        let mut sizes = UsbTrackSizes::default();
        for fact in facts.by_path.into_values() {
            let Some(size) = fact.size else {
                continue;
            };
            if let Some(fingerprint) = fact.fingerprint {
                sizes.by_fingerprint.insert(fingerprint);
            }
            sizes
                .by_path
                .insert(usb_track_size_key(&fact.path), u64::from(size));
        }
        let sizes = Arc::new(sizes);
        if let Ok(mut guard) = self.usb_track_sizes_cache.lock() {
            *guard = Some((usb_root.to_path_buf(), sig, Arc::clone(&sizes)));
        }
        Ok(sizes)
    }

    pub(super) fn usb_playlists_cache_hit(
        &self,
        key: &UsbParseCacheKey,
    ) -> Option<Arc<CachedUsbPlaylists>> {
        let guard = self.usb_playlists_cache.lock().ok()?;
        guard
            .as_ref()
            .filter(|(cached_key, _)| cached_key == key)
            .map(|(_, value)| Arc::clone(value))
    }

    pub(super) fn usb_histories_cache_hit(
        &self,
        key: &UsbParseCacheKey,
    ) -> Option<Arc<crate::models::FetchUsbHistoriesData>> {
        let guard = self.usb_histories_cache.lock().ok()?;
        guard
            .as_ref()
            .filter(|(cached_key, _)| cached_key == key)
            .map(|(_, value)| Arc::clone(value))
    }

    /// The resolved USB histories for a stick, from cache or a fresh parse.
    pub(in crate::service) fn usb_histories_resolved(
        &self,
        usb_root: &Path,
    ) -> BackendResult<Arc<crate::models::FetchUsbHistoriesData>> {
        let key = self.usb_parse_cache_key(usb_root);
        if let Some(hit) = self.usb_histories_cache_hit(&key) {
            return Ok(hit);
        }
        // The no-PDB branch returns without caching, so fall back to the
        // fresh result when the read-back misses.
        let fresh = self.fetch_usb_histories_with_progress(
            FetchUsbHistoriesRequest {
                usb_root: Some(usb_root.to_string_lossy().into_owned()),
            },
            |_, _, _| {},
        )?;
        Ok(self
            .usb_histories_cache_hit(&key)
            .unwrap_or_else(|| Arc::new(fresh)))
    }

    /// The resolved (pre-materialization) playlists for a stick, from cache or
    /// a fresh parse. Used by `fetch_usb_playlist_tracks` to avoid re-parsing
    /// the whole stick per page.
    pub(in crate::service) fn usb_playlists_resolved(
        &self,
        usb_root: &Path,
    ) -> BackendResult<Arc<CachedUsbPlaylists>> {
        let key = self.usb_parse_cache_key(usb_root);
        if let Some(hit) = self.usb_playlists_cache_hit(&key) {
            return Ok(hit);
        }
        // Populate the cache via the full resolve, then read it back. This
        // path always stores to the cache, so a miss here means the staged
        // signature shifted mid-call -- rare; a retry succeeds.
        self.fetch_usb_playlists_with_progress(
            FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().into_owned()),
            },
            |_, _, _| {},
        )?;
        self.usb_playlists_cache_hit(&key).ok_or_else(|| {
            BackendError::Internal("USB playlist parse cache was not populated".to_string())
        })
    }
}

/// Parse `export.pdb` through the local HDD staging layer instead of
/// reading it fresh off the USB mount every call. Falls back to reading the
/// USB path directly when staging hasn't been initialized (CLI tools,
/// tests that don't construct a `BackendService`).
pub(crate) fn parse_staged_pdb(usb_root: &Path) -> BackendResult<crate::pdb_reader::ParsedPdb> {
    parse_pdb(crate::service::usb::staging::UsbDatabases::of(usb_root)?.require_pdb()?)
}
