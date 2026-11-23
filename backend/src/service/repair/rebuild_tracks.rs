//! Track rows a crashed rekordbox write damaged that have no intact twin to
//! merge into (`damaged_tracks`): rebuilt in place from what the stick still
//! knows about them.
//!
//! The crash leaves NUL runs in a row's strings; each run stands for one
//! UTF-16 or two ASCII characters, and the rest of the string is intact. The
//! number fields (file size, duration, ids) survive. So a damaged path is a
//! pattern the real path still fits, and the file size confirms it:
//!
//! - *Path pattern:* every file on the stick no intact row of the same
//!   database points at, whose size is the row's and whose path fits the
//!   damaged one.
//! - *Analysis file:* an undamaged analysis path names a bundle whose `PPTH`
//!   holds the audio path.
//! - *Backups* (PDB rows): the newest backup PDB whose row with the same id
//!   has an undamaged path.
//! - *The other database:* the one intact row with the same file size and
//!   duration.
//!
//! A source only counts when it names such a file. The row is rebuilt when
//! the sources together name exactly one file and no other damaged row of
//! the database names it too. None of them needs the eDB, so a stick with
//! only a PDB is rebuilt the same way.
//!
//! Rows whose path survived and whose file is there, but whose title or
//! another string didn't, get their strings back the same way, path kept.
//!
//! What no source repairs is still damage rekordbox breaks on, so it doesn't
//! stay on the stick whatever the user selects: a row whose path survived
//! (its file is missing) gets its other strings back the same way, as far as
//! the other database has them, the rest emptied; a row whose path is lost is
//! removed, or, when it is in a play history (never removed), keeps its
//! strings with the NULs dropped.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use super::*;

/// The strings a rebuild repairs: the PDB row's field (as
/// `PdbTrackRowMutation::changed_fields` names it) and the eDB `content`
/// column holding the same value.
const REBUILT_STRINGS: [(&str, &str); 9] = [
    ("file_path", "path"),
    ("file_name", "fileName"),
    ("title", "title"),
    ("anlz_path", "analysisDataFilePath"),
    ("isrc", "isrc"),
    ("dj_comment", "djComment"),
    // rekordbox's pairing (see `PdbTrackRow::date_added`).
    ("date_added", "dateCreated"),
    ("analyze_date", "dateAdded"),
    ("release_date", "releaseDate"),
];

/// eDB-only text columns: cleared when damaged, as the export leaves them.
const EDB_ONLY_STRINGS: [&str; 3] = ["titleForSearch", "subtitle", "kuvoDeliveryComment"];

/// A NUL inside the string, not just padding at its end.
pub(super) fn has_nul_run(value: &str) -> bool {
    value.trim_end_matches('\0').contains('\0')
}

/// One damaged row's rebuild: its id and the new value of every string it
/// changes, keyed by PDB field name (`REBUILT_STRINGS`) or, for eDB-only
/// columns, by column name. `None` empties the string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RowRebuild<Id> {
    pub(super) id: Id,
    /// The damaged path the row had; the missing-audio fixes leave it alone.
    pub(super) old_path: String,
    pub(super) values: BTreeMap<&'static str, Option<String>>,
}

/// A track row as the rebuild sees it, from either database.
#[derive(Debug, Clone)]
struct RowFacts<Id> {
    id: Id,
    size: Option<u64>,
    duration: Option<u64>,
    /// By PDB field name (`REBUILT_STRINGS`), plus eDB-only columns.
    strings: HashMap<&'static str, String>,
}

impl<Id> RowFacts<Id> {
    fn path(&self) -> &str {
        self.strings.get("file_path").map_or("", String::as_str)
    }
    /// NUL runs in a string, or no path at all.
    fn is_damaged(&self) -> bool {
        self.path().is_empty() || self.strings.values().any(|v| has_nul_run(v))
    }
}

fn pdb_facts(parsed: &ParsedPdb) -> Vec<RowFacts<u32>> {
    parsed
        .tracks
        .iter()
        .map(|t| {
            let mut strings = HashMap::new();
            strings.insert("file_path", t.track_file_path.clone());
            strings.insert("title", t.title.clone());
            strings.insert("anlz_path", t.anlz_path.clone());
            for (name, value) in [
                ("file_name", &t.file_name),
                ("isrc", &t.isrc),
                ("dj_comment", &t.dj_comment),
                ("date_added", &t.date_added),
                ("analyze_date", &t.analyze_date),
                ("release_date", &t.release_date),
            ] {
                strings.insert(name, value.clone().unwrap_or_default());
            }
            RowFacts {
                id: t.id,
                size: t.file_size_bytes.map(u64::from).filter(|&s| s > 0),
                duration: t.duration_seconds.map(u64::from),
                strings,
            }
        })
        .collect()
}

