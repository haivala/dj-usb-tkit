//! Which track an analysis bundle on the USB belongs to. Players find a
//! track's bundle from its path (the hash folder of
//! `canonical_analysis_bundle_paths`) and tell the `ANLZ000N` files in that
//! folder apart by the `PPTH` chunk, the track path stored in the file. When
//! two tracks share one bundle, or a bundle has no `PPTH`, players can show
//! one track's waveform and beat grid for another. Several track rows naming
//! one audio file (a stick exported from more than one rekordbox library has
//! such rows) are one track here: the file and its bundle are theirs alike.
//! The diagnostics, the BPM check, the repairs and export all decide
//! ownership here.

use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The `PPTH` track path of one ANLZ file. Only the start of the file is
/// read; `PPTH` is the first chunk.
fn file_ppth(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut head = Vec::with_capacity(4096);
    std::fs::File::open(path)
        .ok()?
        .take(4096)
        .read_to_end(&mut head)
        .ok()?;
    crate::service::anlz::ppth_path_from_anlz(&head)
}

/// The `PPTH` track path of a bundle: its `.DAT`'s, else its `.EXT`'s.
pub(crate) fn bundle_ppth(dat: &Path) -> Option<String> {
    file_ppth(dat).or_else(|| file_ppth(&dat.with_extension("EXT")))
}

/// Whether every file of the bundle at `dat` (`.DAT`, `.EXT`, `.2EX`, as
/// far as they exist) names `track_path` in its `PPTH`.
fn bundle_fully_named(dat: &Path, track_path: &str) -> bool {
    let key = audio_path_key(track_path);
    [
        dat.to_path_buf(),
        dat.with_extension("EXT"),
        dat.with_extension("2EX"),
    ]
    .iter()
    .filter(|file| file.is_file())
    .all(|file| file_ppth(file).is_some_and(|ppth| audio_path_key(&ppth) == key))
}

/// A bundle more than one track uses, or one whose `PPTH` names another track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedBundle {
    /// Absolute `.DAT` path.
    pub(crate) dat: PathBuf,
    /// The tracks pointing at it, by PDB id.
    pub(crate) track_ids: Vec<u32>,
    /// The rows of the audio file that keeps it (more than one when several
    /// rows name that file); the others get their own. Empty: no track can
    /// be told to own it, so every one gets its own.
    pub(crate) owners: Vec<u32>,
}

/// A bundle only one track uses, but without a `PPTH` naming that track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnmarkedBundle {
    pub(crate) dat: PathBuf,
    pub(crate) track_id: u32,
    pub(crate) track_path: String,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct AnalysisOwnership {
    pub(crate) shared: Vec<SharedBundle>,
    pub(crate) unmarked: Vec<UnmarkedBundle>,
}

impl AnalysisOwnership {
    /// The tracks in a shared bundle other than its owner: each gets a
    /// bundle of its own, so the shared one's grid is not theirs to check.
    pub(crate) fn non_owner_track_ids(&self) -> HashSet<u32> {
        self.shared
            .iter()
            .flat_map(|b| {
                b.track_ids
                    .iter()
                    .copied()
                    .filter(|id| !b.owners.contains(id))
            })
            .collect()
    }
}

/// Group the PDB tracks by the bundle their row points at and decide whose
/// each one is. With `read_ppth` false only the databases are used: a bundle
/// is shared when track rows naming different audio files point at it, and
/// nothing is unmarked.
///
/// The owner of a shared bundle is, in order: the file its `PPTH` names;
/// else the one sharing file `app_master_db_id` exported, since that export
/// wrote the bundle's current contents; else none.
pub(crate) fn scan_analysis_ownership(
    usb_root: &Path,
    parsed: &crate::pdb_reader::ParsedPdb,
    app_master_db_id: Option<u32>,
    read_ppth: bool,
) -> AnalysisOwnership {
    // Database-only, the paths are just keys: resolve them without a
    // filesystem lookup per track, which on a cold, large stick is what the
    // whole diagnosis waits on.
    let resolve = if read_ppth {
        crate::service::usb::root::resolve_usb_side_path
    } else {
        crate::service::usb::root::resolve_usb_side_path_lexical
    };
    let mut by_bundle = BTreeMap::<String, (PathBuf, Vec<&crate::pdb_reader::PdbTrackRow>)>::new();
    for track in &parsed.tracks {
        let Some(dat) = resolve(usb_root, &track.anlz_path) else {
            continue;
        };
        by_bundle
            .entry(dat.to_string_lossy().to_ascii_lowercase())
            .or_insert_with(|| (dat, Vec::new()))
            .1
            .push(track);
    }
    let track_keys = parsed
        .tracks
        .iter()
        .map(|t| audio_path_key(&t.track_file_path))
        .collect::<HashSet<_>>();

    let mut out = AnalysisOwnership::default();
    for (_, (dat, tracks)) in by_bundle {
        let ppth = if read_ppth && dat.is_file() {
            bundle_ppth(&dat)
        } else {
            None
        };
        let ppth_key = ppth.as_deref().map(audio_path_key);
        let named = |t: &&crate::pdb_reader::PdbTrackRow| {
            ppth_key.as_deref() == Some(audio_path_key(&t.track_file_path).as_str())
        };
        // The rows by the audio file they name: rows of one file are one track.
        let mut files = BTreeMap::<String, Vec<&crate::pdb_reader::PdbTrackRow>>::new();
        for track in &tracks {
            files
                .entry(audio_path_key(&track.track_file_path))
                .or_default()
                .push(*track);
        }
        if files.len() == 1 {
            let track = tracks[0];
            if !read_ppth || !dat.is_file() || bundle_fully_named(&dat, &track.track_file_path) {
                continue;
            }
            match ppth_key {
                // The file names another track on the stick: it isn't this one's.
                Some(key) if !named(&track) && track_keys.contains(&key) => {
                    out.shared.push(SharedBundle {
                        dat,
                        track_ids: tracks.iter().map(|t| t.id).collect(),
                        owners: Vec::new(),
                    })
                }
                // No PPTH, or a stale one naming no track: this track's file.
                // One entry: the PPTH is written into the bundle once.
                _ => out.unmarked.push(UnmarkedBundle {
                    dat,
                    track_id: track.id,
                    track_path: track.track_file_path.clone(),
                }),
            }
            continue;
        }
        let owner_file = files
            .values()
            .find(|rows| rows.iter().any(named))
            .or_else(|| {
                let app_files = files
                    .values()
                    .filter(|rows| {
                        app_master_db_id.is_some()
                            && rows.iter().any(|t| t.master_db_id == app_master_db_id)
                    })
                    .collect::<Vec<_>>();
                match app_files.as_slice() {
                    [only] => Some(*only),
                    _ => None,
                }
            });
        out.shared.push(SharedBundle {
            dat,
            track_ids: tracks.iter().map(|t| t.id).collect(),
            owners: owner_file.map_or_else(Vec::new, |rows| rows.iter().map(|t| t.id).collect()),
        });
    }
    out
}
