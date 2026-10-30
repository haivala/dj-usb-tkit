//! Source roots: the library's source folders and its filter, analysis
//! status, relocation and removal.

use super::*;

pub(super) fn sanitize_source_roots(source_roots: Vec<String>) -> Vec<String> {
    let mut roots = Vec::<String>::new();
    for root in source_roots {
        let trimmed = root.trim().to_string();
        if trimmed.is_empty() || roots.iter().any(|existing| existing == &trimmed) {
            continue;
        }
        roots.push(trimmed);
    }
    roots
}

pub(super) fn source_root_is_usable(root: &Path) -> bool {
    root.is_dir()
}

/// Where `root` really is, following symlinks; a missing folder is taken as
/// written. With `Data` linking to `/mnt/raid`, a source `/mnt/raid/music/a`
/// lies inside a source `/home/me/Data/music`.
fn resolved_source_root(root: &str) -> String {
    std::fs::canonicalize(root)
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| root.trim().to_string())
}

/// For each of `roots`, the index of the nearest other root it lies inside
/// (resolved through symlinks), if any. Two roots naming the same folder
/// don't nest: the first one keeps it, the later one is inside it.
fn source_root_parents(roots: &[String]) -> Vec<Option<usize>> {
    let resolved: Vec<String> = roots
        .iter()
        .map(|root| resolved_source_root(root))
        .collect();
    resolved
        .iter()
        .enumerate()
        .map(|(index, child)| {
            resolved
                .iter()
                .enumerate()
                .filter(|&(other, parent)| {
                    other != index
                        && browse_path_matches_root(child, parent)
                        && (browse_path_key(child) != browse_path_key(parent) || other < index)
                })
                .max_by_key(|(_, parent)| parent.len())
                .map(|(other, _)| other)
        })
        .collect()
}

/// The indexes of `parents` depth first: each top-level folder in the order
/// added, followed by the folders inside it, in the order added.
fn source_tree_order(parents: &[Option<usize>]) -> Vec<usize> {
    fn visit(index: usize, parents: &[Option<usize>], order: &mut Vec<usize>) {
        order.push(index);
        for child in (0..parents.len()).filter(|&child| parents[child] == Some(index)) {
            visit(child, parents, order);
        }
    }
    let mut order = Vec::with_capacity(parents.len());
    for root in (0..parents.len()).filter(|&index| parents[index].is_none()) {
        visit(root, parents, &mut order);
    }
    order
}

/// A nested folder's chip text: its path inside `parent`, ending in `/`.
fn nested_source_label(root: &str, parent: &str) -> String {
    let child = resolved_source_root(root);
    let parent = resolved_source_root(parent);
    match relative_path_under_source_root(&child, &parent) {
        Some(relative) if !relative.is_empty() => format!("{}/", relative.trim_end_matches('/')),
        _ => root.to_string(),
    }
}

pub(crate) fn normalize_source_root_for_matching(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

pub(super) fn relative_path_under_source_root(
    file_path: &str,
    source_root: &str,
) -> Option<String> {
    let normalized_path = file_path.trim().replace('\\', "/");
    let normalized_root = source_root.trim().replace('\\', "/");
    let root = normalized_root.trim_end_matches('/');
    if root.is_empty() {
        return None;
    }

    let path_key = normalized_path.to_ascii_lowercase();
    let root_key = root.to_ascii_lowercase();
    if path_key == root_key {
        return Some(String::new());
    }
    let prefix = format!("{root_key}/");
    if path_key.starts_with(&prefix) {
        return Some(normalized_path[root.len() + 1..].to_string());
    }
    None
}

/// Configured source folders that are currently missing (unplugged drive,
/// moved directory) *and* actually hold at least one of `track_file_paths`.
/// Empty when every source folder a playlist track lives under is present.
///
/// The frontend used to run this prefix match itself (`trackPathMatchesAnyRoot`
/// / `playlistTracksAffectedByMissingRoots`) over only the loaded page of a
/// playlist; hoisting it into the export path checks the whole playlist and
/// keeps the rule in one place.
pub(crate) fn export_blocking_missing_source_roots<I, S>(
    conn: &rusqlite::Connection,
    track_file_paths: I,
) -> BackendResult<Vec<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let missing: Vec<String> = configured_source_roots(conn)?
        .into_iter()
        .filter(|root| !source_root_is_usable(Path::new(root)))
        .collect();
    if missing.is_empty() {
        return Ok(Vec::new());
    }
    let paths: Vec<String> = track_file_paths
        .into_iter()
        .map(|path| path.as_ref().to_string())
        .collect();
    Ok(missing
        .into_iter()
        .filter(|root| {
            paths
                .iter()
                .any(|path| relative_path_under_source_root(path, root).is_some())
        })
        .collect())
}