fn edb_facts(conn: &rusqlite::Connection) -> Vec<RowFacts<i64>> {
    if !table_exists(conn, "content") {
        return Vec::new();
    }
    let columns: Vec<&'static str> = REBUILT_STRINGS
        .iter()
        .map(|(_, column)| *column)
        .chain(EDB_ONLY_STRINGS)
        .collect();
    let sql = format!(
        "SELECT content_id, fileSize, length, {} FROM content",
        columns.join(", ")
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |row| {
        let mut strings = HashMap::new();
        for (i, column) in columns.iter().enumerate() {
            let value: Option<String> = row.get(3 + i)?;
            let key = REBUILT_STRINGS
                .iter()
                .find(|(_, c)| c == column)
                .map_or(*column, |(field, _)| *field);
            strings.insert(key, value.unwrap_or_default());
        }
        let size: Option<i64> = row.get(1)?;
        let length: Option<i64> = row.get(2)?;
        Ok(RowFacts {
            id: row.get(0)?,
            size: size.filter(|&s| s > 0).map(|s| s as u64),
            duration: length.filter(|&l| l >= 0).map(|l| l as u64),
            strings,
        })
    });
    rows.map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// Whether `candidate` fits `damaged`: each run of NULs stands for up to as
/// many characters, everything else matches literally from the start. A
/// damaged path that no longer ends in its extension was cut short and fits
/// any longer path.
pub(super) fn damaged_path_fits(damaged: &str, candidate: &str) -> bool {
    let cut_short = !crate::scanner::is_library_audio_file(Path::new(damaged));
    nul_pattern_fits(damaged.trim_end_matches('\0'), candidate, cut_short)
}

/// Whether `candidate` is a name the damaged `damaged` can be: each NUL run
/// stands for up to as many characters, the rest matches literally and in
/// full. A run at the end counts too (the name's length survives).
pub(super) fn damaged_name_fits(damaged: &str, candidate: &str) -> bool {
    nul_pattern_fits(damaged, candidate, false)
}

/// `pattern` with each NUL run matching 0 up to its length in characters,
/// everything else literally, from the start; to the end of `candidate`
/// unless `prefix` allows stopping early. An all-NUL pattern fits nothing.
fn nul_pattern_fits(pattern: &str, candidate: &str, prefix: bool) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    // Nothing left to match on.
    if pattern.iter().all(|&c| c == '\0') {
        return false;
    }
    let text: Vec<char> = candidate.chars().collect();
    // Positions in `text` reachable after matching the pattern so far.
    let mut reachable = vec![false; text.len() + 1];
    reachable[0] = true;
    let mut i = 0;
    while i < pattern.len() {
        let mut next = vec![false; text.len() + 1];
        if pattern[i] == '\0' {
            let run = pattern[i..].iter().take_while(|&&c| c == '\0').count();
            for (at, _) in reachable.iter().enumerate().filter(|(_, r)| **r) {
                for skip in 0..=run {
                    if let Some(slot) = next.get_mut(at + skip) {
                        *slot = true;
                    }
                }
            }
            i += run;
        } else {
            for (at, _) in reachable.iter().enumerate().filter(|(_, r)| **r) {
                if text.get(at) == Some(&pattern[i]) {
                    next[at + 1] = true;
                }
            }
            i += 1;
        }
        reachable = next;
    }
    reachable[text.len()] || (prefix && reachable.iter().any(|&r| r))
}

/// The file each damaged row names, by the rule in the module docs. `rows`
/// are the damaged rows that need a path, each with the paths other sources
/// name for it; `files` are the stick's files no intact row of the database
/// points at, with their sizes.
fn pick_paths<Id: Copy + Eq + std::hash::Hash>(
    rows: &[(&RowFacts<Id>, Vec<String>)],
    files: &HashMap<String, u64>,
) -> HashMap<Id, String> {
    let mut picked: HashMap<Id, String> = HashMap::new();
    for (row, sourced) in rows {
        let Some(size) = row.size else { continue };
        let fits_size = |path: &String| files.get(path) == Some(&size);
        let mut named: HashSet<&String> = files
            .iter()
            .filter(|(path, file_size)| **file_size == size && damaged_path_fits(row.path(), path))
            .map(|(path, _)| path)
            .collect();
        named.extend(sourced.iter().filter(|p| fits_size(p)));
        if named.len() == 1 {
            picked.insert(row.id, (*named.iter().next().unwrap()).clone());
        }
    }
    let mut claims: HashMap<&String, usize> = HashMap::new();
    for path in picked.values() {
        *claims.entry(path).or_default() += 1;
    }
    let shared: HashSet<String> = claims
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(p, _)| p.clone())
        .collect();
    picked.retain(|_, path| !shared.contains(path));
    picked
}

/// What the stick and the other database know about a row's file, for its
/// damaged strings.
struct StringSources<'a> {
    usb_root: &'a Path,
    /// Intact rows of the other database, by `audio_path_key`.
    other_by_path: HashMap<String, &'a HashMap<&'static str, String>>,
    /// Analysis bundles by the `audio_path_key` of their `PPTH`; read on
    /// first use.
    bundles: std::cell::OnceCell<HashMap<String, String>>,
}

