//! Source roots: status, analysis, relocation and removal.

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

pub(super) fn source_root_status(root: &str) -> SourceRootStatus {
    let path = Path::new(root);
    SourceRootStatus {
        source_root: root.to_string(),
        exists: path.exists(),
        is_dir: path.is_dir(),
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
    /// Each source folder's analyzed status on its own (every track in it,
    /// enabled in the library filter or not) -- the source chips' green state.
    /// Same scan and count as `browse_source_files`, without the search,
    /// sort and paging a browse does.
    pub fn get_source_root_analysis(
        &self,
        req: GetSourceRootAnalysisRequest,
    ) -> BackendResult<GetSourceRootAnalysisData> {
        let source_roots = sanitize_source_roots(req.source_roots);
        if source_roots.is_empty() {
            return Ok(GetSourceRootAnalysisData { items: Vec::new() });
        }
        let conn = self.db.connect()?;
        let tracks =
            self.compute_all_library_tracks(&conn, &source_roots, ExternalLibraries::default())?;
        Ok(GetSourceRootAnalysisData {
            items: source_root_analysis(&source_roots, &tracks),
        })
    }

    pub fn check_source_roots(
        &self,
        req: CheckSourceRootsRequest,
    ) -> BackendResult<CheckSourceRootsData> {
        let items = sanitize_source_roots(req.source_roots)
            .into_iter()
            .map(|root| source_root_status(&root))
            .collect::<Vec<_>>();
        let missing = items
            .iter()
            .filter(|item| !item.exists || !item.is_dir)
            .map(|item| item.source_root.clone())
            .collect::<Vec<_>>();

        Ok(CheckSourceRootsData { items, missing })
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

        replace_relocated_source_root_settings(&tx, &old_root, &new_root)?;
        tx.commit()?;

        Ok(RelocateSourceRootData {
            old_root,
            new_root,
            matched,
            updated,
            unchanged,
            missing_at_new_root,
            conflicts,
        })
    }

    pub fn remove_tracks_by_source_roots(
        &self,
        req: RemoveTracksBySourceRootsRequest,
    ) -> BackendResult<RemoveTracksBySourceRootsData> {
        let mut roots = req
            .source_roots
            .into_iter()
            .map(|root| root.trim().to_string())
            .filter(|root| !root.is_empty())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        if roots.is_empty() {
            return Ok(RemoveTracksBySourceRootsData { removed: 0 });
        }

        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        // A track another source folder still covers (a parent of the removed
        // one, or a subfolder of it) stays. The removed roots are left out
        // explicitly: the setting may not have been rewritten yet.
        let remaining_roots = configured_source_roots(&tx)?
            .into_iter()
            .filter(|root| {
                !roots
                    .iter()
                    .any(|removed| source_root_keys_equal(root, removed))
            })
            .collect::<Vec<_>>();
        let mut removed = 0usize;
        for root in roots {
            let escaped = root
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let slash_like = format!("{escaped}/%");
            let backslash_like = format!("{escaped}\\\\%");
            let mut stmt = tx.prepare(
                r#"
                SELECT id, file_path FROM tracks
                WHERE file_path = ?1
                   OR file_path LIKE ?2 ESCAPE '\'
                   OR file_path LIKE ?3 ESCAPE '\'
                "#,
            )?;
            let rows = stmt
                .query_map(params![root, slash_like, backslash_like], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (id, file_path) in rows {
                if remaining_roots
                    .iter()
                    .any(|remaining| browse_path_matches_root(&file_path, remaining))
                {
                    continue;
                }
                removed += tx.execute("DELETE FROM tracks WHERE id = ?1", params![id])?;
            }
        }
        tx.commit()?;
        Ok(RemoveTracksBySourceRootsData { removed })
    }
}

/// Per source root: how many of `tracks` live under it and how many of those
/// have their core analysis. Callers pass the full, unfiltered track set so a
/// folder's status never depends on a search query.
pub(super) fn source_root_analysis(
    source_roots: &[String],
    tracks: &[Track],
) -> Vec<SourceRootAnalysisStatus> {
    source_roots
        .iter()
        .map(|root| {
            let mut total = 0usize;
            let mut analyzed = 0usize;
            for track in tracks {
                if browse_path_matches_root(&track.file_path, root) {
                    total += 1;
                    if track_has_core_analysis_for_source_status(track) {
                        analyzed += 1;
                    }
                }
            }
            SourceRootAnalysisStatus {
                source_root: root.clone(),
                total,
                analyzed,
                fully_analyzed: total > 0 && analyzed == total,
            }
        })
        .collect()
}

pub(super) fn source_root_keys_equal(left: &str, right: &str) -> bool {
    normalize_source_root_for_matching(left) == normalize_source_root_for_matching(right)
}

pub(super) fn write_app_setting_tx(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    value: String,
) -> BackendResult<()> {
    tx.execute(
        r#"
        INSERT INTO app_settings (key, value, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
        params![key, value, now()],
    )?;
    Ok(())
}

pub(super) fn replace_relocated_source_root_settings(
    tx: &rusqlite::Transaction<'_>,
    old_root: &str,
    new_root: &str,
) -> BackendResult<()> {
    if let Some(raw_roots) = tx
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_SOURCE_ROOTS],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        && let Ok(roots) = serde_json::from_str::<Vec<String>>(&raw_roots)
    {
        let mut changed = false;
        let mut next_roots = Vec::<String>::new();
        for root in roots {
            let replacement = if source_root_keys_equal(&root, old_root) {
                changed = true;
                new_root.to_string()
            } else {
                root
            };
            if next_roots
                .iter()
                .any(|existing| source_root_keys_equal(existing, &replacement))
            {
                changed = true;
            } else {
                next_roots.push(replacement);
            }
        }
        if changed {
            let encoded = serde_json::to_string(&next_roots).map_err(|err| {
                BackendError::Internal(format!("failed to encode source roots setting: {err}"))
            })?;
            write_app_setting_tx(tx, SETTING_UI_SOURCE_ROOTS, encoded)?;
        }
    }

    if let Some(raw_enabled) = tx
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_SOURCE_ROOT_ENABLED],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        && let Ok(mut enabled) =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw_enabled)
    {
        let old_keys = enabled
            .keys()
            .filter(|key| source_root_keys_equal(key, old_root))
            .cloned()
            .collect::<Vec<_>>();
        if !old_keys.is_empty() {
            let mut moved_value = None;
            for key in old_keys {
                if moved_value.is_none() {
                    moved_value = enabled.remove(&key);
                } else {
                    enabled.remove(&key);
                }
            }
            if let Some(value) = moved_value {
                let has_new_key = enabled
                    .keys()
                    .any(|key| source_root_keys_equal(key, new_root));
                if !has_new_key {
                    enabled.insert(new_root.to_string(), value);
                }
            }
            let encoded = serde_json::to_string(&enabled).map_err(|err| {
                BackendError::Internal(format!(
                    "failed to encode source root enabled setting: {err}"
                ))
            })?;
            write_app_setting_tx(tx, SETTING_UI_SOURCE_ROOT_ENABLED, encoded)?;
        }
    }

    Ok(())
}