pub(super) fn relocated_source_path(new_root: &Path, relative_path: &str) -> PathBuf {
    let trimmed = relative_path.trim_matches('/');
    if trimmed.is_empty() {
        return new_root.to_path_buf();
    }
    let mut out = new_root.to_path_buf();
    for segment in trimmed.split('/') {
        if !segment.is_empty() {
            out.push(segment);
        }
    }
    out
}

/// The single source of truth for "this track has its core analysis": a
/// non-empty waveform-peaks path, a positive BPM, and a positive duration.
/// DB fields only -- deliberately no filesystem check (the export gate's
/// DAT/EXT/2EX bundle verification is separate, see
/// `service::export::has_required_analysis`). Every wrapper below and the
/// `Track::analysis_ready` flag route through this.
pub(crate) fn has_core_analysis_fields(
    waveform_peaks_path: Option<&str>,
    bpm: Option<f64>,
    duration_ms: Option<u64>,
) -> bool {
    let has_waveform_path = waveform_peaks_path
        .map(|path| !path.trim().is_empty())
        .unwrap_or(false);
    let has_bpm = bpm.map(|bpm| bpm > 0.0).unwrap_or(false);
    let has_duration = duration_ms.map(|duration| duration > 0).unwrap_or(false);
    has_waveform_path && has_bpm && has_duration
}

pub(super) fn track_has_core_analysis_for_source_status(track: &Track) -> bool {
    has_core_analysis_fields(
        track.waveform_peaks_path.as_deref(),
        track.bpm,
        track.duration_ms,
    )
}

impl BackendService {
    pub fn get_library_sources(&self) -> BackendResult<LibrarySourcesData> {
        let conn = self.db.connect()?;
        let mut sources = LibrarySources::load(&conn)?;
        // A library whose database is gone has no chip to switch it off
        // with: its imported tracks leave the library until it's found and
        // switched on again.
        let rekordbox_gone = sources.rekordbox_db && !detect_external_rekordbox_db_util().found;
        let mixxx_gone = sources.mixxx_db && !super::mixxx_import::detect_external_mixxx_db().found;
        if rekordbox_gone || mixxx_gone {
            sources.rekordbox_db &= !rekordbox_gone;
            sources.mixxx_db &= !mixxx_gone;
            sources.save(&conn)?;
        }
        self.library_sources_data(&conn, &sources)
    }

    /// The source chips' view of `sources`. Each folder's analyzed status is
    /// its own (every track in it, switched on or not); same scan and count
    /// as `browse_source_files`, without the search, sort and paging.
    pub(super) fn library_sources_data(
        &self,
        conn: &rusqlite::Connection,
        sources: &LibrarySources,
    ) -> BackendResult<LibrarySourcesData> {
        let present: Vec<String> = sources
            .roots
            .iter()
            .filter(|root| source_root_is_usable(Path::new(root)))
            .cloned()
            .collect();
        let tracks = if present.is_empty() {
            Vec::new()
        } else {
            self.compute_all_library_tracks(conn, &present, ExternalLibraries::default())?
        };
        let parents = source_root_parents(&sources.roots);
        let folders = source_tree_order(&parents)
            .into_iter()
            .map(|index| {
                let root = &sources.roots[index];
                let parent = parents[index].map(|parent| sources.roots[parent].clone());
                let missing = !present.contains(root);
                SourceFolder {
                    path: root.clone(),
                    label: parent
                        .as_deref()
                        .map_or_else(|| root.clone(), |parent| nested_source_label(root, parent)),
                    parent,
                    enabled: sources.is_enabled(root),
                    missing,
                    fully_analyzed: !missing && folder_fully_analyzed(root, &tracks),
                }
            })
            .collect();
        Ok(LibrarySourcesData {
            folders,
            rekordbox_db_enabled: sources.rekordbox_db,
            mixxx_db_enabled: sources.mixxx_db,
            ever_configured: sources.ever_configured(),
        })
    }