impl StringSources<'_> {
    fn bundle_for(&self, path: &str) -> Option<String> {
        self.bundles
            .get_or_init(|| analysis_bundles_by_track(self.usb_root))
            .get(&audio_path_key(path))
            .cloned()
    }

    /// The new value of every string of `row` that changes, its file at `path`.
    /// `twin`: the strings of the row this one stays in place of, the first
    /// source for each damaged string.
    fn values<Id>(
        &self,
        row: &RowFacts<Id>,
        path: &str,
        path_changed: bool,
        twin: Option<&HashMap<&'static str, String>>,
    ) -> BTreeMap<&'static str, Option<String>> {
        let other = self.other_by_path.get(&audio_path_key(path));
        let undamaged = |strings: Option<&&HashMap<&'static str, String>>, field: &str| {
            strings
                .and_then(|strings| strings.get(field))
                .filter(|v| !v.is_empty() && !has_nul_run(v))
                .cloned()
        };
        let other_value =
            |field: &str| undamaged(twin.as_ref(), field).or_else(|| undamaged(other, field));
        let file_name = path.rsplit('/').next().unwrap_or(path).to_string();
        let mut values = BTreeMap::new();
        if path_changed {
            values.insert("file_path", Some(path.to_string()));
            values.insert("file_name", Some(file_name.clone()));
        }
        for (field, value) in &row.strings {
            if values.contains_key(field) || !has_nul_run(value) {
                continue;
            }
            let new = match *field {
                "file_path" => Some(path.to_string()),
                "file_name" => Some(file_name.clone()),
                "title" => other_value(field).or_else(|| {
                    let abs = self.usb_root.join(path.trim_start_matches('/'));
                    crate::scanner::scan_audio_file(&abs)
                        .ok()
                        .map(|track| track.title)
                        .filter(|title| !title.trim().is_empty())
                        .or_else(|| {
                            Path::new(&file_name)
                                .file_stem()
                                .map(|stem| stem.to_string_lossy().into_owned())
                        })
                }),
                "anlz_path" => other_value(field).or_else(|| self.bundle_for(path)),
                _ => other_value(field),
            };
            values.insert(*field, new);
        }
        values
    }
}

/// Every analysis bundle's `.DAT`, USB-relative, by the `audio_path_key` of
/// the track its `PPTH` names.
fn analysis_bundles_by_track(usb_root: &Path) -> HashMap<String, String> {
    let anlz_root = usb_root.join("PIONEER").join("USBANLZ");
    walkdir::WalkDir::new(&anlz_root)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("DAT"))
        })
        .filter_map(|entry| {
            let ppth = crate::service::diagnostics::bundle_ppth(entry.path())?;
            let rel = entry.path().strip_prefix(usb_root).ok()?;
            Some((
                audio_path_key(&ppth),
                format!("/{}", rel.to_string_lossy().replace('\\', "/")),
            ))
        })
        .collect()
}

/// The stick's files under `Contents/` no row of `rows` with an undamaged
/// path points at, with their sizes.
fn unowned_files<Id>(
    usb_root: &Path,
    contents: &[String],
    rows: &[RowFacts<Id>],
) -> HashMap<String, u64> {
    let owned: HashSet<String> = rows
        .iter()
        .filter(|r| !r.path().is_empty() && !has_nul_run(r.path()))
        .map(|r| audio_path_key(r.path()))
        .collect();
    contents
        .iter()
        .filter(|path| !owned.contains(&audio_path_key(path)))
        .filter_map(|path| {
            let size = std::fs::metadata(usb_root.join(path.trim_start_matches('/')))
                .ok()?
                .len();
            Some((path.clone(), size))
        })
        .collect()
}

/// The paths the analysis file and the other database name for `row`.
fn other_sources<Id, Other>(
    usb_root: &Path,
    row: &RowFacts<Id>,
    other_rows: &[RowFacts<Other>],
) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(anlz) = row
        .strings
        .get("anlz_path")
        .filter(|a| !a.is_empty() && !has_nul_run(a))
        && let Some(ppth) =
            crate::service::diagnostics::bundle_ppth(&usb_root.join(anlz.trim_start_matches('/')))
    {
        paths.push(ppth);
    }
    paths.extend(other_db_twin(row, other_rows).map(str::to_string));
    paths
}

/// The path of the one undamaged row of the other database with `row`'s file
/// size and duration. Size and duration only: the PDB's master ids don't
/// number tracks the way the eDB's do.
fn other_db_twin<'a, Id, Other>(
    row: &RowFacts<Id>,
    other_rows: &'a [RowFacts<Other>],
) -> Option<&'a str> {
    let mut twins = other_rows.iter().filter(|o| {
        !o.is_damaged() && o.size.is_some() && o.size == row.size && o.duration == row.duration
    });
    match (twins.next(), twins.next()) {
        (Some(twin), None) => Some(twin.path()),
        _ => None,
    }
}

/// The undamaged path the newest backup PDB has for each of `ids`.
fn backup_paths(usb_root: &Path, ids: &[u32]) -> HashMap<u32, String> {
    let mut found = HashMap::new();
    for backup in crate::service::usb_backups::backup_pdb_files(usb_root) {
        if found.len() == ids.len() {
            break;
        }
        let Ok(parsed) = parse_pdb(&backup) else {
            continue;
        };
        for track in &parsed.tracks {
            if ids.contains(&track.id)
                && !found.contains_key(&track.id)
                && !track.track_file_path.is_empty()
                && !has_nul_run(&track.track_file_path)
            {
                found.insert(track.id, track.track_file_path.clone());
            }
        }
    }
    found
}

/// Damaged rows that get rebuilt, per database.
#[derive(Debug, Default, Clone)]
pub(super) struct TrackRebuilds {
    pub(super) pdb: Vec<RowRebuild<u32>>,
    pub(super) edb: Vec<RowRebuild<i64>>,
    /// Rows whose path is lost, in no play history: removed.
    pub(super) pdb_removed: Vec<RowRebuild<u32>>,
    pub(super) edb_removed: Vec<RowRebuild<i64>>,
    /// The damaged paths of rows whose path is lost but which are in a play
    /// history: kept, NULs dropped.
    pub(super) kept_for_history: Vec<String>,
}

impl TrackRebuilds {
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(super) fn len(&self) -> usize {
        self.pdb.len() + self.edb.len() + self.removed()
    }

    pub(super) fn removed(&self) -> usize {
        self.pdb_removed.len() + self.edb_removed.len()
    }

    /// The damaged paths of the removed rows.
    pub(super) fn removed_paths(&self) -> impl Iterator<Item = &str> {
        self.pdb_removed
            .iter()
            .map(|r| r.old_path.as_str())
            .chain(self.edb_removed.iter().map(|r| r.old_path.as_str()))
    }

    /// The files rebuilt rows name again: no longer unindexed.
    pub(super) fn claimed_paths(&self) -> HashSet<String> {
        self.pdb
            .iter()
            .filter_map(|r| r.values.get("file_path").cloned().flatten())
            .chain(
                self.edb
                    .iter()
                    .filter_map(|r| r.values.get("file_path").cloned().flatten()),
            )
            .map(|p| audio_path_key(&p))
            .collect()
    }

    /// The damaged paths rebuilt and removed rows had.
    pub(super) fn old_paths(&self) -> impl Iterator<Item = &str> {
        self.pdb
            .iter()
            .map(|r| r.old_path.as_str())
            .chain(self.edb.iter().map(|r| r.old_path.as_str()))
            .chain(self.removed_paths())
    }
}

/// Finds the damaged rows to rebuild. `twinned` are the rows
/// `detect_nul_damaged_tracks` merges into a twin instead; `missing_keys`
/// the `audio_path_key`s of indexed paths with no file on the stick.
pub(super) fn detect_track_rebuilds(
    usb_root: &Path,
    parsed: &ParsedPdb,
    edb: Option<&rusqlite::Connection>,
    twinned: &NulDamagedTracks,
    missing_keys: &HashSet<String>,
) -> TrackRebuilds {
    let pdb_rows = pdb_facts(parsed);
    let edb_rows = edb.map(edb_facts).unwrap_or_default();
    if !pdb_rows.iter().any(RowFacts::is_damaged) && !edb_rows.iter().any(RowFacts::is_damaged) {
        return TrackRebuilds::default();
    }
    let contents = crate::service::usb_utils::collect_contents_files(usb_root);
    let pdb_backups = {
        let ids: Vec<u32> = pdb_rows
            .iter()
            .filter(|r| has_nul_run(r.path()) && !is_paired(&twinned.pdb_twins, &r.id))
            .map(|r| r.id)
            .collect();
        if ids.is_empty() {
            HashMap::new()
        } else {
            backup_paths(usb_root, &ids)
        }
    };

    // First the file each damaged row names, in both databases; then its
    // strings, from the other database's row for the same file wherever
    // that row's string is undamaged, even when its path was rebuilt too.
    let mut pdb_targets = targets_for(
        usb_root,
        &contents,
        &pdb_rows,
        &edb_rows,
        |id| is_paired(&twinned.pdb_twins, id),
        |id| pdb_backups.get(id).cloned(),
        missing_keys,
    );
    let mut edb_targets = targets_for(
        usb_root,
        &contents,
        &edb_rows,
        &pdb_rows,
        |id| is_paired(&twinned.edb_twins, id),
        |_| None,
        missing_keys,
    );
    pdb_targets.extend(kept_twin_targets(&pdb_rows, &twinned.pdb_twins));
    edb_targets.extend(kept_twin_targets(&edb_rows, &twinned.edb_twins));
    let pdb_history: HashSet<u32> = parsed
        .history_entries
        .iter()
        .filter_map(|e| e.track_id)
        .collect();
    let edb_history = edb.map(edb_history_ids).unwrap_or_default();
    let pdb_left = leftovers(
        &pdb_rows,
        &pdb_targets,
        |id| is_paired(&twinned.pdb_twins, id),
        &pdb_history,
        true,
    );
    let edb_left = leftovers(
        &edb_rows,
        &edb_targets,
        |id| is_paired(&twinned.edb_twins, id),
        &edb_history,
        false,
    );
    pdb_targets.extend(pdb_left.cleaned);
    edb_targets.extend(edb_left.cleaned);
    let pdb_sources = StringSources {
        usb_root,
        other_by_path: rows_by_path(&edb_rows, &edb_targets),
        bundles: std::cell::OnceCell::new(),
    };
    let edb_sources = StringSources {
        usb_root,
        other_by_path: rows_by_path(&pdb_rows, &pdb_targets),
        bundles: std::cell::OnceCell::new(),
    };
    let kept_for_history = pdb_left
        .stripped
        .iter()
        .map(|r| r.old_path.clone())
        .chain(edb_left.stripped.iter().map(|r| r.old_path.clone()))
        .collect();
    let mut pdb = rebuilds_of(&pdb_targets, &pdb_sources);
    pdb.extend(pdb_left.stripped);
    let mut edb_rebuilds = rebuilds_of(&edb_targets, &edb_sources);
    edb_rebuilds.extend(edb_left.stripped);
    TrackRebuilds {
        pdb,
        edb: edb_rebuilds,
        pdb_removed: pdb_left.removed,
        edb_removed: edb_left.removed,
        kept_for_history,
    }
}