    /// Adds the folders not configured yet, switched on. Indexing them is
    /// `scan_library`'s job.
    pub fn add_source_folders(
        &self,
        req: AddSourceFoldersRequest,
    ) -> BackendResult<AddSourceFoldersData> {
        let conn = self.db.connect()?;
        let mut sources = LibrarySources::load(&conn)?;
        let mut added = Vec::new();
        for path in sanitize_source_roots(req.paths) {
            if !sources.roots.contains(&path) {
                sources.roots.push(path.clone());
                added.push(path);
            }
        }
        if !added.is_empty() {
            sources.ever_configured = true;
            sources.save(&conn)?;
        }
        Ok(AddSourceFoldersData {
            sources: self.library_sources_data(&conn, &sources)?,
            added,
        })
    }

    pub fn set_source_folder_enabled(
        &self,
        req: SetSourceFolderEnabledRequest,
    ) -> BackendResult<LibrarySourcesData> {
        let conn = self.db.connect()?;
        let mut sources = LibrarySources::load(&conn)?;
        if !sources.roots.contains(&req.path) {
            return Err(BackendError::NotFound(format!(
                "source folder not configured: {}",
                req.path
            )));
        }
        sources.enabled.insert(req.path, req.enabled);
        sources.save(&conn)?;
        self.library_sources_data(&conn, &sources)
    }

    /// Shows or hides an imported library's tracks. Switching one on counts
    /// as setting up a source (an import does it).
    pub fn set_external_library_enabled(
        &self,
        req: SetExternalLibraryEnabledRequest,
    ) -> BackendResult<LibrarySourcesData> {
        let conn = self.db.connect()?;
        let mut sources = LibrarySources::load(&conn)?;
        match req.library {
            ExternalLibrary::Rekordbox => sources.rekordbox_db = req.enabled,
            ExternalLibrary::Mixxx => sources.mixxx_db = req.enabled,
        }
        sources.ever_configured |= req.enabled;
        sources.save(&conn)?;
        self.library_sources_data(&conn, &sources)
    }