/// The eDB content ids in a play history.
fn edb_history_ids(conn: &rusqlite::Connection) -> HashSet<i64> {
    if !table_exists(conn, "history_content") {
        return HashSet::new();
    }
    conn.prepare("SELECT DISTINCT content_id FROM history_content")
        .and_then(|mut stmt| {
            let ids = stmt.query_map([], |row| row.get::<_, i64>(0))?;
            Ok(ids.flatten().collect())
        })
        .unwrap_or_default()
}

/// The damaged rows no twin and no rebuild takes, by what happens to them
/// (see the module docs).
struct Leftovers<'a, Id> {
    /// Path intact, file missing: strings back as a rebuild gives them.
    cleaned: Vec<Target<'a, Id>>,
    /// Path lost, in a play history: NULs dropped from every string.
    stripped: Vec<RowRebuild<Id>>,
    /// Path lost, in no play history.
    removed: Vec<RowRebuild<Id>>,
}

/// `empty_path_is_damage`: whether a row with no path at all counts (PDB rows,
/// where the crash empties it; an eDB row can lack a path otherwise).
fn leftovers<'a, Id: Copy + Eq + std::hash::Hash>(
    rows: &'a [RowFacts<Id>],
    targets: &[Target<'_, Id>],
    is_twinned: impl Fn(&Id) -> bool,
    in_history: &HashSet<Id>,
    empty_path_is_damage: bool,
) -> Leftovers<'a, Id> {
    let targeted: HashSet<Id> = targets.iter().map(|t| t.row.id).collect();
    let mut out = Leftovers {
        cleaned: Vec::new(),
        stripped: Vec::new(),
        removed: Vec::new(),
    };
    for row in rows {
        let damaged = row.strings.values().any(|v| has_nul_run(v))
            || (empty_path_is_damage && row.path().is_empty());
        if !damaged || is_twinned(&row.id) || targeted.contains(&row.id) {
            continue;
        }
        let path_lost = row.path().is_empty() || has_nul_run(row.path());
        if !path_lost {
            out.cleaned.push(Target {
                row,
                path: row.path().to_string(),
                path_changed: false,
                twin: None,
            });
            continue;
        }
        let rebuild = RowRebuild {
            id: row.id,
            old_path: row.path().to_string(),
            values: stripped_values(row),
        };
        if in_history.contains(&row.id) {
            out.stripped.push(rebuild);
        } else {
            out.removed.push(rebuild);
        }
    }
    out
}

/// Every damaged string of `row` with its NULs dropped; the file name follows
/// the path.
fn stripped_values<Id>(row: &RowFacts<Id>) -> BTreeMap<&'static str, Option<String>> {
    let mut values: BTreeMap<&'static str, Option<String>> = row
        .strings
        .iter()
        .filter(|(_, v)| has_nul_run(v))
        .map(|(field, v)| {
            let stripped = v.replace('\0', "");
            (*field, (!stripped.is_empty()).then_some(stripped))
        })
        .collect();
    if let Some(Some(path)) = values.get("file_path") {
        let file_name = path.rsplit('/').next().unwrap_or(path).to_string();
        values.insert("file_name", Some(file_name));
    }
    values
}

/// A damaged row to rebuild and the file it names.
struct Target<'a, Id> {
    row: &'a RowFacts<Id>,
    path: String,
    path_changed: bool,
    /// The strings of the twin it stays in place of, when it has one.
    twin: Option<&'a HashMap<&'static str, String>>,
}

/// The damaged rows of `rows` that get rebuilt: those whose damaged path
/// names exactly one file (see the module docs), and those whose path
/// survived with its file on the stick.
fn targets_for<'a, Id: Copy + Eq + std::hash::Hash, Other>(
    usb_root: &Path,
    contents: &[String],
    rows: &'a [RowFacts<Id>],
    other_rows: &[RowFacts<Other>],
    is_twinned: impl Fn(&Id) -> bool,
    backup_path: impl Fn(&Id) -> Option<String>,
    missing_keys: &HashSet<String>,
) -> Vec<Target<'a, Id>> {
    let damaged: Vec<&RowFacts<Id>> = rows
        .iter()
        .filter(|r| r.is_damaged() && !is_twinned(&r.id))
        .collect();
    // A path the crash emptied is as lost as one with NUL runs.
    let path_lost = |r: &RowFacts<Id>| {
        r.path().is_empty()
            || (has_nul_run(r.path()) && missing_keys.contains(&audio_path_key(r.path())))
    };
    let file_there = |r: &RowFacts<Id>| usb_root.join(r.path().trim_start_matches('/')).is_file();
    let mut targets = Vec::new();

    let need_path: Vec<(&RowFacts<Id>, Vec<String>)> = damaged
        .iter()
        .filter(|r| path_lost(r))
        .map(|r| {
            let mut sourced = other_sources(usb_root, r, other_rows);
            sourced.extend(backup_path(&r.id));
            (*r, sourced)
        })
        .collect();
    if !need_path.is_empty() {
        let picked = pick_paths(&need_path, &unowned_files(usb_root, contents, rows));
        for (row, _) in &need_path {
            // A path the crash emptied, of a file that isn't on the stick:
            // the other database's row for it names the missing file, so
            // the missing-audio fixes treat both databases' rows alike.
            let missing_in_other = || {
                other_db_twin(row, other_rows)
                    .filter(|p| row.path().is_empty() && missing_keys.contains(&audio_path_key(p)))
                    .map(str::to_string)
            };
            if let Some(path) = picked.get(&row.id).cloned().or_else(missing_in_other) {
                targets.push(Target {
                    row,
                    path,
                    path_changed: true,
                    twin: None,
                });
            }
        }
    }
    for row in damaged
        .iter()
        .filter(|r| !r.path().is_empty() && !has_nul_run(r.path()) && file_there(r))
    {
        targets.push(Target {
            row,
            path: row.path().to_string(),
            path_changed: false,
            twin: None,
        });
    }
    targets
}

/// The damaged rows that stay in place of their twin (`twins`: row that
/// goes → row that stays): they take its file and its undamaged strings.
fn kept_twin_targets<'a, Id: Copy + Eq + std::hash::Hash>(
    rows: &'a [RowFacts<Id>],
    twins: &HashMap<Id, Id>,
) -> Vec<Target<'a, Id>> {
    let by_id: HashMap<Id, &RowFacts<Id>> = rows.iter().map(|r| (r.id, r)).collect();
    twins
        .iter()
        .filter_map(|(gone, kept)| {
            let (gone, kept) = (by_id.get(gone)?, by_id.get(kept)?);
            kept.is_damaged().then(|| Target {
                row: kept,
                path: gone.path().to_string(),
                path_changed: true,
                twin: Some(&gone.strings),
            })
        })
        .collect()
}

/// Every row of `rows` by the `audio_path_key` of the file it names: its own
/// path when undamaged, else the one its rebuild found.
fn rows_by_path<'a, Id: Copy + Eq + std::hash::Hash>(
    rows: &'a [RowFacts<Id>],
    targets: &[Target<'_, Id>],
) -> HashMap<String, &'a HashMap<&'static str, String>> {
    let picked: HashMap<Id, &str> = targets
        .iter()
        .map(|t| (t.row.id, t.path.as_str()))
        .collect();
    rows.iter()
        .filter_map(|r| {
            let path = picked
                .get(&r.id)
                .copied()
                .or_else(|| (!r.path().is_empty() && !has_nul_run(r.path())).then(|| r.path()))?;
            Some((audio_path_key(path), &r.strings))
        })
        .collect()
}