    pub fn relocate_source_root(
        &self,
        req: RelocateSourceRootRequest,
    ) -> BackendResult<RelocateSourceRootData> {
        let old_root = req.old_root.trim().to_string();
        let new_root = req.new_root.trim().to_string();
        if old_root.is_empty() || new_root.is_empty() {
            return Err(BackendError::Validation(
                "oldRoot and newRoot must not be empty".to_string(),
            ));
        }
        // Compared as paths (component-wise, so a trailing separator doesn't
        // count) and case-sensitively: on Linux `music` and `Music` are
        // different folders, and a root renamed to a new case is a real move.
        if Path::new(&old_root) == Path::new(&new_root) {
            return Err(BackendError::Validation(
                "newRoot must be different from oldRoot".to_string(),
            ));
        }
        let new_root_path = PathBuf::from(&new_root);
        if !new_root_path.exists() || !new_root_path.is_dir() {
            return Err(BackendError::Validation(format!(
                "newRoot does not exist or is not a directory: {new_root}"
            )));
        }

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let tracks = {
            let mut stmt = tx.prepare("SELECT id, file_path FROM tracks")?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };

        let mut matched = 0usize;
        let mut updated = 0usize;
        let mut unchanged = 0usize;
        let mut missing_at_new_root = 0usize;
        let mut conflicts = 0usize;
        let timestamp = now();

        for (track_id, old_file_path) in tracks {
            let Some(relative_path) = relative_path_under_source_root(&old_file_path, &old_root)
            else {
                continue;
            };
            matched += 1;

            let new_file_path = relocated_source_path(&new_root_path, &relative_path);
            let new_file_path_string = new_file_path.to_string_lossy().to_string();
            if new_file_path_string == old_file_path {
                unchanged += 1;
                continue;
            }
            if !new_file_path.is_file() {
                missing_at_new_root += 1;
                continue;
            }

            let existing_id = tx
                .query_row(
                    "SELECT id FROM tracks WHERE file_path = ?1 LIMIT 1",
                    params![&new_file_path_string],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if existing_id
                .as_deref()
                .is_some_and(|existing| existing != track_id)
            {
                conflicts += 1;
                continue;
            }

            tx.execute(
                "UPDATE tracks SET file_path = ?1, updated_at = ?2 WHERE id = ?3",
                params![&new_file_path_string, &timestamp, &track_id],
            )?;
            updated += 1;
        }

        let mut sources = LibrarySources::load(&tx)?;
        sources.relocate(&old_root, &new_root, missing_at_new_root + conflicts == 0);
        sources.save(&tx)?;
        tx.commit()?;
        self.forget_source_folder_listings();

        Ok(RelocateSourceRootData {
            old_root,
            new_root,
            matched,
            updated,
            unchanged,
            missing_at_new_root,
            conflicts,
            unresolved: missing_at_new_root + conflicts,
            sources: self.library_sources_data(&conn, &sources)?,
        })
    }

    /// Removes the folder from the library's sources and its tracks from the
    /// library. A track another source folder still covers (a parent of the
    /// removed one, or a subfolder of it) stays.
    pub fn remove_source_folder(
        &self,
        req: RemoveSourceFolderRequest,
    ) -> BackendResult<RemoveSourceFolderData> {
        let root = req.path.trim().to_string();
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let mut sources = LibrarySources::load(&tx)?;
        let Some(index) = sources.roots.iter().position(|existing| existing == &root) else {
            return Err(BackendError::NotFound(format!(
                "source folder not configured: {root}"
            )));
        };
        sources.roots.remove(index);
        sources.save(&tx)?;

        let escaped = root
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let slash_like = format!("{escaped}/%");
        let backslash_like = format!("{escaped}\\\\%");
        let rows = tx
            .prepare(
                r#"
                SELECT id, file_path FROM tracks
                WHERE file_path = ?1
                   OR file_path LIKE ?2 ESCAPE '\'
                   OR file_path LIKE ?3 ESCAPE '\'
                "#,
            )?
            .query_map(params![root, slash_like, backslash_like], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut removed = 0usize;
        for (id, file_path) in rows {
            if sources
                .roots
                .iter()
                .any(|remaining| browse_path_matches_root(&file_path, remaining))
            {
                continue;
            }
            removed += tx.execute("DELETE FROM tracks WHERE id = ?1", params![id])?;
        }
        tx.commit()?;
        self.forget_source_folder_listings();
        Ok(RemoveSourceFolderData {
            sources: self.library_sources_data(&conn, &sources)?,
            removed,
        })
    }
}

/// Whether every one of `tracks` under `root` has its core analysis, and
/// there is at least one. Callers pass the full, unfiltered track set so a
/// folder's status never depends on a search query.
pub(super) fn folder_fully_analyzed(root: &str, tracks: &[Track]) -> bool {
    let mut under_root = tracks
        .iter()
        .filter(|track| browse_path_matches_root(&track.file_path, root))
        .peekable();
    under_root.peek().is_some() && under_root.all(track_has_core_analysis_for_source_status)
}

pub(super) fn source_root_keys_equal(left: &str, right: &str) -> bool {
    normalize_source_root_for_matching(left) == normalize_source_root_for_matching(right)
}

/// The library's sources as stored in `app_settings` (under the `ui_` keys
/// the frontend once wrote itself): the folders in the order added, their
/// switches (a folder without one is on), the imported libraries' switches,
/// and whether a source was ever set up.
#[derive(Debug, Clone, Default)]
pub(crate) struct LibrarySources {
    pub(crate) roots: Vec<String>,
    pub(crate) enabled: std::collections::BTreeMap<String, bool>,
    pub(crate) rekordbox_db: bool,
    pub(crate) mixxx_db: bool,
    pub(crate) ever_configured: bool,
}

impl LibrarySources {
    /// Tolerates missing keys (fresh install) and malformed values.
    pub(crate) fn load(conn: &rusqlite::Connection) -> BackendResult<Self> {
        let read = |key: &str| -> BackendResult<Option<String>> {
            Ok(conn
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?)
        };
        let roots = read(SETTING_UI_SOURCE_ROOTS)?
            .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
            .map(sanitize_source_roots)
            .unwrap_or_default();
        let enabled = read(SETTING_UI_SOURCE_ROOT_ENABLED)?
            .and_then(|value| {
                serde_json::from_str::<std::collections::BTreeMap<String, bool>>(&value).ok()
            })
            .unwrap_or_default();
        let flag = |value: Option<String>| value.as_deref() == Some("1");
        Ok(Self {
            roots,
            enabled,
            rekordbox_db: flag(read(SETTING_UI_REKORDBOX_DB_ENABLED)?),
            mixxx_db: flag(read(SETTING_UI_MIXXX_DB_ENABLED)?),
            ever_configured: flag(read(SETTING_UI_SOURCES_EVER_CONFIGURED)?),
        })
    }

    /// Writes every key; the switches only for the configured folders.
    pub(crate) fn save(&self, conn: &rusqlite::Connection) -> BackendResult<()> {
        let enabled: std::collections::BTreeMap<&str, bool> = self
            .roots
            .iter()
            .map(|root| (root.as_str(), self.is_enabled(root)))
            .collect();
        let encode = |value: Result<String, serde_json::Error>| {
            value.map_err(|err| {
                BackendError::Internal(format!("failed to encode library sources: {err}"))
            })
        };
        let flag = |on: bool| if on { "1" } else { "0" }.to_string();
        for (key, value) in [
            (
                SETTING_UI_SOURCE_ROOTS,
                encode(serde_json::to_string(&self.roots))?,
            ),
            (
                SETTING_UI_SOURCE_ROOT_ENABLED,
                encode(serde_json::to_string(&enabled))?,
            ),
            (SETTING_UI_REKORDBOX_DB_ENABLED, flag(self.rekordbox_db)),
            (SETTING_UI_MIXXX_DB_ENABLED, flag(self.mixxx_db)),
            (
                SETTING_UI_SOURCES_EVER_CONFIGURED,
                flag(self.ever_configured()),
            ),
        ] {
            conn.execute(
                r#"
                INSERT INTO app_settings (key, value, updated_at)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
                "#,
                params![key, value, now()],
            )?;
        }
        Ok(())
    }

    pub(crate) fn is_enabled(&self, root: &str) -> bool {
        self.enabled.get(root).copied().unwrap_or(true)
    }

    pub(crate) fn ever_configured(&self) -> bool {
        self.ever_configured || !self.roots.is_empty() || self.rekordbox_db || self.mixxx_db
    }

    /// The folders the library lists: switched on and present on disk.
    pub(crate) fn listed_roots(&self) -> Vec<String> {
        self.roots
            .iter()
            .filter(|root| self.is_enabled(root) && source_root_is_usable(Path::new(root)))
            .cloned()
            .collect()
    }

    pub(crate) fn external(&self) -> ExternalLibraries {
        ExternalLibraries::from_flags(self.rekordbox_db, self.mixxx_db)
    }

    /// `old_root` moved to `new_root`. Once all its tracks moved
    /// (`fully_moved`) the new folder takes the old one's place (or the old
    /// one just goes, when the new one is configured already); otherwise the
    /// old folder stays, for the tracks left behind, and the new one is
    /// added. The new folder takes the old one's switch.
    pub(crate) fn relocate(&mut self, old_root: &str, new_root: &str, fully_moved: bool) {
        let old_index = self
            .roots
            .iter()
            .position(|root| source_root_keys_equal(root, old_root));
        // Not the old root itself: the keys ignore case, and a case-only
        // rename (`music` → `Music`) is a real move on Linux.
        let has_new_root = self.roots.iter().enumerate().any(|(index, root)| {
            Some(index) != old_index && source_root_keys_equal(root, new_root)
        });
        let old_enabled = old_index.is_none_or(|index| self.is_enabled(&self.roots[index]));
        match old_index {
            Some(index) if fully_moved => {
                if has_new_root {
                    self.roots.remove(index);
                } else {
                    self.roots[index] = new_root.to_string();
                }
            }
            _ if !has_new_root => self.roots.push(new_root.to_string()),
            _ => {}
        }
        self.enabled
            .entry(new_root.to_string())
            .or_insert(old_enabled);
    }
}