fn rebuilds_of<Id: Copy + Ord>(
    targets: &[Target<'_, Id>],
    sources: &StringSources<'_>,
) -> Vec<RowRebuild<Id>> {
    let mut out: Vec<RowRebuild<Id>> = targets
        .iter()
        .map(|t| RowRebuild {
            id: t.row.id,
            old_path: t.row.path().to_string(),
            values: sources.values(t.row, &t.path, t.path_changed, t.twin),
        })
        .collect();
    out.sort_by_key(|r| r.id);
    out
}

/// Rewrites the PDB track rows of `rebuilds` with their new strings.
pub(super) fn apply_pdb_track_rebuilds(
    bytes: &mut Vec<u8>,
    rebuilds: &[RowRebuild<u32>],
) -> BackendResult<usize> {
    if rebuilds.is_empty() {
        return Ok(0);
    }
    let page_size = pdb_page_size(bytes)?;
    let parsed = crate::pdb_reader::parse_pdb_bytes(bytes)?;
    let by_id: HashMap<u32, &crate::pdb_reader::PdbTrackRow> =
        parsed.tracks.iter().map(|t| (t.id, t)).collect();
    let mutations: Vec<crate::pdb_writer::PdbTrackRowMutation> = rebuilds
        .iter()
        .filter_map(|rebuild| {
            let mut row = track_row_data_from_reader_row(by_id.get(&rebuild.id)?);
            let mut changed = Vec::new();
            for (&field, value) in &rebuild.values {
                let Some(name) = REBUILT_STRINGS
                    .iter()
                    .map(|(f, _)| *f)
                    .find(|f| *f == field)
                else {
                    continue;
                };
                let text = value.clone().unwrap_or_default();
                match name {
                    "file_path" => row.file_path = text,
                    "title" => row.title = text,
                    "anlz_path" => row.anlz_path = text,
                    "file_name" => row.file_name = value.clone(),
                    "isrc" => row.isrc = value.clone(),
                    "dj_comment" => row.dj_comment = value.clone(),
                    "date_added" => row.date_added = value.clone(),
                    "analyze_date" => row.analyze_date = value.clone(),
                    "release_date" => row.release_date = value.clone(),
                    _ => continue,
                }
                changed.push(name);
            }
            (!changed.is_empty()).then_some(crate::pdb_writer::PdbTrackRowMutation {
                row,
                changed_fields: changed,
            })
        })
        .collect();
    crate::pdb_writer::mutate_tracks_in_place(
        bytes,
        &mutations,
        PdbLayoutProfile::DEFAULT,
        page_size,
    )
}

/// Rewrites the eDB `content` rows of `rebuilds` with their new strings.
pub(super) fn apply_edb_track_rebuilds(
    tx: &rusqlite::Transaction<'_>,
    rebuilds: &[RowRebuild<i64>],
) -> BackendResult<usize> {
    let mut updated = 0;
    for rebuild in rebuilds {
        let assignments: Vec<(&str, Option<String>)> = rebuild
            .values
            .iter()
            .map(|(&field, value)| {
                let column = REBUILT_STRINGS
                    .iter()
                    .find(|(f, _)| *f == field)
                    .map_or(field, |(_, column)| *column);
                (column, value.clone())
            })
            .collect();
        if assignments.is_empty() {
            continue;
        }
        let set = assignments
            .iter()
            .enumerate()
            .map(|(i, (column, _))| format!("{column} = ?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(rebuild.id)];
        params.extend(
            assignments
                .into_iter()
                .map(|(_, value)| Box::new(value) as Box<dyn rusqlite::ToSql>),
        );
        updated += tx.execute(
            &format!("UPDATE content SET {set} WHERE content_id = ?1"),
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
        )?;
    }
    Ok(updated)
}

/// Deletes the eDB `content` rows of `removed` and their playlist entries.
/// Returns the rows deleted.
pub(super) fn apply_edb_track_removals(
    tx: &rusqlite::Transaction<'_>,
    removed: &[RowRebuild<i64>],
) -> BackendResult<usize> {
    let has_playlists = table_exists(tx, "playlist_content");
    let mut deleted = 0;
    for row in removed {
        if has_playlists {
            tx.execute(
                "DELETE FROM playlist_content WHERE content_id = ?1",
                [row.id],
            )?;
        }
        deleted += tx.execute("DELETE FROM content WHERE content_id = ?1", [row.id])?;
    }
    if deleted > 0 {
        sync_property_content_count(tx)?;
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titled(id: u32, path: &str, title: &str) -> RowFacts<u32> {
        RowFacts {
            id,
            size: Some(1000),
            duration: Some(300),
            strings: HashMap::from([
                ("file_path", path.to_string()),
                ("title", title.to_string()),
            ]),
        }
    }

    #[test]
    fn rows_no_source_repairs_are_removed_or_cleaned() {
        let rows = vec![
            // Path lost, in no play history: removed.
            titled(1, "/Contents/A/S\0\0g.mp3", "Song"),
            // Path lost, in a play history: kept, NULs dropped.
            titled(2, "/Contents/B/S\0\0g.mp3", "T\0\0le"),
            // Path intact (its file missing), title damaged: cleaned.
            titled(3, "/Contents/C/Song.mp3", "T\0\0le"),
            // Undamaged: left alone.
            titled(4, "/Contents/D/Song.mp3", "Title"),
            // No path at all: damage only where the crash empties paths.
            titled(5, "", "Title"),
        ];
        let history = HashSet::from([2]);

        let left = leftovers(&rows, &[], |_| false, &history, true);
        let removed: Vec<u32> = left.removed.iter().map(|r| r.id).collect();
        assert_eq!(removed, vec![1, 5]);
        assert_eq!(left.stripped.len(), 1);
        let kept = &left.stripped[0];
        assert_eq!(kept.id, 2);
        assert_eq!(
            kept.values["file_path"].as_deref(),
            Some("/Contents/B/Sg.mp3")
        );
        assert_eq!(kept.values["file_name"].as_deref(), Some("Sg.mp3"));
        assert_eq!(kept.values["title"].as_deref(), Some("Tle"));
        let cleaned: Vec<u32> = left.cleaned.iter().map(|t| t.row.id).collect();
        assert_eq!(cleaned, vec![3]);

        let left = leftovers(&rows, &[], |_| false, &history, false);
        let removed: Vec<u32> = left.removed.iter().map(|r| r.id).collect();
        assert_eq!(
            removed,
            vec![1],
            "an eDB row without a path is not crash damage"
        );
    }

    #[test]
    fn a_nul_run_stands_for_up_to_as_many_characters() {
        let fits = damaged_path_fits;
        assert!(fits("/C\0\0tents/A/Song.\0\0p3", "/Contents/A/Song.mp3"));
        // One UTF-16 character, or a character that was dropped.
        assert!(fits("/Contents/A/S\0\0ng.mp3", "/Contents/A/Song.mp3"));
        assert!(fits("/Contents/A/S\0\0ong.mp3", "/Contents/A/Song.mp3"));
        // A run may hide a folder separator.
        assert!(fits("/Contents/A\0\0Song.mp3", "/Contents/A/Song.mp3"));
        // Not more characters than the run is long.
        assert!(!fits("/Contents/A/S\0\0.mp3", "/Contents/A/Songs.mp3"));
        // Everything outside the runs is literal.
        assert!(!fits("/Contents/B/S\0\0ng.mp3", "/Contents/A/Song.mp3"));
        assert!(!fits("/Contents/A/S\0\0ng.mp3", "/Contents/A/Song.mp3.bak"));
    }

    #[test]
    fn a_damaged_name_fits_only_in_full() {
        assert!(damaged_name_fits("k\0\0das", "keidas"));
        // A name is never cut short.
        assert!(!damaged_name_fits("k\0\0das", "keidas_extra"));
        // A run at the end stands for characters too.
        assert!(damaged_name_fits("_house\0", "_houses"));
        assert!(damaged_name_fits("_house\0\0", "_housey"));
        assert!(!damaged_name_fits("_house\0", "_housesy"));
        assert!(!damaged_name_fits("\0\0", "ab"));
    }

    #[test]
    fn a_path_cut_short_fits_a_longer_one() {
        assert!(damaged_path_fits(
            "/Contents/A/Long Na\0\0",
            "/Contents/A/Long Name.mp3"
        ));
        assert!(!damaged_path_fits(
            "/Contents/A/Long Na\0\0",
            "/Contents/B/Long Name.mp3"
        ));
    }

    fn row(id: u32, path: &str, size: u64) -> RowFacts<u32> {
        RowFacts {
            id,
            size: Some(size),
            duration: Some(300),
            strings: HashMap::from([("file_path", path.to_string())]),
        }
    }

    #[test]
    fn a_path_is_picked_only_when_one_file_fits() {
        let files = HashMap::from([
            ("/Contents/A/Song One.mp3".to_string(), 1000),
            ("/Contents/A/Song Two.mp3".to_string(), 1000),
            ("/Contents/B/Other.mp3".to_string(), 2000),
            ("/Contents/C/Same.mp3".to_string(), 3000),
        ]);
        let one = row(1, "/Contents/A/Song O\0\0.mp3", 1000);
        // Both songs fit the pattern and the size.
        let either = row(2, "/Contents/A/Song \0\0\0.mp3", 1000);
        // Fits the pattern, wrong size.
        let wrong_size = row(3, "/Contents/B/Oth\0\0.mp3", 999);
        // Two rows naming the same file: neither gets it.
        let same_a = row(4, "/Contents/C/S\0\0e.mp3", 3000);
        let same_b = row(5, "/Contents/C/Sa\0\0.mp3", 3000);
        let rows = [&one, &either, &wrong_size, &same_a, &same_b];
        let input: Vec<_> = rows.iter().map(|r| (*r, Vec::new())).collect();
        assert_eq!(
            pick_paths(&input, &files),
            HashMap::from([(1, "/Contents/A/Song One.mp3".to_string())])
        );
    }

    #[test]
    fn sources_that_disagree_pick_nothing() {
        let files = HashMap::from([
            ("/Contents/A/Song One.mp3".to_string(), 1000),
            ("/Contents/A/Elsewhere.mp3".to_string(), 1000),
        ]);
        let damaged = row(1, "/Contents/A/Song O\0\0.mp3", 1000);
        let agree = [(&damaged, vec!["/Contents/A/Song One.mp3".to_string()])];
        assert_eq!(pick_paths(&agree, &files).len(), 1);
        let disagree = [(&damaged, vec!["/Contents/A/Elsewhere.mp3".to_string()])];
        assert!(pick_paths(&disagree, &files).is_empty());
        // A source alone finds a file the pattern no longer fits.
        let unreadable = row(2, "\0\0\0\0", 1000);
        let sourced = [(&unreadable, vec!["/Contents/A/Elsewhere.mp3".to_string()])];
        assert_eq!(
            pick_paths(&sourced, &files),
            HashMap::from([(2, "/Contents/A/Elsewhere.mp3".to_string())])
        );
    }
}
