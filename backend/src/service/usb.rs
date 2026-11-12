//! USB validation, playlist/history fetching, track inspection.

use crate::i18n::tr;
use crate::utils::{counted, track_list_summary, warning_text};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use chrono::NaiveDate;
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::diagnostics::{OutdatedBeatGrid, detect_outdated_beat_grids};
use super::usb_staging::{UsbDatabases, UsbDatabasesExt};
use crate::edb::{open_edb_from_usb_root, table_exists};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    FetchUsbHistoriesData, FetchUsbHistoriesRequest, FetchUsbPlaylistsData,
    FetchUsbPlaylistsRequest, FetchUsbTracksData, FetchUsbTracksRequest, GetSelectionDurationData,
    InspectUsbTrackData, InspectUsbTrackRequest, RemoveUsbPlaylistData, RemoveUsbPlaylistRequest,
    UsbHistory, UsbHistoryCounts, UsbImportStats, UsbPlaylist, UsbTrack, ValidateUsbRootData,
    ValidateUsbRootRequest, WarningEntry,
};
use crate::pdb_reader::{ParsedPdb, PdbHistoryEntryRow, PdbHistoryPlaylistRow};

use super::analysis::normalize_text;
use super::export_helpers::{
    analysis_bundle_path_variants, edb_playlist_exclusive_tracks, prune_stale_export_owned_files,
    remove_playlist_and_tracks_from_pdb_in, remove_playlist_from_edb,
};
use super::key_notation::KeyNotation;
use super::track_list::{duration_totals, selection_duration};
use super::track_matching::ContentFingerprint;
use super::usb_display_files::{
    DisplayReadCounts, LibraryDisplayFiles, UsbDisplaySource, library_files_off_usb,
};
use super::usb_helpers::{
    PlaylistCandidate, decode_history_playlist_id, decode_history_track_id,
    dedupe_usb_playlists_by_name, history_entry_sort_key, is_named_history_playlist,
    lookup_playlist_tracks, merge_playlist_tracks, normalize_packed_id,
    parse_history_name_numeric_id, parse_history_slot_id, sanitize_history_name, sanitize_text,
};
use super::usb_identity::UsbAudioFormats;
use super::usb_utils::{
    self, artwork_path_to_data_url, canonicalize_or_self, canonicalize_playlist_name,
    has_write_access, load_waveform_preview_from_analysis_path, normalize_usb_root_path,
    parse_history_numeric_id, resolve_usb_root, resolve_usb_side_path_lexical_text,
};
use super::usb_vendor_compat::{USB_CONTENTS_DIR, USB_VENDOR_ROOT_DIR};
use super::{
    BackendService, browse_path_matches_root, build_track_match_fingerprint, export_log,
    find_confident_fingerprint_match, now, untainted_usb_root_paths,
};

const SLOW_USB_STAGE_MS: u128 = 8_000;

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
    pub(super) items: Vec<UsbPlaylist>,
    stats: UsbImportStats,
    warnings: Vec<WarningEntry>,
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

fn build_usb_track_index(
    parsed: &crate::pdb_reader::ParsedPdb,
    usb_root: &Path,
) -> HashMap<u32, UsbTrack> {
    parsed
        .tracks
        .iter()
        .map(|t| {
            let artist = parsed
                .artists
                .get(&t.artist_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Artist".to_string());
            let album = parsed.albums.get(&t.album_id).cloned();
            let key = parsed.keys.get(&t.key_id).cloned();
            let artwork_path = parsed
                .artworks
                .get(&t.artwork_id)
                .and_then(|p| resolve_usb_side_path_lexical_text(usb_root, p));
            let resolved_file_path =
                resolve_usb_side_path_lexical_text(usb_root, &t.track_file_path)
                    .unwrap_or_else(|| t.track_file_path.clone());
            let usb_analysis_path = resolve_usb_side_path_lexical_text(usb_root, &t.anlz_path);
            let format_ext =
                crate::audio_format::format_ext_from_path(&t.track_file_path).or_else(|| {
                    t.file_name
                        .as_deref()
                        .and_then(crate::audio_format::format_ext_from_path)
                });
            (
                t.id,
                UsbTrack {
                    id: t.id.to_string(),
                    local_track_id: None,
                    title: if t.title.is_empty() {
                        "Unknown Title".to_string()
                    } else {
                        t.title.clone()
                    },
                    artist,
                    album,
                    year: t.release_year.map(u32::from),
                    genre: parsed
                        .genres
                        .get(&t.genre_id)
                        .map(|g| g.trim().to_string())
                        .filter(|g| !g.is_empty()),
                    track_number: (t.track_number > 0).then_some(t.track_number),
                    bpm: if t.tempo_x100 > 0 {
                        Some(t.tempo_x100 as f64 / 100.0)
                    } else {
                        None
                    },
                    key,
                    key_display: None,
                    key_color: None,
                    file_path: resolved_file_path,
                    needs_hydration: false,
                    // Judged when the row is hydrated for a page.
                    format_compat: Default::default(),
                    format_ext,
                    usb_media_path: Some(t.track_file_path.clone()),
                    artwork_data_url: None,
                    artwork_path,
                    waveform_peaks_path: usb_analysis_path.clone(),
                    usb_analysis_path,
                    usb_analysis_path_raw: Some(t.anlz_path.clone()),
                    waveform_preview: None,
                    duration_ms: t.duration_seconds.map(|s| u64::from(s) * 1000),
                    file_size_bytes: t.file_size_bytes.map(i64::from),
                },
            )
        })
        .collect()
}

fn select_history_rows(
    playlists: &[PdbHistoryPlaylistRow],
    entries: &[PdbHistoryEntryRow],
) -> (Vec<PdbHistoryPlaylistRow>, Vec<PdbHistoryEntryRow>) {
    let t17_playlists = playlists
        .iter()
        .filter(|row| row.source_table == 17)
        .cloned()
        .collect::<Vec<_>>();
    let t18_entries = entries
        .iter()
        .filter(|row| row.source_table == 18)
        .cloned()
        .collect::<Vec<_>>();
    let t11_playlists = playlists
        .iter()
        .filter(|row| row.source_table == 11)
        .cloned()
        .collect::<Vec<_>>();
    let t12_entries = entries
        .iter()
        .filter(|row| row.source_table == 12)
        .cloned()
        .collect::<Vec<_>>();

    // Runtime history import policy: prefer t11/t12 whenever present.
    // t17/t18 on initialized/exported sticks can contain seed/template rows
    // that should not be surfaced as user history playlists.
    if !t11_playlists.is_empty() || !t12_entries.is_empty() {
        (t11_playlists, t12_entries)
    } else {
        (t17_playlists, t18_entries)
    }
}

/// Drop blank/template history playlist rows. Fresh or never-played exports
/// ship a fixed block of these alongside t17/t18 (empty name, no track
/// link) — real rekordbox-recorded sessions are always named "HISTORY NNN".
fn filter_named_history_playlists(
    playlists: Vec<PdbHistoryPlaylistRow>,
) -> Vec<PdbHistoryPlaylistRow> {
    playlists
        .into_iter()
        .filter(|row| is_named_history_playlist(&row.name))
        .collect()
}

fn normalize_date_created(value: &str) -> Option<NaiveDate> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let ten = trimmed.chars().take(10).collect::<String>();
    NaiveDate::parse_from_str(&ten, "%Y-%m-%d").ok()
}

fn apply_history_dates_from_track_date_created(
    histories: &mut [UsbHistory],
    date_created_by_track_id: &HashMap<u32, String>,
) {
    if histories.is_empty() || date_created_by_track_id.is_empty() {
        return;
    }

    let mut carry = None::<NaiveDate>;
    for history in histories.iter_mut() {
        let existing = history
            .created_at
            .as_deref()
            .and_then(normalize_date_created);
        if let Some(date) = existing {
            carry = Some(carry.map_or(date, |prev| prev.max(date)));
            continue;
        }

        let own_latest = history
            .tracks
            .iter()
            .filter_map(|t| t.id.parse::<u32>().ok())
            .filter_map(|id| date_created_by_track_id.get(&id))
            .filter_map(|raw| normalize_date_created(raw))
            .max();

        let resolved = match (carry, own_latest) {
            (Some(prev), Some(current)) => Some(prev.max(current)),
            (Some(prev), None) => Some(prev),
            (None, Some(current)) => Some(current),
            (None, None) => None,
        };

        if let Some(date) = resolved {
            history.created_at = Some(date.format("%Y-%m-%d").to_string());
            carry = Some(date);
        }
    }
}

/// Groups a parsed PDB's tracks by id so callers can look a track up in
/// O(1) instead of doing `parsed.tracks.iter().filter(|t| t.id == id)` (an
/// O(pdb_size) scan) once per id being resolved -- the latter turns a batch
/// of N ids into an O(N * pdb_size) scan, which is what made hydrating a
/// large USB playlist slow regardless of how the batch was chunked. `Vec`-
/// valued because a PDB can legitimately contain more than one row for the
/// same id; `resolve_usb_track_from_sources` picks the best-scoring one.
fn build_pdb_track_index(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
) -> Option<HashMap<u32, Vec<&crate::pdb_reader::PdbTrackRow>>> {
    let parsed = parsed?;
    let mut index: HashMap<u32, Vec<&crate::pdb_reader::PdbTrackRow>> = HashMap::new();
    for t in &parsed.tracks {
        index.entry(t.id).or_default().push(t);
    }
    Some(index)
}

/// Fills in the expensive payload fields (waveform-preview bytes, artwork data
/// URL) on an already-resolved cheap `UsbTrack`. Called only for the tracks on
/// the returned page.
///
/// `files: None` skips the file reads (waveform, artwork) but still fills the
/// cheap fields -- for a page whose load was superseded. See
/// `UsbDisplaySource` for where the files are read from.
fn hydrate_usb_track_in_place(
    track: &mut UsbTrack,
    notation: KeyNotation,
    formats: &UsbAudioFormats,
    files: Option<&UsbDisplaySource>,
) -> DisplayReadCounts {
    let counts = files.map(|files| files.fill(track)).unwrap_or_default();
    // USB rows only carry `format_ext`, so this flags an extension players
    // don't read, or a format the stick's USB audio formats leave unticked.
    track.format_compat = super::format_compat::compute_usb_format_compat(
        track.format_ext.as_deref(),
        Path::new(&track.file_path),
        formats,
    );

    // Whether a row is still missing display data the `inspect_usb_track`
    // deep-resolve could fill in (waveform bytes, artwork, BPM, key). Computed
    // here, after the cheap hydration above, so the frontend just reads the
    // flag instead of re-deriving it from four field checks. An inspect's own
    // row comes back with it cleared.
    let has_waveform = track
        .waveform_preview
        .as_ref()
        .is_some_and(|preview| !preview.is_empty());
    let has_artwork = track
        .artwork_data_url
        .as_deref()
        .is_some_and(|url| !url.is_empty())
        || track
            .artwork_path
            .as_deref()
            .is_some_and(|path| !path.trim().is_empty());
    let has_bpm = track.bpm.is_some_and(|bpm| bpm > 0.0);
    let has_key = track
        .key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty());
    track.needs_hydration = !(has_waveform && has_artwork && has_bpm && has_key);
    (track.key_display, track.key_color) =
        super::key_notation::key_display_fields(track.key.as_deref(), notation);
    if track.title.trim().is_empty() {
        track.title = tr::track_row_unknown_title().render();
    }
    if track.artist.trim().is_empty() {
        track.artist = tr::track_row_unknown_artist().render();
    }
    counts
}

/// Filter → whole-list aggregates → sort → offset-slice. The page comes back
/// un-hydrated (see `hydrate_usb_page`). Shared by `fetch_usb_playlist_tracks`
/// and `fetch_usb_history_tracks`.
fn paginate_usb_tracks(
    command: &str,
    scope_id: &str,
    mut tracks: Vec<UsbTrack>,
    req: &crate::models::FetchUsbTracksRequest,
    warnings: Vec<WarningEntry>,
) -> BackendResult<crate::models::FetchUsbTracksData> {
    let (list_track_count, list_size_bytes) = super::list_totals(&tracks);
    let query = super::TrackQuery::new(&req.query);
    query.retain(&mut tracks);
    let (total_duration_ms, duration_unknown_count) = duration_totals(&tracks);
    let total = tracks.len();

    let sort = super::TrackSort::new(req.sort_by.as_deref(), req.sort_dir.as_deref());
    sort.apply(&mut tracks);

    let signature = super::build_track_cursor_signature(&[
        "track_cursor_v1",
        command,
        scope_id,
        query.as_str(),
        sort.by,
        sort.dir,
    ]);
    let start = match req.offset {
        Some(offset) => offset,
        None => super::decode_offset_cursor(req.cursor.as_deref(), &signature)?.unwrap_or(0),
    };
    let start = start.min(total);
    let limit = if req.limit == 0 {
        total.saturating_sub(start).max(1)
    } else {
        req.limit.clamp(1, 5000)
    };

    let page: Vec<UsbTrack> = tracks.into_iter().skip(start).take(limit).collect();
    let next_offset = start + page.len();
    let has_more = next_offset < total;
    let next_cursor = has_more.then(|| super::encode_offset_cursor(&signature, next_offset));

    Ok(crate::models::FetchUsbTracksData {
        items: page,
        total,
        next_cursor,
        has_more,
        total_duration_ms,
        duration_unknown_count,
        list_track_count,
        list_size_bytes,
        list_summary_text: track_list_summary(list_track_count, list_size_bytes),
        duration_text: crate::utils::track_list_total_time(
            total_duration_ms,
            duration_unknown_count,
        ),
        warnings,
    })
}

/// What the UI says about a selected folder, and whether it can be set up: a
/// folder that exists and is writable, but isn't a USB library yet.
fn with_usb_selection_text(mut data: ValidateUsbRootData) -> ValidateUsbRootData {
    let warning_text = data
        .warnings
        .iter()
        .map(|warning| warning.message.trim())
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    let warned = !warning_text.is_empty();
    data.can_initialize = data.normalized_root.is_some() && !data.valid && data.has_write_access;
    if data.valid {
        let path = data.normalized_root.as_deref().unwrap_or_default();
        data.status_text = if warned {
            tr::usb_root_selected_with_warnings(path, &warning_text)
        } else {
            tr::usb_root_selected(path)
        }
        .render();
    } else if data.can_initialize {
        data.status_text = tr::usb_root_not_initialized().render();
        data.init_hint_text = if warned {
            tr::usb_root_init_hint_missing_structure_with_warnings(&warning_text)
        } else {
            tr::usb_root_init_hint_missing_structure()
        }
        .render();
    } else {
        let why = if warned {
            warning_text.clone()
        } else {
            tr::usb_root_invalid_unknown().render()
        };
        data.status_text = tr::usb_root_invalid(why).render();
        data.init_hint_text = if warned {
            tr::usb_root_init_hint_not_ready_with_warnings(&warning_text)
        } else {
            tr::usb_root_init_hint_not_ready()
        }
        .render();
    }
    data
}

/// A USB History read with its panel totals and status line, built when answering: the parse
/// cache holds the data only, so a cached read answers in the current language.
fn with_usb_history_texts(data: &FetchUsbHistoriesData) -> FetchUsbHistoriesData {
    let mut data = data.clone();
    data.summary_text =
        tr::usb_history_summary(data.counts.imported_playlists, data.counts.imported_tracks)
            .render();
    data.status_text = tr::usb_history_loaded(data.items.len()).render();
    data.warning_text = warning_text(data.warning_count);
    data
}

/// A USB playlist with its click and remove texts, built when answering (see
/// `with_usb_history_texts`).
fn with_usb_playlist_texts(playlist: &UsbPlaylist) -> UsbPlaylist {
    let mut playlist = playlist.clone();
    playlist.selected_text =
        tr::usb_playlists_selected(&playlist.name, playlist.track_count).render();
    playlist.remove_confirm_text = tr::usb_playlists_remove_confirm(&playlist.name).render();
    playlist
}

/// Hydrates a page from `paginate_usb_tracks`. Reading each row's waveform +
/// artwork off the stick is the slow part (seconds per page on Windows).
/// Once a newer load of this list has started, the frontend discards this
/// page, so stop reading: abandoned loads would otherwise queue up on the
/// USB ahead of the one on screen.
fn hydrate_usb_page(
    command: &str,
    page: &mut [UsbTrack],
    notation: KeyNotation,
    files: &UsbDisplaySource,
    is_superseded: impl Fn() -> bool,
) {
    let started = std::time::Instant::now();
    let mut read_count = 0usize;
    let mut counts = DisplayReadCounts::default();
    let mut superseded = false;
    for track in page.iter_mut() {
        superseded = superseded || is_superseded();
        let track_started = std::time::Instant::now();
        counts.add(hydrate_usb_track_in_place(
            track,
            notation,
            files.formats(),
            (!superseded).then_some(files),
        ));
        if !superseded {
            read_count += 1;
        }
        let track_ms = track_started.elapsed().as_millis();
        if track_ms >= 500 {
            logging::debug(
                "usb-import",
                format!(
                    "{command}: slow USB read for '{}' ({track_ms}ms): analysis {:?}, artwork {:?}",
                    track.title, track.usb_analysis_path, track.artwork_path
                ),
            );
        }
    }
    let elapsed = started.elapsed().as_millis();
    let sources = format!(
        "files: {} from library, {} from local cache, {} from USB",
        counts.library, counts.cache, counts.usb
    );
    if superseded {
        logging::debug(
            "usb-import",
            format!(
                "{command}: superseded by a newer load after reading {read_count} of {} in {elapsed}ms ({sources})",
                counted(page.len(), "track", "tracks")
            ),
        );
    } else {
        logging::debug(
            "usb-import",
            format!(
                "{command}: read {} in {elapsed}ms ({sources})",
                counted(page.len(), "track", "tracks")
            ),
        );
    }
}

/// `usb_track_rows`: the first row of each wanted media path in `tracks`.
fn collect_usb_rows<'a>(
    rows: &mut HashMap<String, UsbTrack>,
    wanted: &HashSet<&str>,
    tracks: impl Iterator<Item = &'a UsbTrack>,
) {
    for track in tracks {
        let Some(path) = track.usb_media_path.as_deref().map(str::trim) else {
            continue;
        };
        if wanted.contains(path) && !rows.contains_key(path) {
            rows.insert(path.to_string(), track.clone());
        }
    }
}

/// The registered `usb_devices` id for the stick mounted at `usb_root`.
fn usb_device_id_for_root(
    conn: &rusqlite::Connection,
    usb_root: &Path,
) -> BackendResult<Option<String>> {
    let root_key = super::normalize_source_root_for_matching(&usb_root.to_string_lossy());
    Ok(conn
        .query_row(
            "SELECT id FROM usb_devices WHERE root_path_key = ?1",
            params![root_key],
            |row| row.get(0),
        )
        .optional()?)
}

fn build_history_track_date_index(
    parsed_tracks: &[crate::pdb_reader::PdbTrackRow],
) -> HashMap<u32, String> {
    parsed_tracks
        .iter()
        .filter_map(|track| {
            let date = track
                .date_added
                .as_deref()
                .map(sanitize_text)
                .filter(|value| normalize_date_created(value).is_some())?;
            Some((track.id, date))
        })
        .collect()
}

/// Recursively remove empty directories bottom-up.
fn cleanup_empty_dirs_recursive(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            cleanup_empty_dirs_recursive(&path);
            // Try to remove if now empty (ignore errors)
            let _ = std::fs::remove_dir(&path);
        }
    }
}

fn push_usb_stage_timing(stage: &str, started: &mut std::time::Instant) {
    push_usb_stage_timing_with_threshold(stage, started, SLOW_USB_STAGE_MS);
}

/// Extra allowance for eDB-heavy stages whose wall-clock cost scales with
/// library size (SQL joins over `content`/`playlist`/etc. that grow with
/// track count), not media speed -- see `push_usb_stage_timing_with_threshold`
/// for why a flat threshold alone misdiagnoses "slow media" on large-but-fast
/// libraries.
const SLOW_USB_STAGE_PER_TRACK_MS: u128 = 1;

fn slow_stage_threshold_ms(item_count: usize) -> u128 {
    SLOW_USB_STAGE_MS + (item_count as u128) * SLOW_USB_STAGE_PER_TRACK_MS
}

/// Same as `push_usb_stage_timing`, but with an explicit threshold instead
/// of the flat `SLOW_USB_STAGE_MS` default. Stages whose cost is genuinely
/// proportional to library size (not media speed) should pass a threshold
/// computed via `slow_stage_threshold_ms` instead of assuming every stage
/// costs the same regardless of how much data it reads.
/// Logs the stage's time to the log file; returns whether it looked slow.
fn push_usb_stage_timing_with_threshold(
    stage: &str,
    started: &mut std::time::Instant,
    threshold_ms: u128,
) -> bool {
    log_stage_timing("usb-import", stage, started, threshold_ms)
}

/// Logs how long `stage` took since `started` under `category`, flags it as
/// slow media at `threshold_ms` or more, and restarts `started` for the next
/// stage. Returns whether it looked slow.
pub(crate) fn log_stage_timing(
    category: &str,
    stage: &str,
    started: &mut std::time::Instant,
    threshold_ms: u128,
) -> bool {
    let elapsed = started.elapsed().as_millis();
    logging::debug(category, format!("stage timing: {stage}: {elapsed}ms"));
    let slow = elapsed >= threshold_ms;
    if slow {
        logging::debug(
            category,
            format!("slow-media suspected: stage '{stage}' took {elapsed}ms"),
        );
    }
    *started = std::time::Instant::now();
    slow
}

/// An artwork file and its medium variant (`.jpg` → `_m.jpg`).
fn artwork_size_variants(small: &str) -> Vec<String> {
    let medium = small.replace(".jpg", "_m.jpg").replace(".png", "_m.png");
    if medium == small {
        vec![medium]
    } else {
        vec![small.to_string(), medium]
    }
}

impl BackendService {
    /// Checks the USB at `req.path`, and makes it the selected USB when it's valid (no USB
    /// selected otherwise).
    pub fn validate_usb_root(
        &self,
        req: ValidateUsbRootRequest,
    ) -> BackendResult<ValidateUsbRootData> {
        let result = self.check_usb_root(req);
        let selected = result
            .as_ref()
            .ok()
            .filter(|data| data.valid)
            .and_then(|data| data.normalized_root.as_ref())
            .map(std::path::PathBuf::from);
        self.set_selected_usb_root(selected);
        result
    }

    fn check_usb_root(&self, req: ValidateUsbRootRequest) -> BackendResult<ValidateUsbRootData> {
        let mut warnings = Vec::<WarningEntry>::new();
        let trimmed = req.path.trim();
        if trimmed.is_empty() {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.path-empty",
                &tr::usb_warning_path_empty(),
            ));
            return Ok(with_usb_selection_text(ValidateUsbRootData {
                valid: false,
                has_write_access: false,
                normalized_root: None,
                has_vendor_root: false,
                has_contents: false,
                has_pdb: false,
                has_edb: false,
                can_initialize: false,
                status_text: String::new(),
                init_hint_text: String::new(),
                warnings,
            }));
        }

        let raw = std::path::PathBuf::from(trimmed);
        let candidate = if raw.exists() {
            raw
        } else if raw.is_relative() {
            std::env::current_dir()?.join(raw)
        } else {
            raw
        };
        if !candidate.exists() {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.path-not-found",
                &tr::usb_warning_path_not_found(candidate.display()),
            ));
            return Ok(with_usb_selection_text(ValidateUsbRootData {
                valid: false,
                has_write_access: false,
                normalized_root: None,
                has_vendor_root: false,
                has_contents: false,
                has_pdb: false,
                has_edb: false,
                can_initialize: false,
                status_text: String::new(),
                init_hint_text: String::new(),
                warnings,
            }));
        }

        let normalized = normalize_usb_root_path(canonicalize_or_self(candidate));
        let mount_conn = self.db.connect()?;
        usb_utils::upsert_usb_device(&mount_conn, &normalized, true, &now())?;
        // Before staging below: a drive carrying another drive's name must
        // not stage into that drive's cache.
        self.check_drive_identity(&mount_conn, &normalized)?;
        drop(mount_conn);
        let has_vendor_root = normalized.join(USB_VENDOR_ROOT_DIR).is_dir();
        let has_contents = normalized.join(USB_CONTENTS_DIR).is_dir();

        if !has_vendor_root {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.missing-vendor-root",
                &tr::usb_warning_missing_vendor_root(),
            ));
        }
        if !has_contents {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.missing-contents",
                &tr::usb_warning_missing_contents(),
            ));
        }

        // Stage the local HDD working copy of the PDB/eDB exactly once, here
        // at connect time, so every read for the rest of this connected
        // session (diagnostics, playlist fetch, track inspect, export, ...)
        // can trust the local copy and skip its own USB stat() -- see
        // `usb_staging::stage_file_with_root`'s verified-this-connect fast
        // path. A database the USB lacks is dropped from staging. Best-effort:
        // a staging hiccup shouldn't fail validation.
        let databases = UsbDatabases::on_connect(&normalized);
        let has_pdb = databases.pdb().is_some();
        let has_edb = databases.edb().is_some();

        let has_write_access = has_write_access(&normalized);
        if !has_write_access {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.read-only",
                &tr::usb_warning_read_only(),
            ));
        }

        Ok(with_usb_selection_text(ValidateUsbRootData {
            valid: has_vendor_root && has_contents,
            has_write_access,
            normalized_root: Some(normalized.to_string_lossy().to_string()),
            has_vendor_root,
            has_contents,
            has_pdb,
            has_edb,
            can_initialize: false,
            status_text: String::new(),
            init_hint_text: String::new(),
            warnings,
        }))
    }

    pub fn list_usb_devices(&self) -> BackendResult<crate::models::ListUsbDevicesData> {
        let conn = self.db.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, root_path, label, mounted, first_seen_at, last_seen_at
             FROM usb_devices
             WHERE deleted_at IS NULL
             ORDER BY last_seen_at DESC",
        )?;
        let items = stmt
            .query_map([], |row| {
                Ok(crate::models::UsbDeviceSummary {
                    id: row.get(0)?,
                    root_path: row.get(1)?,
                    label: row.get(2)?,
                    mounted: row.get::<_, i64>(3)? != 0,
                    first_seen_at: row.get(4)?,
                    last_seen_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(crate::models::ListUsbDevicesData { items })
    }

    pub fn prune_usb_device(
        &self,
        req: crate::models::PruneUsbDeviceRequest,
    ) -> BackendResult<crate::models::PruneUsbDeviceData> {
        let conn = self.db.connect()?;
        let now_ts = now();
        let label: Option<String> = conn
            .query_row(
                "SELECT label FROM usb_devices WHERE id = ?1 AND deleted_at IS NULL",
                params![req.id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let changed = conn.execute(
            "UPDATE usb_devices SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            params![now_ts, req.id],
        )?;
        // A removed drive's name is free again.
        if let Some(label) = label.filter(|label| !label.trim().is_empty()) {
            usb_utils::forget_drive_name(&conn, label.trim())?;
        }
        drop(conn);
        Ok(crate::models::PruneUsbDeviceData {
            pruned: changed > 0,
            devices: self.list_usb_devices()?.items,
        })
    }

    /// Reads the user-assigned name for `req.usb_root` from the on-drive
    /// marker (survives a different computer). A drive without one is
    /// unnamed, even when `usb_devices` has a name cached for its mount path:
    /// drives with the same volume label mount at the same path, so that
    /// name may belong to a different drive. A drive whose marker names
    /// another drive is unnamed too (`check_drive_identity`). See
    /// `usb_identity` for why a name, not a hardware serial, is this app's
    /// notion of drive identity.
    pub fn get_usb_device_name(
        &self,
        req: crate::models::GetUsbDeviceNameRequest,
    ) -> BackendResult<crate::models::GetUsbDeviceNameData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let conn = self.db.connect()?;
        let name_taken_by = self.check_drive_identity(&conn, &usb_root)?;
        if name_taken_by.is_none()
            && let Some(name) = super::usb_identity::read_drive_name(&usb_root)
        {
            return Ok(crate::models::GetUsbDeviceNameData {
                name: Some(name),
                suggested_name: None,
                name_taken_by: None,
            });
        }

        // The OS label is often the other drive's name: suggesting it would
        // have the two share it again.
        let suggested_name = super::usb_identity::suggest_drive_name(&usb_root)
            .map(|suggested| suggested.trim().to_string())
            .filter(|suggested| {
                name_taken_by.as_deref().is_none_or(|taken| {
                    super::usb_identity::slug(taken) != super::usb_identity::slug(suggested)
                })
            });
        Ok(crate::models::GetUsbDeviceNameData {
            name: None,
            suggested_name,
            name_taken_by,
        })
    }

    /// Decides whether the drive at `usb_root` is the drive its name marker
    /// names, or a different one carrying that name: a copy of the drive
    /// (stick or folder), or a drive that took its name through an old bug.
    /// It's a different drive when the drive this computer knows by that
    /// name had a different size, or got an export (logged on it, see
    /// `export_log`) that this drive's export log doesn't have. A different
    /// drive counts as unnamed (`usb_identity::drive_name`) so it gets its
    /// own cache and backups; returns the name it carries.
    ///
    /// Otherwise registers the name, so drives named on another computer
    /// count as taken here too, and labels the mount path with it.
    pub(crate) fn check_drive_identity(
        &self,
        conn: &rusqlite::Connection,
        usb_root: &Path,
    ) -> BackendResult<Option<String>> {
        let marker = super::usb_identity::read_drive_name(usb_root);
        let registered = match &marker {
            Some(name) => usb_utils::find_registered_drive(conn, name)?,
            None => None,
        };
        let total_bytes = fs4::total_space(usb_root).ok();
        let conflict = registered.as_ref().is_some_and(|drive| {
            let size_differs =
                matches!((drive.total_bytes, total_bytes), (Some(known), Some(now)) if known != now);
            // An unreadable log proves nothing either way.
            let export_missing = drive.last_export_at.as_deref().is_some_and(|exported_at| {
                super::export_log::has_export_log_record(usb_root, exported_at) == Some(false)
            });
            size_differs || export_missing
        });

        if super::usb_identity::has_name_conflict(usb_root) != conflict {
            super::usb_identity::set_name_conflict(usb_root, conflict);
            super::usb_staging::forget_cache_key_for_root(usb_root);
            self.invalidate_usb_parse_cache();
        }
        if conflict {
            return Ok(registered.map(|drive| drive.name));
        }
        if let Some(name) = &marker {
            let now_ts = now();
            usb_utils::register_drive_name(conn, name, &now_ts)?;
            if let Some(total_bytes) = total_bytes {
                usb_utils::fill_registered_drive_size(conn, name, total_bytes)?;
            }
            // The path's label follows the drive last seen there, also one
            // named elsewhere (another computer, or a hand-edited marker).
            usb_utils::set_device_label(conn, usb_root, name, &now_ts)?;
        }
        Ok(None)
    }

    /// Assigns `req.name` to `req.usb_root`, rejecting names already taken by
    /// a different drive (names double as cache/backup-archive keys). Also
    /// renames: the drive's cache dir moves to the new name and its old name
    /// is freed. A drive carrying another drive's name may take that name
    /// back (it is that drive after all), which makes it the drive the name
    /// is checked against. Writes the on-drive marker (source of truth), the
    /// local `usb_devices.label` cache and the `usb_drive_names` registry.
    pub fn set_usb_device_name(
        &self,
        req: crate::models::SetUsbDeviceNameRequest,
    ) -> BackendResult<crate::models::SetUsbDeviceNameData> {
        let usb_root = resolve_usb_root(Some(&req.usb_root))?;
        let name =
            super::usb_identity::validate_name(&req.name).map_err(BackendError::Validation)?;
        let same_name =
            |other: &str| super::usb_identity::slug(other) == super::usb_identity::slug(&name);
        let marker = super::usb_identity::read_drive_name(&usb_root);
        let conflicted = super::usb_identity::has_name_conflict(&usb_root);
        // A carried name isn't this drive's: renaming away from it must not
        // take the other drive's cache or free its name.
        let current = marker.as_deref().filter(|_| !conflicted);
        let updated = current != Some(name.as_str());
        let renaming_away = current.filter(|current| !same_name(current));
        let claims_name = conflicted && marker.as_deref().is_some_and(same_name);

        let conn = self.db.connect()?;
        // Keeping its own name (or changing only its case) is not a collision.
        let keeps_own_name = current.is_some() && renaming_away.is_none();
        if !keeps_own_name
            && !claims_name
            && let Some(taken) = usb_utils::find_registered_drive(&conn, &name)?
        {
            return Err(BackendError::Validation(tr::usb_error_name_taken(
                &name,
                &taken.name,
            )));
        }

        if let Some(old_name) = renaming_away {
            super::usb_staging::rename_cache_dir(old_name, &name).map_err(|e| {
                BackendError::Internal(format!("failed to move the drive's cache: {e}"))
            })?;
        }
        if let Err(e) = super::usb_identity::write_drive_name(&usb_root, &name) {
            if let Some(old_name) = renaming_away {
                let _ = super::usb_staging::rename_cache_dir(&name, old_name);
            }
            return Err(BackendError::Internal(format!(
                "failed to write drive name marker: {e}"
            )));
        }
        super::usb_identity::set_name_conflict(&usb_root, false);
        super::usb_staging::forget_cache_key_for_root(&usb_root);
        let now_ts = now();
        if let Some(old_name) = renaming_away {
            usb_utils::forget_drive_name(&conn, old_name)?;
            // Earlier exports to this drive name it as it is now called. By
            // its old name, not its mount path: another stick may have used
            // the same path, and names are unique to one drive.
            conn.execute(
                "UPDATE playlists SET last_exported_usb_name = ?1 WHERE last_exported_usb_name = ?2",
                params![name, old_name],
            )?;
        }
        usb_utils::register_drive_name(&conn, &name, &now_ts)?;
        if !keeps_own_name {
            // This drive is now the one the name is checked against.
            usb_utils::set_registered_drive(
                &conn,
                &name,
                super::export_log::latest_export_log_record_at(&usb_root).as_deref(),
                fs4::total_space(&usb_root).ok(),
            )?;
        }
        usb_utils::set_device_label(&conn, &usb_root, &name, &now_ts)?;
        self.invalidate_usb_parse_cache();

        Ok(crate::models::SetUsbDeviceNameData {
            status_text: if updated {
                tr::usb_drive_renamed(&name)
            } else {
                tr::usb_drive_name_unchanged()
            }
            .render(),
            name,
            updated,
        })
    }

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

    /// The selected USB (`selected_usb_root`), if any.
    pub(crate) fn selected_usb_root(&self) -> Option<std::path::PathBuf> {
        self.selected_usb_root
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn set_selected_usb_root(&self, root: Option<std::path::PathBuf>) {
        *self
            .selected_usb_root
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = root;
    }

    /// No USB selected any more: the window cleared its choice.
    /// Selects the USB at `req.path` (none when empty): checks it, makes it the selected USB
    /// when valid, saves it as the folder the app reopens with, and answers with what the USB
    /// page shows for it.
    pub fn select_usb(
        &self,
        req: crate::models::SelectUsbRequest,
    ) -> BackendResult<crate::models::SelectUsbData> {
        let path = req.path.trim();
        // Another USB's order edit ends here (refused while it's being saved).
        let new_root = (!path.is_empty())
            .then(|| resolve_usb_root(Some(path)).ok())
            .flatten();
        self.switch_usb_order_edit_root(new_root.as_deref())?;
        let usb = if path.is_empty() {
            self.set_selected_usb_root(None);
            ValidateUsbRootData {
                valid: false,
                normalized_root: None,
                has_write_access: false,
                has_vendor_root: false,
                has_contents: false,
                has_pdb: false,
                has_edb: false,
                can_initialize: false,
                status_text: tr::usb_root_cleared().render(),
                init_hint_text: String::new(),
                warnings: Vec::new(),
            }
        } else {
            self.validate_usb_root(ValidateUsbRootRequest {
                path: path.to_string(),
            })?
        };
        let saved_root = usb
            .normalized_root
            .clone()
            .or_else(|| (!path.is_empty()).then(|| path.to_string()));
        self.save_usb_root_setting(saved_root.as_deref())?;

        let valid_root = usb.normalized_root.clone().filter(|_| usb.valid);
        let audio_formats = valid_root.as_ref().and_then(|root| {
            self.get_usb_audio_formats(crate::models::GetUsbAudioFormatsRequest {
                usb_root: root.clone(),
            })
            .map_err(|err| {
                logging::emit_text(Level::Warn, "usb", &tr::usb_audio_formats_read_failed(&err));
            })
            .ok()
        });
        let device_name = valid_root.as_ref().and_then(|root| {
            self.get_usb_device_name(crate::models::GetUsbDeviceNameRequest {
                usb_root: root.clone(),
            })
            .map_err(|err| {
                logging::emit_text(Level::Warn, "usb", &tr::usb_name_check_failed(&err));
            })
            .ok()
        });
        let devices = self.list_usb_devices()?.items;
        Ok(crate::models::SelectUsbData {
            usb,
            audio_formats,
            device_name,
            devices,
        })
    }

    /// The names of the playlists on `usb_root`, for the export status. From the parse
    /// caches while the staged databases are unchanged, so a playlist change doesn't read
    /// the USB again; else a names-only read of the staged PDB and eDB.
    pub(crate) fn usb_playlist_names(&self, usb_root: &Path) -> Arc<HashSet<String>> {
        let parsed = UsbDatabases::present(usb_root)
            .ok()
            .flatten()
            .pdb()
            .and_then(|path| self.parsed_staged_pdb(path).ok());
        let edb = self.staged_edb(usb_root, None, &mut Vec::new());
        Arc::new(super::export::usb_playlist_names_for_export_compare(
            parsed.as_deref().map(|staged| &*staged.parsed),
            edb.iter()
                .flat_map(|library| library.playlists.keys().cloned()),
        ))
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
                StagedFileSig::of(&super::export_log::export_log_path(usb_root)),
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
        let facts = super::usb_track_facts::UsbTrackFacts::read(
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

    fn usb_playlists_cache_hit(&self, key: &UsbParseCacheKey) -> Option<Arc<CachedUsbPlaylists>> {
        let guard = self.usb_playlists_cache.lock().ok()?;
        guard
            .as_ref()
            .filter(|(cached_key, _)| cached_key == key)
            .map(|(_, value)| Arc::clone(value))
    }

    fn usb_histories_cache_hit(
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
    pub(super) fn usb_histories_resolved(
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
    pub(super) fn usb_playlists_resolved(
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

    /// Build the `FetchUsbPlaylistsData` response from a resolved parse: in
    /// the pending order while an order edit is on.
    fn finish_usb_playlists(
        &self,
        usb_root: &Path,
        resolved: &CachedUsbPlaylists,
    ) -> BackendResult<FetchUsbPlaylistsData> {
        let playlist_count = resolved.items.len();
        let track_total: usize = resolved.items.iter().map(|p| p.track_count).sum();
        let summary_text = tr::usb_playlists_summary(playlist_count, track_total).render();
        let warning_count = logging::count_warnings(&resolved.warnings);
        let order_edit = self.usb_order_edit_state(usb_root);
        let mut items: Vec<UsbPlaylist> =
            resolved.items.iter().map(with_usb_playlist_texts).collect();
        if let Some(order) = self.pending_usb_playlist_order(usb_root) {
            super::usb_order_edit::apply_pending_order(&mut items, &order, |p| p.id.clone());
        }
        for item in &mut items {
            item.order_edited = order_edit.edited_playlist_ids.contains(&item.id);
        }
        Ok(FetchUsbPlaylistsData {
            items,
            order_edit,
            order_edit_blocked_text: Self::usb_order_edit_blocked_text(usb_root),
            stats: resolved.stats.clone(),
            status_text: tr::usb_playlists_loaded(playlist_count).render(),
            done_text: tr::usb_playlists_done(&summary_text).render(),
            summary_text,
            warning_text: warning_text(warning_count),
            warnings: resolved.warnings.clone(),
        })
    }

    pub fn fetch_usb_playlists(
        &self,
        req: FetchUsbPlaylistsRequest,
    ) -> BackendResult<FetchUsbPlaylistsData> {
        self.fetch_usb_playlists_with_progress(req, |_, _, _| {})
    }

    pub fn fetch_usb_playlists_with_progress<F>(
        &self,
        req: FetchUsbPlaylistsRequest,
        mut on_progress: F,
    ) -> BackendResult<FetchUsbPlaylistsData>
    where
        F: FnMut(usize, usize, &str),
    {
        let mut warnings = Vec::<WarningEntry>::new();
        let mut stage_started = std::time::Instant::now();
        on_progress(2, 100, &tr::usb_progress_resolving_root().render());
        on_progress(10, 100, &tr::usb_progress_parsing_pdb().render());
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        push_usb_stage_timing("resolve usb root", &mut stage_started);

        let cache_key = self.usb_parse_cache_key(&usb_root);
        if let Some(resolved) = self.usb_playlists_cache_hit(&cache_key) {
            return self.finish_usb_playlists(&usb_root, &resolved);
        }

        let databases = UsbDatabases::present(&usb_root)?;
        let staged_parse = if let Some(pdb_path) = databases.pdb() {
            let staged = self.parsed_staged_pdb(pdb_path)?;
            for message in &staged.parsed.warnings {
                logging::debug("usb-import", message);
            }
            Some(staged)
        } else {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.pdb-not-found",
                &tr::usb_warning_pdb_not_found_edb_only(usb_root.display()),
            ));
            None
        };
        let parsed: Option<&ParsedPdb> = staged_parse.as_deref().map(|staged| &*staged.parsed);
        push_usb_stage_timing("parse PDB", &mut stage_started);

        on_progress(30, 100, &tr::usb_progress_reading_edb().render());
        let (edb_track_by_id, edb_playlists) = self
            .staged_edb(&usb_root, None, &mut warnings)
            .map(|library| {
                let library = library.resolved(&usb_root);
                let playlists = (!library.playlists.is_empty()).then_some(library.playlists);
                (library.tracks, playlists)
            })
            .unwrap_or_default();
        let edb_playlist_tracks = edb_playlists.as_ref().map(|m| {
            m.iter()
                .map(|(name, playlist)| (name.clone(), playlist.tracks.clone()))
                .collect::<HashMap<_, _>>()
        });
        let eedb_playlist_tracks_canonical = edb_playlist_tracks.as_ref().map(|m| {
            m.iter()
                .map(|(name, tracks)| (canonicalize_playlist_name(name), tracks.clone()))
                .collect::<HashMap<_, _>>()
        });
        push_usb_stage_timing_with_threshold(
            "read eDB",
            &mut stage_started,
            slow_stage_threshold_ms(edb_track_by_id.len()),
        );

        let mut track_by_id = HashMap::<u32, UsbTrack>::new();
        let mut entries_by_playlist =
            HashMap::<u32, Vec<crate::pdb_reader::PdbPlaylistEntryRow>>::new();
        let mut playlist_candidates = Vec::<PlaylistCandidate>::new();

        if let Some(parsed) = parsed {
            track_by_id = build_usb_track_index(parsed, &usb_root);

            entries_by_playlist =
                parsed
                    .playlist_entries
                    .iter()
                    .fold(HashMap::<u32, Vec<_>>::new(), |mut acc, e| {
                        acc.entry(e.playlist_id).or_default().push(e.clone());
                        acc
                    });

            let mut leaves = parsed
                .playlist_tree
                .iter()
                .filter(|n| !n.row_is_folder)
                .cloned()
                .collect::<Vec<_>>();
            leaves.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
            for node in leaves {
                let folder_name = parsed
                    .playlist_tree
                    .iter()
                    .find(|p| p.id == node.parent_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let display_name = if folder_name.is_empty() {
                    node.name.clone()
                } else {
                    format!("{folder_name} / {}", node.name)
                };
                playlist_candidates.push(PlaylistCandidate {
                    pdb_id: Some(node.id),
                    short_name: node.name,
                    display_name,
                    sort_order: node.sort_order,
                });
            }
        }

        on_progress(
            50,
            100,
            &tr::usb_progress_resolving_edb_candidates().render(),
        );
        let mut seen = playlist_candidates
            .iter()
            .map(|c| canonicalize_playlist_name(&c.display_name))
            .collect::<HashSet<_>>();
        if let Some(map) = &edb_playlists {
            for (name, playlist) in map {
                let key = canonicalize_playlist_name(name);
                if seen.insert(key) {
                    playlist_candidates.push(PlaylistCandidate {
                        pdb_id: None,
                        short_name: name.clone(),
                        display_name: name.clone(),
                        // A reorder gives both databases one numbering.
                        sort_order: u32::try_from(playlist.sort_order).unwrap_or(0),
                    });
                }
            }
        }
        playlist_candidates.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then(a.display_name.cmp(&b.display_name))
        });

        on_progress(70, 100, &tr::usb_progress_resolving_playlists().render());
        let mut items = Vec::new();
        let mut referenced_track_ids = HashSet::<u32>::new();
        let mut playlist_entries_total = 0usize;
        let mut source_counts = HashMap::<&'static str, usize>::new();
        let mut empty_source_playlists = Vec::<String>::new();
        for candidate in playlist_candidates {
            let mut pdb_tracks = Vec::<UsbTrack>::new();
            if let Some(pdb_id) = candidate.pdb_id {
                let mut rows = entries_by_playlist.remove(&pdb_id).unwrap_or_default();
                rows.sort_by_key(|e| e.entry_index);
                for entry in rows {
                    referenced_track_ids.insert(entry.track_id);
                    if let Some(track) = track_by_id.get(&entry.track_id) {
                        pdb_tracks.push(track.clone());
                    } else if let Some(track) = edb_track_by_id.get(&entry.track_id) {
                        pdb_tracks.push(track.clone());
                    }
                }
            }

            let export_tracks = lookup_playlist_tracks(
                &edb_playlist_tracks,
                &eedb_playlist_tracks_canonical,
                &candidate.short_name,
                &candidate.display_name,
            )
            .cloned()
            .unwrap_or_default();
            let (playlist_tracks, source) = merge_playlist_tracks(&pdb_tracks, &export_tracks);
            for t in &playlist_tracks {
                if let Ok(id) = t.id.parse::<u32>() {
                    referenced_track_ids.insert(id);
                }
            }
            if playlist_tracks.is_empty() {
                empty_source_playlists.push(candidate.display_name.clone());
            }
            *source_counts.entry(source).or_insert(0) += 1;
            playlist_entries_total += playlist_tracks.len();

            let (total_duration_ms, duration_unknown_count) = duration_totals(&playlist_tracks);
            items.push(UsbPlaylist {
                id: candidate
                    .pdb_id
                    .map(|id| format!("usb-pl-{id}"))
                    .unwrap_or_else(|| {
                        format!(
                            "usb-pl-name-{}",
                            canonicalize_playlist_name(&candidate.display_name)
                        )
                    }),
                name: candidate.display_name.clone(),
                source: source.to_string(),
                track_count: playlist_tracks.len(),
                // Built when answering: see `with_usb_playlist_texts`.
                selected_text: String::new(),
                remove_confirm_text: String::new(),
                tracks: playlist_tracks,
                total_duration_ms,
                duration_unknown_count,
                order_edited: false,
            });
        }
        push_usb_stage_timing("resolve playlists", &mut stage_started);
        let (deduped_items, collapsed) = dedupe_usb_playlists_by_name(items);
        items = deduped_items;
        if collapsed > 0 {
            logging::debug(
                "usb-import",
                format!(
                    "collapsed {} from USB sources",
                    counted(
                        collapsed,
                        "duplicate playlist name",
                        "duplicate playlist names"
                    )
                ),
            );
        }

        on_progress(
            90,
            100,
            &tr::usb_progress_finalizing_playlist_import().render(),
        );
        let stats = UsbImportStats {
            indexed_tracks: parsed.map(|p| p.tracks.len()).unwrap_or(0),
            playlist_referenced_tracks: referenced_track_ids.len(),
            playlist_entries: playlist_entries_total,
        };
        // Per-track materialization (local `tracks` rows, `track_usb_links`,
        // ANLZ cue import) is deferred out of the import: the add-to-playlist
        // path materializes on demand (browsing pages never writes). Importing every
        // track of every playlist here meant thousands of scattered on-USB
        // ANLZ reads -- minutes on an HDD-backed stick -- for data no unviewed,
        // unadded row ever uses.

        logging::debug(
            "usb-import",
            format!("USB root in use: {}", usb_root.display()),
        );

        if !source_counts.is_empty() {
            let pdb_count = source_counts.get("pdb").copied().unwrap_or(0);
            let edb_count = source_counts.get("eDB").copied().unwrap_or(0);
            if edb_count > 0 {
                logging::debug(
                    "usb-import",
                    format!(
                        "used eDB as playlist source for {}",
                        counted(edb_count, "playlist", "playlists")
                    ),
                );
            }
            if pdb_count > 0 {
                logging::debug(
                    "usb-import",
                    format!(
                        "used PDB as playlist source for {}",
                        counted(pdb_count, "playlist", "playlists")
                    ),
                );
            }
        }
        if !empty_source_playlists.is_empty() {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.playlists.empty-source",
                &tr::usb_playlists_warning_empty_source(
                    empty_source_playlists.len(),
                    empty_source_playlists.join(", "),
                ),
            ));
        }

        let resolved = Arc::new(CachedUsbPlaylists {
            items,
            stats,
            warnings,
        });
        if let Ok(mut guard) = self.usb_playlists_cache.lock() {
            *guard = Some((cache_key, Arc::clone(&resolved)));
        }
        self.finish_usb_playlists(&usb_root, &resolved)
    }

    pub fn fetch_usb_histories(
        &self,
        req: FetchUsbHistoriesRequest,
    ) -> BackendResult<FetchUsbHistoriesData> {
        self.fetch_usb_histories_with_progress(req, |_, _, _| {})
    }

    pub fn remove_usb_playlist(
        &self,
        req: RemoveUsbPlaylistRequest,
    ) -> BackendResult<RemoveUsbPlaylistData> {
        self.remove_usb_playlist_with_progress(req, |_, _, _| {})
    }

    pub fn remove_usb_playlist_with_progress<F>(
        &self,
        req: RemoveUsbPlaylistRequest,
        mut on_progress: F,
    ) -> BackendResult<RemoveUsbPlaylistData>
    where
        F: FnMut(usize, usize, &str),
    {
        let mut warnings = Vec::<WarningEntry>::new();
        let mut stage_started = std::time::Instant::now();
        let name = req.playlist_name.trim().to_string();
        if name.is_empty() {
            return Err(BackendError::Internal(
                "playlistName must not be empty".to_string(),
            ));
        }

        // Stage 1: Resolve USB root, name candidates (0-10%)
        on_progress(0, 100, &tr::usb_progress_resolving_usb_root().render());
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        push_usb_stage_timing("resolve usb root", &mut stage_started);

        let mut session = self.usb_write_session(
            &usb_root,
            // Removal can change the count, so it's worth a fresh
            // (best-effort) parse rather than carrying the previous
            // backup's count forward.
            super::usb_staging::SessionBackup::CountingPlaylists {
                reason: "Before playlist removal",
            },
            &mut warnings,
        );
        push_usb_stage_timing("backup usb databases", &mut stage_started);

        on_progress(
            5,
            100,
            &tr::usb_progress_resolving_playlist_identifiers().render(),
        );
        let mut name_candidates = vec![name.clone()];
        if let Some((_, leaf)) = name.rsplit_once(" / ") {
            let leaf_trimmed = leaf.trim().to_string();
            if !leaf_trimmed.is_empty()
                && !name_candidates.iter().any(|n| {
                    canonicalize_playlist_name(n) == canonicalize_playlist_name(&leaf_trimmed)
                })
            {
                name_candidates.push(leaf_trimmed);
            }
        }
        push_usb_stage_timing("resolve playlist identifiers", &mut stage_started);

        // Stage 2: Remove playlist + detect shared tracks in PDB (10-30%)
        on_progress(
            10,
            100,
            &tr::usb_progress_analyzing_shared_tracks().render(),
        );
        // Without a PDB, the eDB says which tracks only this playlist holds.
        let pdb_result = if session.databases()?.pdb().is_some() {
            let result = remove_playlist_and_tracks_from_pdb_in(
                &mut session,
                req.playlist_id.as_deref(),
                &name_candidates,
                &mut warnings,
            )?;
            session.write_back(super::usb_staging::DbKind::Pdb)?;
            result
        } else {
            edb_playlist_exclusive_tracks(&mut session, &name_candidates)?
        };
        push_usb_stage_timing("remove playlist and tracks from PDB", &mut stage_started);

        let removed_pdb = pdb_result.removed_playlist_count;
        let tracks_removed = pdb_result.exclusive_tracks.len();
        let tracks_kept_shared = pdb_result.shared_track_count;

        if tracks_kept_shared > 0 {
            warnings.push(logging::log_text(
                Level::Info,
                "usb-import",
                "usb.remove.shared-tracks-preserved",
                &tr::usb_playlists_warning_shared_tracks_kept(tracks_kept_shared),
            ));
        }

        // Stage 3: Delete exclusive track files (30-65%)
        on_progress(
            30,
            100,
            &tr::usb_progress_deleting_exclusive_track_files().render(),
        );
        let mut stale_paths = Vec::<String>::new();
        let mut exclusive_track_file_paths = Vec::<String>::new();

        for track in &pdb_result.exclusive_tracks {
            // Audio file
            if !track.track_file_path.is_empty() {
                stale_paths.push(track.track_file_path.clone());
                exclusive_track_file_paths.push(track.track_file_path.clone());
            }
            // ANLZ bundle (.DAT/.EXT/.2EX)
            if !track.anlz_path.is_empty() {
                for variant in analysis_bundle_path_variants(&track.anlz_path) {
                    stale_paths.push(variant);
                }
            }
        }

        for art_path in &pdb_result.exclusive_artwork_paths {
            stale_paths.extend(artwork_size_variants(art_path));
        }
        // Artwork files for exclusive artwork_ids
        let exclusive_artwork_ids: std::collections::HashSet<u32> = pdb_result
            .exclusive_tracks
            .iter()
            .filter(|t| t.artwork_id != 0)
            .map(|t| t.artwork_id)
            .collect();
        // Read artwork paths from parsed PDB artworks map
        if !exclusive_artwork_ids.is_empty()
            && let Some(pdb_path) = session.databases()?.pdb()
            && let Ok(staged) = self.parsed_staged_pdb(pdb_path)
            && let parsed = &staged.parsed
        {
            for art_id in &exclusive_artwork_ids {
                if let Some(art_path) = parsed.artworks.get(art_id) {
                    stale_paths.extend(artwork_size_variants(art_path));
                }
            }
        }

        let mut files_deleted = 0usize;
        if !stale_paths.is_empty() {
            let prune_result = prune_stale_export_owned_files(&usb_root, &stale_paths)?;
            files_deleted = prune_result.removed;
            if prune_result.missing > 0 {
                warnings.push(logging::log_text(
                    Level::Warn,
                    "usb-import",
                    "usb.remove.file-missing",
                    &tr::usb_playlists_warning_files_missing(prune_result.missing),
                ));
            }
            for path in &stale_paths {
                warnings.push(logging::log_text(
                    Level::Info,
                    "usb-import",
                    "usb.remove.file-deleted",
                    &tr::usb_playlists_warning_file_deleted(path),
                ));
            }
        }
        push_usb_stage_timing("delete exclusive track files", &mut stage_started);

        // Stage 4: Remove from eDB (65-80%)
        on_progress(65, 100, &tr::usb_progress_cleaning_edb().render());
        let removed_edb = remove_playlist_from_edb(
            &mut session,
            &name_candidates,
            &exclusive_track_file_paths,
            &mut warnings,
        )?;
        if let Err(err) = session.write_back(super::usb_staging::DbKind::Edb) {
            warnings.push(logging::log_text(
                Level::Error,
                "usb-import",
                "usb.remove.edb-write-back-failed",
                &tr::usb_warning_edb_write_back_failed(err),
            ));
        }
        push_usb_stage_timing("remove playlist from eDB", &mut stage_started);

        // Stage 5: Clean up empty directories under Contents/ (80-95%)
        on_progress(
            80,
            100,
            &tr::usb_progress_cleaning_empty_directories().render(),
        );
        let contents_dir = usb_root.join("Contents");
        if contents_dir.is_dir() {
            cleanup_empty_dirs_recursive(&contents_dir);
        }
        push_usb_stage_timing("cleanup empty directories", &mut stage_started);

        if removed_edb == 0 && removed_pdb == 0 {
            return Err(BackendError::NotFound(tr::error_usb_playlist_not_found(
                &name,
            )));
        }

        // Stage 6: Finalize (95-100%)
        on_progress(95, 100, &tr::usb_progress_finalizing().render());
        warnings.extend(self.refresh_engine_library(&usb_root));
        push_usb_stage_timing("finalize playlist removal", &mut stage_started);
        on_progress(
            100,
            100,
            &tr::usb_progress_playlist_removal_completed().render(),
        );
        session.commit()?;

        let warning_count = logging::count_warnings(&warnings);
        Ok(RemoveUsbPlaylistData {
            status_text: tr::usb_playlists_removed(&name, removed_edb, removed_pdb).render(),
            playlist_name: name,
            removed_from_edb: removed_edb,
            removed_from_pdb: removed_pdb,
            tracks_removed,
            files_deleted,
            tracks_kept_shared,
            warning_text: warning_text(warning_count),
            warnings,
        })
    }

    pub fn fetch_usb_histories_with_progress<F>(
        &self,
        req: FetchUsbHistoriesRequest,
        mut on_progress: F,
    ) -> BackendResult<FetchUsbHistoriesData>
    where
        F: FnMut(usize, usize, &str),
    {
        let mut stage_started = std::time::Instant::now();
        on_progress(2, 100, &tr::usb_progress_resolving_root().render());
        on_progress(10, 100, &tr::usb_progress_parsing_pdb().render());
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        push_usb_stage_timing("resolve usb root", &mut stage_started);

        let cache_key = self.usb_parse_cache_key(&usb_root);
        if let Some(hit) = self.usb_histories_cache_hit(&cache_key) {
            return Ok(with_usb_history_texts(&hit));
        }

        // Without a PDB, the eDB's `history` / `history_content` rows, read
        // into the PDB's history shape so one path lists them.
        let databases = UsbDatabases::present(&usb_root)?;
        let staged = databases
            .pdb()
            .map(|pdb_path| self.parsed_staged_pdb(pdb_path))
            .transpose()?;
        let edb_histories = match (&staged, &databases) {
            (None, Some(databases)) => databases
                .open_edb(&mut Vec::new())
                .map(|conn| crate::edb::edb_as_parsed_pdb(&conn))
                .transpose()?,
            _ => None,
        };
        if staged.is_none() && edb_histories.is_none() {
            logging::debug(
                "usb-import",
                format!("USB root in use: {}", usb_root.display()),
            );
            let warnings = vec![logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.histories.pdb-not-found",
                &tr::usb_history_warning_pdb_not_found(usb_root.display()),
            )];
            return Ok(with_usb_history_texts(&FetchUsbHistoriesData {
                items: Vec::new(),
                counts: UsbHistoryCounts {
                    imported_playlists: 0,
                    imported_tracks: 0,
                    pdb_t11_playlists: 0,
                    pdb_t12_entries: 0,
                    pdb_t17_playlists: 0,
                    pdb_t18_entries: 0,
                    edb_history_rows: 0,
                    edb_history_content_rows: 0,
                },
                summary_text: String::new(),
                status_text: String::new(),
                warning_count: logging::count_warnings(&warnings),
                warning_text: String::new(),
                warnings,
            }));
        }

        let parsed: &ParsedPdb = match (&staged, &edb_histories) {
            (Some(staged), _) => &staged.parsed,
            (None, Some(edb_histories)) => edb_histories,
            (None, None) => unreachable!("returned above"),
        };
        push_usb_stage_timing("parse PDB", &mut stage_started);
        on_progress(
            30,
            100,
            &tr::usb_progress_reading_supplemental_databases().render(),
        );
        let mut supplemental_warnings = Vec::<WarningEntry>::new();
        // Open the eDB once and reuse it for every read below -- opening it
        // per-call was redundantly re-running SQLCipher key negotiation up
        // to 4x in this one stage (see try_read_track_index_from_edb_with_conn's
        // doc comment), which could dominate wall time on a large eDB even
        // on fast media and misdiagnose as "slow media suspected".
        let supplemental_edb_conn = open_edb_from_usb_root(&usb_root, &mut supplemental_warnings);
        let supplemental_edb = supplemental_edb_conn
            .as_ref()
            .and_then(|conn| self.staged_edb(&usb_root, Some(conn), &mut supplemental_warnings));
        let supplemental_track_by_id = supplemental_edb
            .as_deref()
            .map(|library| library.resolved_tracks(&usb_root))
            .unwrap_or_default();
        let mut date_created_by_track_id = build_history_track_date_index(&parsed.tracks);
        if date_created_by_track_id.is_empty() {
            date_created_by_track_id = supplemental_edb
                .as_deref()
                .map(|library| library.date_created.clone())
                .unwrap_or_default();
        }
        let export_log = match export_log::load_export_log(&usb_root) {
            Ok(log) => log,
            Err(err) => {
                logging::debug(
                    "usb-import",
                    format!("USB export log ignored: {err}", err = err.english()),
                );
                None
            }
        };
        let (edb_history_rows, edb_history_content_rows) =
            if let Some(conn) = supplemental_edb_conn.as_ref() {
                let history_rows = if table_exists(conn, "history") {
                    conn.query_row(
                        "SELECT COUNT(*) FROM history",
                        [],
                        |row: &rusqlite::Row<'_>| row.get::<_, i64>(0),
                    )
                    .ok()
                    .unwrap_or(0)
                    .max(0) as usize
                } else {
                    0
                };
                let history_content_rows = if table_exists(conn, "history_content") {
                    conn.query_row(
                        "SELECT COUNT(*) FROM history_content",
                        [],
                        |row: &rusqlite::Row<'_>| row.get::<_, i64>(0),
                    )
                    .ok()
                    .unwrap_or(0)
                    .max(0) as usize
                } else {
                    0
                };
                (history_rows, history_content_rows)
            } else {
                (0, 0)
            };
        push_usb_stage_timing_with_threshold(
            "read supplemental databases",
            &mut stage_started,
            slow_stage_threshold_ms(supplemental_track_by_id.len()),
        );
        let history_date_by_num = parsed.history_rows.iter().fold(
            std::collections::HashMap::<u32, String>::new(),
            |mut acc, row| {
                let date = row
                    .date
                    .as_deref()
                    .map(sanitize_text)
                    .filter(|v| !v.is_empty());
                let num = row.num.as_deref().and_then(parse_history_slot_id);
                if let (Some(num), Some(date)) = (num, date) {
                    acc.entry(num).or_insert(date);
                }
                acc
            },
        );

        on_progress(50, 100, &tr::usb_progress_building_track_index().render());
        let track_by_id = build_usb_track_index(parsed, &usb_root);
        push_usb_stage_timing("build track index", &mut stage_started);

        on_progress(
            70,
            100,
            &tr::usb_progress_resolving_history_entries().render(),
        );
        let (history_playlists, selected_history_entries) =
            select_history_rows(&parsed.history_playlists, &parsed.history_entries);
        let history_playlists = filter_named_history_playlists(history_playlists);

        let known_history_ids = history_playlists
            .iter()
            .map(|row| normalize_packed_id(row.id))
            .collect::<HashSet<_>>();

        let mut entries_by_history = selected_history_entries.iter().fold(
            std::collections::HashMap::<u32, Vec<_>>::new(),
            |mut acc, e| {
                let decoded =
                    decode_history_playlist_id(e.playlist_id, e.entry_index, &known_history_ids)
                        .unwrap_or_else(|| normalize_packed_id(e.playlist_id));
                acc.entry(decoded).or_default().push(e);
                acc
            },
        );

        let mut items = history_playlists
            .iter()
            .map(|row| {
                let logical_playlist_id = normalize_packed_id(row.id);
                let mut entries = entries_by_history
                    .remove(&logical_playlist_id)
                    .unwrap_or_default();
                entries.sort_by_key(|e| history_entry_sort_key(e.entry_index));
                let tracks = entries
                    .iter()
                    .filter_map(|e| {
                        e.track_id
                            .or_else(|| decode_history_track_id(e.playlist_id, e.entry_index))
                    })
                    .map(|track_id| {
                        track_by_id
                            .get(&track_id)
                            .cloned()
                            .or_else(|| supplemental_track_by_id.get(&track_id).cloned())
                            .unwrap_or_else(|| {
                                supplemental_track_by_id.get(&track_id).cloned().unwrap_or(
                                    UsbTrack {
                                        id: track_id.to_string(),
                                        local_track_id: None,
                                        title: format!("Unknown Track #{track_id}"),
                                        artist: "Unknown Artist".to_string(),
                                        album: None,
                                        year: None,
                                        genre: None,
                                        track_number: None,
                                        bpm: None,
                                        key: None,
                                        key_display: None,
                                        key_color: None,
                                        file_path: String::new(),
                                        format_ext: None,
                                        needs_hydration: false,
                                        format_compat: crate::models::FormatCompat::default(),
                                        usb_media_path: None,
                                        artwork_path: None,
                                        artwork_data_url: None,
                                        waveform_peaks_path: None,
                                        usb_analysis_path: None,
                                        usb_analysis_path_raw: None,
                                        waveform_preview: None,
                                        duration_ms: None,
                                        file_size_bytes: None,
                                    },
                                )
                            })
                    })
                    .collect::<Vec<_>>();

                let cleaned_name = sanitize_history_name(&row.name);
                let created_at = history_date_by_num
                    .get(&logical_playlist_id)
                    .cloned()
                    .or_else(|| {
                        parse_history_name_numeric_id(&cleaned_name)
                            .and_then(|n| history_date_by_num.get(&n).cloned())
                    });
                let (total_duration_ms, duration_unknown_count) = duration_totals(&tracks);
                UsbHistory {
                    id: format!("usb-h-{}", logical_playlist_id),
                    name: if cleaned_name.is_empty() {
                        format!("History {logical_playlist_id}")
                    } else {
                        cleaned_name
                    },
                    created_at,
                    can_export_tracklist: !tracks.is_empty(),
                    track_count: tracks.len(),
                    tracks,
                    total_duration_ms,
                    duration_unknown_count,
                }
            })
            .collect::<Vec<_>>();
        push_usb_stage_timing("resolve history entries", &mut stage_started);

        items.sort_by_key(|h| parse_history_numeric_id(&h.id));
        export_log::apply_history_dates_from_export_log(&mut items, export_log.as_ref());
        apply_history_dates_from_track_date_created(&mut items, &date_created_by_track_id);
        // Newest first. Only now: the date carry-forward above needs ascending order.
        items.reverse();

        on_progress(
            90,
            100,
            &tr::usb_progress_finalizing_history_import().render(),
        );
        for message in &parsed.warnings {
            logging::debug("usb-import", message);
        }
        let mut warnings = Vec::<WarningEntry>::new();
        logging::debug(
            "usb-import",
            format!(
                "history import: selected populated history table family from t11/t12 vs t17/t18: playlists={}, entries={}",
                history_playlists.len(),
                selected_history_entries.len()
            ),
        );
        logging::debug(
            "usb-import",
            format!("USB root in use: {}", usb_root.display()),
        );
        warnings.extend(supplemental_warnings);
        // Per-track materialization is deferred to `fetch_usb_history_tracks`
        // (viewed page) and the add-to-playlist path -- see the note in
        // `fetch_usb_playlists_with_progress`.

        let imported_tracks: usize = items.iter().map(|history| history.tracks.len()).sum();
        let data = FetchUsbHistoriesData {
            items,
            counts: UsbHistoryCounts {
                imported_playlists: history_playlists.len(),
                imported_tracks,
                pdb_t11_playlists: parsed
                    .history_playlists
                    .iter()
                    .filter(|row| row.source_table == 11)
                    .count(),
                pdb_t12_entries: parsed
                    .history_entries
                    .iter()
                    .filter(|row| row.source_table == 12)
                    .count(),
                pdb_t17_playlists: parsed
                    .history_playlists
                    .iter()
                    .filter(|row| row.source_table == 17)
                    .count(),
                pdb_t18_entries: parsed
                    .history_entries
                    .iter()
                    .filter(|row| row.source_table == 18)
                    .count(),
                edb_history_rows,
                edb_history_content_rows,
            },
            // Built when answering: see `with_usb_history_texts`.
            summary_text: String::new(),
            status_text: String::new(),
            warning_count: logging::count_warnings(&warnings),
            warning_text: String::new(),
            warnings,
        };
        let mut data = data;
        if staged.is_none() {
            // The rows came from the eDB: the PDB has none.
            data.counts.pdb_t11_playlists = 0;
            data.counts.pdb_t12_entries = 0;
            data.counts.pdb_t17_playlists = 0;
            data.counts.pdb_t18_entries = 0;
        }
        let data = Arc::new(data);
        if let Ok(mut guard) = self.usb_histories_cache.lock() {
            *guard = Some((cache_key, Arc::clone(&data)));
        }
        Ok(with_usb_history_texts(&data))
    }

    /// Fill in `local_track_id` for the tracks on one already-paginated page
    /// from what the local DB already knows -- this device's
    /// `track_usb_links` row, then a confident fingerprint match, then an
    /// exact `file_path` row -- so the playback highlight can match a USB row
    /// to the local track playing. Read-only: browsing a USB never writes;
    /// local rows are created when a track is added to a playlist
    /// (`materialize_usb_add_candidate`). A track with no local row keeps
    /// `None`.
    ///
    /// Returns where the page's display files are read from: a matched local
    /// track's own files first (see `usb_display_files`), so this runs
    /// before the page is hydrated.
    fn resolve_usb_track_page_local_ids(
        &self,
        tracks: &mut [UsbTrack],
        usb_root: &Path,
    ) -> BackendResult<UsbDisplaySource> {
        let conn = self.db.connect()?;
        let usb_device_id = usb_device_id_for_root(&conn, usb_root)?;
        let usb_root_paths = untainted_usb_root_paths(&conn)?;

        for track in tracks.iter_mut() {
            let file_path = track.file_path.trim();
            if file_path.is_empty() {
                continue;
            }
            let linked = match usb_device_id.as_deref() {
                Some(device_id) => conn
                    .query_row(
                        "SELECT track_id FROM track_usb_links WHERE usb_device_id = ?1 AND usb_file_path = ?2",
                        params![device_id, file_path],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?,
                None => None,
            };
            let id = match linked {
                Some(id) => Some(id),
                None => {
                    let fingerprint = build_track_match_fingerprint(
                        &track.title,
                        &track.artist,
                        track.album.as_deref(),
                    );
                    match find_confident_fingerprint_match(
                        &conn,
                        &fingerprint,
                        file_path,
                        track.duration_ms.map(|v| v as i64),
                        track.file_size_bytes,
                        &usb_root_paths,
                    )? {
                        Some(id) => Some(id),
                        None => conn
                            .query_row(
                                "SELECT id FROM tracks WHERE file_path = ?1 LIMIT 1",
                                params![file_path],
                                |row| row.get::<_, String>(0),
                            )
                            .optional()?,
                    }
                }
            };
            track.local_track_id = id;
        }
        self.build_usb_display_source(&conn, usb_root, usb_device_id, tracks)
    }

    /// Where `page`'s display files are read from: the matched local tracks'
    /// own files, else the local cache of this stick's reads, else the stick.
    fn build_usb_display_source(
        &self,
        conn: &rusqlite::Connection,
        usb_root: &Path,
        usb_device_id: Option<String>,
        page: &[UsbTrack],
    ) -> BackendResult<UsbDisplaySource> {
        let device_key = match usb_device_id {
            Some(id) => id,
            None => format!(
                "root:{}",
                super::normalize_source_root_for_matching(&usb_root.to_string_lossy())
            ),
        };
        let mut usb_roots = usb_utils::all_usb_device_root_paths(conn)?;
        usb_roots.push(usb_root.to_string_lossy().into_owned());

        let mut library = HashMap::new();
        let mut stmt = conn
            .prepare_cached("SELECT waveform_peaks_path, artwork_path FROM tracks WHERE id = ?1")?;
        for id in page
            .iter()
            .filter_map(|track| track.local_track_id.as_deref())
        {
            if library.contains_key(id) {
                continue;
            }
            let files = stmt
                .query_row(params![id], |row| {
                    Ok(LibraryDisplayFiles {
                        waveform_path: row.get(0)?,
                        artwork_path: row.get(1)?,
                    })
                })
                .optional()?;
            if let Some(files) = files {
                library.insert(id.to_string(), library_files_off_usb(files, &usb_roots));
            }
        }
        Ok(UsbDisplaySource::new(
            &self.db.data_dir(),
            usb_root,
            device_key,
            library,
        ))
    }

    /// Materialize a single USB-origin add-to-playlist candidate on demand:
    /// create/link its local `tracks` row + `track_usb_links` row and import
    /// its ANLZ cue points / beat-grid anchor (needed for a later export).
    /// Returns the resolved local track id, or `None` when the candidate
    /// carries no usable file path / the stick is gone -- in which case the
    /// caller falls back to today's "usbOrigin, unresolved" behaviour.
    pub(crate) fn materialize_usb_add_candidate(
        &self,
        candidate: &crate::models::AddTrackCandidate,
        request_usb_root: Option<&str>,
    ) -> BackendResult<Option<String>> {
        let file_path = candidate
            .file_path
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string();
        if file_path.is_empty() {
            return Ok(None);
        }
        let root_hint = candidate
            .usb_root
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(request_usb_root);
        let Ok(usb_root) = resolve_usb_root(root_hint) else {
            return Ok(None);
        };

        let analysis_path = candidate
            .usb_analysis_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        // What the candidate doesn't carry comes from the stick's own row.
        let usb_row = self.usb_row_of_file(&usb_root, &file_path);
        let usb_row = usb_row.as_ref();

        let mut track = UsbTrack {
            id: String::new(),
            local_track_id: None,
            title: candidate.title.clone(),
            artist: candidate.artist.clone(),
            album: candidate.album.clone(),
            year: usb_row.and_then(|row| row.year),
            genre: usb_row.and_then(|row| row.genre.clone()),
            track_number: candidate.track_number,
            bpm: candidate.bpm,
            key: candidate.key.clone(),
            key_display: None,
            key_color: None,
            file_path,
            format_ext: candidate.format_ext.clone(),
            usb_media_path: None,
            artwork_path: usb_row.and_then(|row| row.artwork_path.clone()),
            artwork_data_url: None,
            waveform_peaks_path: analysis_path.clone(),
            usb_analysis_path: analysis_path.clone(),
            usb_analysis_path_raw: None,
            waveform_preview: None,
            duration_ms: usb_row.and_then(|row| row.duration_ms),
            file_size_bytes: candidate.file_size_bytes,
            format_compat: Default::default(),
            needs_hydration: false,
        };

        let mut conn = self.db.connect()?;
        // IMMEDIATE: this reads before it writes, and a deferred transaction
        // that's beaten to the write lock fails at once with "database is
        // locked" (busy_timeout can't help a read snapshot another writer
        // has made stale) -- e.g. two adds running at once.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now_ts = now();
        let usb_device_id = usb_utils::upsert_usb_device(&tx, &usb_root, false, &now_ts)?;
        let usb_root_paths = untainted_usb_root_paths(&tx)?;
        self.materialize_usb_track_row(&tx, &mut track, &now_ts, &usb_device_id, &usb_root_paths)?;
        if let (Some(id), Some(path)) = (track.local_track_id.as_deref(), analysis_path.as_deref())
        {
            super::cues::import_anlz_cues_for_track(
                &tx,
                id,
                Path::new(path),
                Some(Path::new(&track.file_path)),
            )?;
        }
        tx.commit()?;
        Ok(track.local_track_id)
    }

    /// The stick's row of this audio file, from its parsed playlists, then its
    /// history (both read from the PDB and the eDB). Without the file reads of
    /// `usb_track_rows`, since adding a whole list asks this per track. `None`
    /// when no list holds it or the stick can't be read: the add goes on with
    /// what the candidate carries.
    fn usb_row_of_file(&self, usb_root: &Path, file_path: &str) -> Option<UsbTrack> {
        let is_file = |track: &&UsbTrack| track.file_path.trim() == file_path;
        let lookup = || -> BackendResult<Option<UsbTrack>> {
            let playlists = self.usb_playlists_resolved(usb_root)?;
            let in_playlists = playlists.items.iter().flat_map(|list| &list.tracks);
            if let Some(track) = in_playlists.into_iter().find(is_file) {
                return Ok(Some(track.clone()));
            }
            let histories = self.usb_histories_resolved(usb_root)?;
            let in_histories = histories.items.iter().flat_map(|list| &list.tracks);
            Ok(in_histories.into_iter().find(is_file).cloned())
        };
        lookup().unwrap_or_else(|err| {
            logging::debug(
                "usb-import",
                format!(
                    "Could not read the USB's row of {file_path}: {err}",
                    err = err.english()
                ),
            );
            None
        })
    }

    /// Matches an incoming USB-sourced track against existing `tracks` rows
    /// before ever creating a placeholder row, so a USB copy of a song the
    /// user already has locally links to that genuine row instead of
    /// spawning a second, disconnected one (which also structurally
    /// prevents the genuine row's `waveform_peaks_path` from ever being
    /// clobbered by USB-sourced data, since a matched row is never written
    /// to at all). See the "Confidence gate" note below for why a fingerprint
    /// match alone isn't sufficient to merge.
    fn materialize_usb_track_row(
        &self,
        tx: &rusqlite::Transaction<'_>,
        track: &mut UsbTrack,
        now_ts: &str,
        usb_device_id: &str,
        usb_root_paths: &[String],
    ) -> BackendResult<bool> {
        let file_path = track.file_path.trim();
        if file_path.is_empty() {
            return Ok(false);
        }

        let fingerprint =
            build_track_match_fingerprint(&track.title, &track.artist, track.album.as_deref());

        let matched_id = find_confident_fingerprint_match(
            tx,
            &fingerprint,
            file_path,
            track.duration_ms.map(|v| v as i64),
            track.file_size_bytes,
            usb_root_paths,
        )?;

        let id = if let Some(id) = matched_id {
            // Genuine local row: don't touch title/artist/album/file_path/
            // waveform_peaks_path -- just record that this device also has
            // a copy of it.
            id
        } else {
            let existing_id = tx
                .query_row(
                    "SELECT id FROM tracks WHERE file_path = ?1 LIMIT 1",
                    params![file_path],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;

            let id = existing_id.unwrap_or_else(|| Uuid::now_v7().to_string());
            let track_number = track.track_number.map(|v| v.max(1));
            let bpm = track.bpm.filter(|&v| v > 0.0);
            let key = track
                .key
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string);

            tx.execute(
                r#"
                INSERT INTO tracks (
                  id, title, artist, album, track_number, bpm, tonality, file_path, file_size_bytes,
                  file_modified_at, artwork_path, waveform_peaks_path, duration_ms, match_fingerprint, format_ext, created_at, updated_at,
                  release_year, genre
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13, ?15, ?14, ?14, ?16, ?17)
                ON CONFLICT(file_path) DO UPDATE SET
                  title = excluded.title,
                  artist = excluded.artist,
                  album = excluded.album,
                  track_number = COALESCE(excluded.track_number, tracks.track_number),
                  bpm = COALESCE(excluded.bpm, tracks.bpm),
                  tonality = excluded.tonality,
                  file_size_bytes = COALESCE(excluded.file_size_bytes, tracks.file_size_bytes),
                  artwork_path = excluded.artwork_path,
                  waveform_peaks_path = COALESCE(tracks.waveform_peaks_path, excluded.waveform_peaks_path),
                  duration_ms = COALESCE(excluded.duration_ms, tracks.duration_ms),
                  match_fingerprint = COALESCE(excluded.match_fingerprint, tracks.match_fingerprint),
                  format_ext = COALESCE(tracks.format_ext, excluded.format_ext),
                  release_year = COALESCE(tracks.release_year, excluded.release_year),
                  genre = COALESCE(tracks.genre, excluded.genre),
                  updated_at = excluded.updated_at
                "#,
                params![
                    id,
                    track.title,
                    track.artist,
                    track.album,
                    track_number,
                    bpm,
                    key,
                    file_path,
                    track.file_size_bytes,
                    track.artwork_path,
                    track.waveform_peaks_path,
                    track.duration_ms,
                    fingerprint,
                    now_ts,
                    crate::audio_format::format_ext_from_path(file_path),
                    track.year,
                    track.genre,
                ],
            )?;
            id
        };

        tx.execute(
            r#"
            INSERT INTO track_usb_links (id, track_id, usb_device_id, usb_file_path, first_seen_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?5)
            ON CONFLICT(usb_device_id, usb_file_path) DO UPDATE SET
              track_id = excluded.track_id,
              last_seen_at = excluded.last_seen_at
            "#,
            params![Uuid::now_v7().to_string(), id, usb_device_id, file_path, now_ts],
        )?;

        // Cue points / beat-grid anchor are NOT imported here -- that reads the
        // on-USB ANLZ bundle from disk, and the only consumers (`get_track_detail`
        // and export) need it only once the track is in a local playlist. The
        // add-to-playlist path (`materialize_usb_add_candidate` /
        // `ensure_usb_analysis_imported`) does the ANLZ import on demand.

        track.local_track_id = Some(id);
        Ok(true)
    }

    /// One-time (but safely re-runnable) cleanup of duplicate `tracks` rows
    /// created by the *pre-fix* `materialize_usb_track_row`, which matched
    /// purely by `file_path` and so always spawned a second, disconnected
    /// row for a USB copy of a song the user already had locally. This does
    /// not touch playback correctness going forward (Step 6/6c's exclusion
    /// and `track_id` fast path already handle that) -- it only removes the
    /// now-redundant duplicate rows so Library search/browse stops showing
    /// the same song twice.
    ///
    /// Missing sizes are recovered from file metadata when available. A
    /// missing duration or an unavailable size still blocks the merge.
    pub(crate) fn merge_orphaned_usb_placeholder_tracks(
        &self,
    ) -> BackendResult<crate::models::MergeUsbPlaceholderTracksData> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let usb_root_paths = untainted_usb_root_paths(&tx)?;

        let mut stmt = tx.prepare(
            "SELECT id, file_path, match_fingerprint, duration_ms, file_size_bytes FROM tracks
             WHERE match_fingerprint IS NOT NULL AND match_fingerprint != ''",
        )?;
        type Row = (String, String, Option<i64>, Option<i64>);
        let rows: Vec<(String, Row)> = stmt
            .query_map([], |row| {
                let fingerprint: String = row.get(2)?;
                Ok((
                    fingerprint,
                    (row.get(0)?, row.get(1)?, row.get(3)?, row.get(4)?),
                ))
            })?
            .collect::<Result<_, _>>()?;
        drop(stmt);

        let mut by_fingerprint = HashMap::<String, Vec<Row>>::new();
        for (fingerprint, row) in rows {
            by_fingerprint.entry(fingerprint).or_default().push(row);
        }

        let mut merged = 0usize;
        for group in by_fingerprint.into_values() {
            let (locals, placeholders): (Vec<Row>, Vec<Row>) =
                group.into_iter().partition(|(_, path, _, _)| {
                    !usb_root_paths
                        .iter()
                        .any(|root| browse_path_matches_root(path, root))
                });
            if locals.len() != 1 {
                // Zero or ambiguous local candidates for this fingerprint --
                // leave every row in the group alone.
                continue;
            }
            let (local_id, local_path, local_duration, local_size) = &locals[0];

            for (placeholder_id, placeholder_path, placeholder_duration, placeholder_size) in
                &placeholders
            {
                if !super::track_matching::placeholder_merge_candidate(
                    local_path,
                    placeholder_path,
                    *local_duration,
                    *placeholder_duration,
                    *local_size,
                    *placeholder_size,
                ) {
                    continue;
                }

                super::library::merge_track_into(&tx, placeholder_id, local_id)?;
                merged += 1;
            }
        }

        tx.commit()?;
        Ok(crate::models::MergeUsbPlaceholderTracksData { merged })
    }

    pub fn inspect_usb_track(
        &self,
        req: InspectUsbTrackRequest,
    ) -> BackendResult<InspectUsbTrackData> {
        let track_id = req.track_id.trim().parse::<u32>().map_err(|_| {
            BackendError::Internal("trackId must be a numeric USB track id".to_string())
        })?;
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut warnings = Vec::<WarningEntry>::new();

        let databases = UsbDatabases::present(&usb_root)?;
        let file_hint = req
            .file_path
            .as_deref()
            .map(normalize_text)
            .unwrap_or_default();
        let title_hint = req.title.as_deref().map(normalize_text).unwrap_or_default();
        let artist_hint = req
            .artist
            .as_deref()
            .map(normalize_text)
            .unwrap_or_default();

        let staged_parse = if let Some(pdb_path) = databases.pdb() {
            let staged = self.parsed_staged_pdb(pdb_path)?;
            for message in &staged.parsed.warnings {
                logging::debug("usb-import", message);
            }
            Some(staged)
        } else {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.pdb-not-found",
                &tr::usb_warning_pdb_not_found_db_fallback(usb_root.display()),
            ));
            None
        };
        let parsed: Option<&ParsedPdb> = staged_parse.as_deref().map(|staged| &*staged.parsed);

        let pdb_track_index = build_pdb_track_index(parsed);
        let pdb = parsed.zip(pdb_track_index.as_ref());
        let edb_index = self
            .staged_edb(&usb_root, None, &mut warnings)
            .map(|library| library.resolved_tracks(&usb_root))
            .filter(|tracks| !tracks.is_empty());

        match resolve_usb_track_from_sources(
            track_id,
            TrackHints {
                file: &file_hint,
                title: &title_hint,
                artist: &artist_hint,
            },
            &usb_root,
            pdb,
            true,
            edb_index.as_ref(),
        ) {
            Some((source, track)) => {
                // The whole row, as a page shows it: library id resolved and
                // display data filled in the same way.
                let mut tracks = vec![track];
                let files = self.resolve_usb_track_page_local_ids(&mut tracks, &usb_root)?;
                let mut track = tracks.pop().expect("one track");
                hydrate_usb_track_in_place(
                    &mut track,
                    self.key_notation()?,
                    files.formats(),
                    Some(&files),
                );
                // Inspected: whatever is still missing, this is all there is.
                track.needs_hydration = false;
                Ok(InspectUsbTrackData {
                    source,
                    track,
                    warnings,
                })
            }
            None => Err(BackendError::Validation(tr::usb_error_track_not_found(
                track_id,
            ))),
        }
    }

    /// One paginated/searched/sorted page of a USB playlist's tracks, with the
    /// waveform-preview bytes + artwork data URLs hydrated server-side for that
    /// page. The playlist list is resolved from the parse cache (or once, on a
    /// cache miss); only the returned page pays the per-track hydration I/O.
    /// Read-only: it never writes to the local DB.
    pub fn fetch_usb_playlist_tracks(
        &self,
        req: FetchUsbTracksRequest,
    ) -> BackendResult<FetchUsbTracksData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        logging::debug(
            "usb-import",
            format!("fetch_usb_playlist_tracks: loading {}", req.id),
        );
        let generation = Arc::clone(&self.usb_playlist_page_generation);
        let this_load = generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let resolve_started = std::time::Instant::now();
        let resolved = self.usb_playlists_resolved(&usb_root)?;
        let resolve_ms = resolve_started.elapsed().as_millis();
        if resolve_ms >= 50 {
            logging::debug(
                "usb-import",
                format!("USB playlist list took {resolve_ms}ms to resolve"),
            );
        }
        let playlist = resolved
            .items
            .iter()
            .find(|playlist| playlist.id == req.id)
            .ok_or_else(|| BackendError::NotFound(tr::error_usb_playlist_not_found(&req.id)))?;
        let mut tracks = playlist.tracks.clone();
        if let Some(order) = self.pending_usb_track_order(&usb_root, &req.id) {
            super::usb_order_edit::apply_pending_order(
                &mut tracks,
                &order,
                super::usb_order_edit::track_order_key,
            );
        }
        let mut data = paginate_usb_tracks(
            "fetch_usb_playlist_tracks",
            &req.id,
            tracks,
            &req,
            resolved.warnings.clone(),
        )?;
        // Local ids first: a matched track's waveform + artwork come from the
        // library's own files instead of the stick.
        let files = self.resolve_usb_track_page_local_ids(&mut data.items, &usb_root)?;
        hydrate_usb_page(
            "fetch_usb_playlist_tracks",
            &mut data.items,
            self.key_notation()?,
            &files,
            || generation.load(std::sync::atomic::Ordering::SeqCst) != this_load,
        );
        Ok(data)
    }

    /// The stick's rows for these media paths (`UsbTrack::usb_media_path`, the
    /// key the USB saves take), in order, as a fetched page shows them:
    /// read fresh from the stick's playlists, then its history, and hydrated.
    /// `None` for a path no list holds. The USB saves answer with these, and
    /// the frontend swaps every row of that file in as it is.
    pub(crate) fn usb_track_rows(
        &self,
        usb_root: &Path,
        media_paths: &[String],
    ) -> BackendResult<Vec<Option<UsbTrack>>> {
        let wanted: HashSet<&str> = media_paths.iter().map(|path| path.trim()).collect();
        let mut rows: HashMap<String, UsbTrack> = HashMap::new();
        let playlists = self.usb_playlists_resolved(usb_root)?;
        collect_usb_rows(
            &mut rows,
            &wanted,
            playlists.items.iter().flat_map(|list| &list.tracks),
        );
        if rows.len() < wanted.len() {
            let histories = self.usb_histories_resolved(usb_root)?;
            collect_usb_rows(
                &mut rows,
                &wanted,
                histories.items.iter().flat_map(|list| &list.tracks),
            );
        }
        let mut found: Vec<UsbTrack> = rows.into_values().collect();
        let files = self.resolve_usb_track_page_local_ids(&mut found, usb_root)?;
        hydrate_usb_page(
            "usb_track_rows",
            &mut found,
            self.key_notation()?,
            &files,
            || false,
        );
        let by_path: HashMap<String, UsbTrack> = found
            .into_iter()
            .filter_map(|track| Some((track.usb_media_path.as_deref()?.trim().to_string(), track)))
            .collect();
        Ok(media_paths
            .iter()
            .map(|path| by_path.get(path.trim()).cloned())
            .collect())
    }

    /// USB-history counterpart to `fetch_usb_playlist_tracks`.
    pub fn fetch_usb_history_tracks(
        &self,
        req: FetchUsbTracksRequest,
    ) -> BackendResult<FetchUsbTracksData> {
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        logging::debug(
            "usb-import",
            format!("fetch_usb_history_tracks: loading {}", req.id),
        );
        let generation = Arc::clone(&self.usb_history_page_generation);
        let this_load = generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let resolve_started = std::time::Instant::now();
        let all = self.usb_histories_resolved(&usb_root)?;
        let resolve_ms = resolve_started.elapsed().as_millis();
        if resolve_ms >= 50 {
            logging::debug(
                "usb-import",
                format!("USB history list took {resolve_ms}ms to resolve"),
            );
        }
        let history = all
            .items
            .iter()
            .find(|history| history.id == req.id)
            .ok_or_else(|| BackendError::NotFound(tr::error_usb_history_not_found(&req.id)))?;
        let mut data = paginate_usb_tracks(
            "fetch_usb_history_tracks",
            &req.id,
            history.tracks.clone(),
            &req,
            all.warnings.clone(),
        )?;
        // Local ids first: a matched track's waveform + artwork come from the
        // library's own files instead of the stick.
        let files = self.resolve_usb_track_page_local_ids(&mut data.items, &usb_root)?;
        hydrate_usb_page(
            "fetch_usb_history_tracks",
            &mut data.items,
            self.key_notation()?,
            &files,
            || generation.load(std::sync::atomic::Ordering::SeqCst) != this_load,
        );
        Ok(data)
    }

    /// Summed length of the ticked tracks of a USB playlist (`history:
    /// false`) or history, from the same cached parse their track lists
    /// page through.
    pub(super) fn usb_selection_duration(
        &self,
        usb_root: Option<&str>,
        history: bool,
        id: &str,
        track_ids: &[String],
    ) -> BackendResult<GetSelectionDurationData> {
        let usb_root = resolve_usb_root(usb_root)?;
        if history {
            let all = self.usb_histories_resolved(&usb_root)?;
            let history = all
                .items
                .iter()
                .find(|history| history.id == id)
                .ok_or_else(|| BackendError::NotFound(tr::error_usb_history_not_found(id)))?;
            Ok(selection_duration(&history.tracks, track_ids, |_| true))
        } else {
            let resolved = self.usb_playlists_resolved(&usb_root)?;
            let playlist = resolved
                .items
                .iter()
                .find(|playlist| playlist.id == id)
                .ok_or_else(|| BackendError::NotFound(tr::error_usb_playlist_not_found(id)))?;
            Ok(selection_duration(&playlist.tracks, track_ids, |_| true))
        }
    }
}

/// Hints used to disambiguate PDB rows that share a track id (rare, but the
/// PDB doesn't enforce uniqueness). Bundled into one struct rather than
/// three separate params to leave room on `resolve_usb_track_from_sources`
/// for `include_artwork_data_url` without tripping
/// `clippy::too_many_arguments`.
struct TrackHints<'a> {
    file: &'a str,
    title: &'a str,
    artist: &'a str,
}

/// Matches a single USB track id against an already-parsed PDB and/or an
/// already-built eDB track index, preferring the PDB best-match scored
/// against the supplied hints and falling back to the eDB index. Callers are
/// responsible for parsing the PDB and opening/querying the eDB exactly
/// once and passing the results in here, so this can be called once per
/// track in a batch without repeating that work.
///
/// `include_artwork_data_url` gates reading the track's cover-art image file
/// and base64-encoding it into the result. That's real per-track disk I/O
/// (plus ~33% payload inflation from base64), so batch callers hydrating
/// thousands of tracks at once should pass `false` -- the frontend already
/// falls back to loading cover art directly from `artwork_path` via Tauri's
/// asset protocol (see `buildCoverSrcCandidates`), so this is purely an
/// eager-inline optimization worth its cost only for low-volume, one-track-
/// at-a-time callers.
fn resolve_usb_track_from_sources(
    track_id: u32,
    hints: TrackHints<'_>,
    usb_root: &Path,
    // Paired together (rather than two separate params) to keep the
    // argument count down: `pdb_track_index` is always built from `parsed`
    // and the two are only ever passed as a matching pair.
    pdb: Option<(
        &ParsedPdb,
        &HashMap<u32, Vec<&crate::pdb_reader::PdbTrackRow>>,
    )>,
    include_artwork_data_url: bool,
    edb_index: Option<&HashMap<u32, UsbTrack>>,
) -> Option<(String, UsbTrack)> {
    let TrackHints {
        file: file_hint,
        title: title_hint,
        artist: artist_hint,
    } = hints;
    if let Some((parsed, pdb_track_index)) = pdb {
        let mut best: Option<(&crate::pdb_reader::PdbTrackRow, i32)> = None;
        let candidates: &[&crate::pdb_reader::PdbTrackRow] = pdb_track_index
            .get(&track_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for t in candidates.iter().copied() {
            let artist = parsed
                .artists
                .get(&t.artist_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Artist".to_string());
            let resolved_file_path =
                resolve_usb_side_path_lexical_text(usb_root, &t.track_file_path)
                    .unwrap_or_else(|| t.track_file_path.clone());
            let mut score = 0i32;
            if !file_hint.is_empty() {
                let candidate = normalize_text(&resolved_file_path);
                if candidate.contains(file_hint) || file_hint.contains(&candidate) {
                    score += 8;
                }
            }
            if !title_hint.is_empty() {
                let candidate = normalize_text(&t.title);
                if candidate.contains(title_hint) || title_hint.contains(&candidate) {
                    score += 4;
                }
            }
            if !artist_hint.is_empty() {
                let candidate = normalize_text(&artist);
                if candidate.contains(artist_hint) || artist_hint.contains(&candidate) {
                    score += 3;
                }
            }
            match best {
                Some((_, best_score)) if best_score >= score => {}
                _ => best = Some((t, score)),
            }
        }
        if let Some((t, score)) = best {
            let has_hints =
                !file_hint.is_empty() || !title_hint.is_empty() || !artist_hint.is_empty();
            if has_hints && score <= 0 {
                // Keep searching via DB fallback when ID collides and hints don't match PDB row.
            } else {
                let artist = parsed
                    .artists
                    .get(&t.artist_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown Artist".to_string());
                let album = parsed.albums.get(&t.album_id).cloned();
                let key = parsed.keys.get(&t.key_id).cloned();
                let artwork_path = parsed
                    .artworks
                    .get(&t.artwork_id)
                    .and_then(|p| resolve_usb_side_path_lexical_text(usb_root, p));
                let resolved_file_path =
                    resolve_usb_side_path_lexical_text(usb_root, &t.track_file_path)
                        .unwrap_or_else(|| t.track_file_path.clone());
                let usb_analysis_path = resolve_usb_side_path_lexical_text(usb_root, &t.anlz_path);
                let waveform_preview = usb_analysis_path
                    .as_deref()
                    .map(Path::new)
                    .and_then(load_waveform_preview_from_analysis_path);
                let format_ext = crate::audio_format::format_ext_from_path(&t.track_file_path)
                    .or_else(|| {
                        t.file_name
                            .as_deref()
                            .and_then(crate::audio_format::format_ext_from_path)
                    });
                return Some((
                    "pdb".to_string(),
                    UsbTrack {
                        id: track_id.to_string(),
                        local_track_id: None,
                        title: if t.title.is_empty() {
                            "Unknown Title".to_string()
                        } else {
                            t.title.clone()
                        },
                        artist,
                        album,
                        year: t.release_year.map(u32::from),
                        genre: parsed
                            .genres
                            .get(&t.genre_id)
                            .map(|g| g.trim().to_string())
                            .filter(|g| !g.is_empty()),
                        track_number: (t.track_number > 0).then_some(t.track_number),
                        bpm: if t.tempo_x100 > 0 {
                            Some(t.tempo_x100 as f64 / 100.0)
                        } else {
                            None
                        },
                        key,
                        key_display: None,
                        key_color: None,
                        file_path: resolved_file_path,
                        needs_hydration: false,
                        // Judged when the row is hydrated.
                        format_compat: Default::default(),
                        format_ext,
                        usb_media_path: Some(t.track_file_path.clone()),
                        artwork_data_url: include_artwork_data_url
                            .then(|| {
                                artwork_path
                                    .as_deref()
                                    .map(Path::new)
                                    .and_then(artwork_path_to_data_url)
                            })
                            .flatten(),
                        artwork_path,
                        waveform_peaks_path: usb_analysis_path.clone(),
                        usb_analysis_path,
                        usb_analysis_path_raw: Some(t.anlz_path.clone()),
                        waveform_preview,
                        duration_ms: t.duration_seconds.map(|seconds| u64::from(seconds) * 1000),
                        file_size_bytes: t.file_size_bytes.map(i64::from),
                    },
                ));
            }
        }
    }

    if let Some(index) = edb_index
        && let Some(mut track) = index.get(&track_id).cloned()
    {
        let mut file_hint_match = true;
        if !file_hint.is_empty() {
            let candidate = normalize_text(&track.file_path);
            file_hint_match = candidate.contains(file_hint) || file_hint.contains(&candidate);
        }
        if file_hint_match {
            if track.waveform_preview.is_none() {
                track.waveform_preview = track
                    .usb_analysis_path
                    .as_deref()
                    .map(Path::new)
                    .and_then(load_waveform_preview_from_analysis_path);
            }
            return Some(("eDB".to_string(), track));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        SLOW_USB_STAGE_MS, UsbDisplaySource, apply_history_dates_from_track_date_created,
        build_history_track_date_index, build_track_match_fingerprint, build_usb_track_index,
        cleanup_empty_dirs_recursive, filter_named_history_playlists, hydrate_usb_page,
        hydrate_usb_track_in_place, normalize_date_created, now, paginate_usb_tracks,
        push_usb_stage_timing_with_threshold, select_history_rows, slow_stage_threshold_ms,
    };
    use crate::models::{UsbHistory, UsbTrack};
    use crate::pdb_reader::{PdbHistoryEntryRow, PdbHistoryPlaylistRow, PdbTrackRow};
    use crate::service::usb_identity::UsbAudioFormats;
    use std::collections::HashMap;

    #[test]
    fn hydrate_usb_page_stops_reading_files_once_superseded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let art = dir.path().join("art.jpg");
        std::fs::write(&art, [0xFF, 0xD8, 0xFF, 0xD9]).expect("write art");
        let tracks: Vec<UsbTrack> = (1..=3)
            .map(|i| {
                let mut track = make_track(&i.to_string(), &format!("/USB/Contents/{i}.mp3"));
                track.artwork_path = Some(art.to_string_lossy().into_owned());
                track.artwork_data_url = None;
                track
            })
            .collect();
        let req = crate::models::FetchUsbTracksRequest::default();
        let files = UsbDisplaySource::direct(dir.path());

        // A newer load starts after the first row: later rows skip the reads
        // but keep their cheap fields.
        let mut page = paginate_usb_tracks(
            "fetch_usb_playlist_tracks",
            "pl",
            tracks.clone(),
            &req,
            Vec::new(),
        )
        .expect("page");
        let checks = std::cell::Cell::new(0);
        hydrate_usb_page(
            "fetch_usb_playlist_tracks",
            &mut page.items,
            crate::service::key_notation::KeyNotation::Classic,
            &files,
            || {
                checks.set(checks.get() + 1);
                checks.get() > 1
            },
        );
        assert!(page.items[0].artwork_data_url.is_some());
        assert!(page.items[1..].iter().all(|t| t.artwork_data_url.is_none()));
        assert_eq!(page.items.len(), 3);

        let mut page =
            paginate_usb_tracks("fetch_usb_playlist_tracks", "pl", tracks, &req, Vec::new())
                .expect("page");
        hydrate_usb_page(
            "fetch_usb_playlist_tracks",
            &mut page.items,
            crate::service::key_notation::KeyNotation::Classic,
            &files,
            || false,
        );
        assert!(page.items.iter().all(|t| t.artwork_data_url.is_some()));
    }

    #[test]
    fn hydrated_usb_rows_follow_the_sticks_usb_audio_formats() {
        let dir = tempfile::tempdir().expect("tempdir");
        let items: Vec<(String, bool)> = crate::audio_format::UsbAudioFormat::DEFAULT_ORDER
            .into_iter()
            .map(|format| (format.id().to_string(), format.id() != "flac"))
            .collect();
        crate::service::usb_identity::write_usb_audio_formats(
            dir.path(),
            &UsbAudioFormats::from_request(&items).expect("formats"),
        )
        .expect("write formats");
        let files = UsbDisplaySource::direct(dir.path());
        let mut page = vec![
            make_track("1", "/USB/Contents/a.flac"),
            make_track("2", "/USB/Contents/b.mp3"),
        ];
        hydrate_usb_page(
            "fetch_usb_playlist_tracks",
            &mut page,
            crate::service::key_notation::KeyNotation::Classic,
            &files,
            || false,
        );
        assert_eq!(
            page[0].format_compat.warning.as_deref(),
            Some("FLAC is not allowed on this USB (USB audio formats)")
        );
        assert_eq!(page[1].format_compat.warning, None);
    }

    fn make_track(id: &str, file_path: &str) -> UsbTrack {
        UsbTrack {
            id: id.to_string(),
            local_track_id: None,
            title: "T".to_string(),
            artist: "A".to_string(),
            album: None,
            year: None,
            genre: None,
            track_number: None,
            bpm: None,
            key: None,
            key_display: None,
            key_color: None,
            file_path: file_path.to_string(),
            format_ext: crate::audio_format::format_ext_from_path(file_path),
            needs_hydration: false,
            format_compat: Default::default(),
            usb_media_path: None,
            artwork_path: None,
            artwork_data_url: None,
            waveform_peaks_path: None,
            usb_analysis_path: None,
            usb_analysis_path_raw: None,
            waveform_preview: None,
            duration_ms: None,
            file_size_bytes: None,
        }
    }

    fn make_pdb_track(id: u32, date_added: Option<&str>) -> PdbTrackRow {
        PdbTrackRow {
            header_flags: 0,
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            id,
            artist_id: 0,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
            composer_id: 0,
            label_id: 0,
            remixer_id: 0,
            color_id: 0,
            rating: 0,
            bitrate_kbps: None,
            track_number: 0,
            tempo_x100: 0,
            release_year: None,
            bit_depth: None,
            duration_seconds: None,
            file_type: None,
            isrc: None,
            date_added: date_added.map(str::to_string),
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info: None,
            autoload_hotcues: None,
            title: String::new(),
            anlz_path: String::new(),
            track_file_path: String::new(),
        }
    }

    #[test]
    fn hydrate_usb_track_in_place_sets_needs_hydration_from_missing_display_data() {
        // Everything present -> no further hydration needed.
        let mut complete = make_track("1", "/USB/Contents/a.mp3");
        complete.waveform_preview = Some(vec![1, 2, 3]);
        complete.artwork_path = Some("/USB/art/a.jpg".to_string());
        complete.bpm = Some(128.0);
        complete.key = Some("8A".to_string());
        hydrate_usb_track_in_place(
            &mut complete,
            super::KeyNotation::Classic,
            &UsbAudioFormats::default(),
            None,
        );
        assert!(!complete.needs_hydration);

        // Any missing piece -> needs hydration.
        for tweak in [
            |t: &mut UsbTrack| t.bpm = None,
            |t: &mut UsbTrack| t.key = None,
            |t: &mut UsbTrack| t.waveform_preview = None,
            |t: &mut UsbTrack| {
                t.artwork_path = None;
                t.artwork_data_url = None;
            },
        ] {
            let mut track = make_track("2", "/USB/Contents/b.mp3");
            track.waveform_preview = Some(vec![1, 2, 3]);
            track.artwork_path = Some("/USB/art/b.jpg".to_string());
            track.bpm = Some(128.0);
            track.key = Some("8A".to_string());
            tweak(&mut track);
            hydrate_usb_track_in_place(
                &mut track,
                super::KeyNotation::Classic,
                &UsbAudioFormats::default(),
                None,
            );
            assert!(
                track.needs_hydration,
                "expected needs_hydration after tweak"
            );
        }
    }

    #[test]
    fn normalize_date_created_accepts_iso_datetime_prefix() {
        assert_eq!(
            normalize_date_created("2024-10-15T09:10:11Z")
                .map(|d| d.format("%Y-%m-%d").to_string()),
            Some("2024-10-15".to_string())
        );
    }

    #[test]
    fn build_history_track_date_index_uses_pdb_track_ids() {
        let index = build_history_track_date_index(&[
            make_pdb_track(1674, Some("2020-09-18")),
            make_pdb_track(917, Some("2025-12-19")),
            make_pdb_track(42, Some("")),
            make_pdb_track(43, None),
        ]);

        assert_eq!(index.get(&1674).map(String::as_str), Some("2020-09-18"));
        assert_eq!(index.get(&917).map(String::as_str), Some("2025-12-19"));
        assert!(!index.contains_key(&42));
        assert!(!index.contains_key(&43));
    }

    #[test]
    fn apply_history_dates_carries_latest_track_date_forward_in_playlist_order() {
        let mut histories = vec![
            UsbHistory {
                id: "usb-h-1".to_string(),
                name: "HISTORY 001".to_string(),
                created_at: Some("2021-09".to_string()),
                can_export_tracklist: true,
                track_count: 2,
                tracks: vec![
                    make_track("10", "/USB/Contents/A/1.mp3"),
                    make_track("20", "/USB/Contents/A/2.mp3"),
                ],
                total_duration_ms: 0,
                duration_unknown_count: 0,
            },
            UsbHistory {
                id: "usb-h-2".to_string(),
                name: "HISTORY 002".to_string(),
                created_at: Some("2021-09".to_string()),
                can_export_tracklist: true,
                track_count: 1,
                tracks: vec![make_track("30", "/USB/Contents/A/3.mp3")],
                total_duration_ms: 0,
                duration_unknown_count: 0,
            },
            UsbHistory {
                id: "usb-h-3".to_string(),
                name: "HISTORY 003".to_string(),
                created_at: Some("2021-09".to_string()),
                can_export_tracklist: true,
                track_count: 1,
                tracks: vec![make_track("40", "/USB/Contents/A/4.mp3")],
                total_duration_ms: 0,
                duration_unknown_count: 0,
            },
        ];
        let date_created_by_track_id = HashMap::from([
            (10u32, "2024-10-15".to_string()),
            (20u32, "2024-10-14".to_string()),
            (30u32, "2024-10-10".to_string()),
            (40u32, "2024-10-20".to_string()),
        ]);

        apply_history_dates_from_track_date_created(&mut histories, &date_created_by_track_id);

        assert_eq!(histories[0].created_at.as_deref(), Some("2024-10-15"));
        assert_eq!(histories[1].created_at.as_deref(), Some("2024-10-15"));
        assert_eq!(histories[2].created_at.as_deref(), Some("2024-10-20"));
    }

    #[test]
    fn apply_history_dates_from_track_date_created_preserves_existing_valid_date() {
        let mut histories = vec![UsbHistory {
            id: "usb-h-1".to_string(),
            name: "HISTORY 001".to_string(),
            created_at: Some("2026-04-03".to_string()),
            can_export_tracklist: true,
            track_count: 1,
            tracks: vec![make_track("10", "/USB/Contents/A/1.mp3")],
            total_duration_ms: 0,
            duration_unknown_count: 0,
        }];
        let date_created_by_track_id = HashMap::from([(10u32, "2024-10-15".to_string())]);

        apply_history_dates_from_track_date_created(&mut histories, &date_created_by_track_id);

        assert_eq!(histories[0].created_at.as_deref(), Some("2026-04-03"));
    }

    #[test]
    fn apply_history_dates_from_track_date_created_carries_existing_valid_date_forward() {
        let mut histories = vec![
            UsbHistory {
                id: "usb-h-1".to_string(),
                name: "HISTORY 001".to_string(),
                created_at: Some("2026-04-03".to_string()),
                can_export_tracklist: true,
                track_count: 1,
                tracks: vec![make_track("10", "/USB/Contents/A/1.mp3")],
                total_duration_ms: 0,
                duration_unknown_count: 0,
            },
            UsbHistory {
                id: "usb-h-2".to_string(),
                name: "HISTORY 002".to_string(),
                created_at: Some("2021-09".to_string()),
                can_export_tracklist: true,
                track_count: 1,
                tracks: vec![make_track("20", "/USB/Contents/A/2.mp3")],
                total_duration_ms: 0,
                duration_unknown_count: 0,
            },
        ];
        let date_created_by_track_id = HashMap::from([(20u32, "2024-10-15".to_string())]);

        apply_history_dates_from_track_date_created(&mut histories, &date_created_by_track_id);

        assert_eq!(histories[0].created_at.as_deref(), Some("2026-04-03"));
        assert_eq!(histories[1].created_at.as_deref(), Some("2026-04-03"));
    }

    #[test]
    fn select_history_rows_prefers_more_populated_t11_t12_family() {
        let playlists = vec![
            PdbHistoryPlaylistRow {
                id: 1,
                name: "HISTORY 001".to_string(),
                source_table: 17,
            },
            PdbHistoryPlaylistRow {
                id: 2,
                name: "HISTORY 002".to_string(),
                source_table: 11,
            },
            PdbHistoryPlaylistRow {
                id: 3,
                name: "".to_string(),
                source_table: 11,
            },
        ];
        let entries = vec![
            PdbHistoryEntryRow {
                track_id: Some(101),
                playlist_id: 1,
                entry_index: 1,
                source_table: 18,
            },
            PdbHistoryEntryRow {
                track_id: Some(201),
                playlist_id: 2,
                entry_index: 1,
                source_table: 12,
            },
            PdbHistoryEntryRow {
                track_id: Some(202),
                playlist_id: 3,
                entry_index: 2,
                source_table: 12,
            },
        ];

        let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
        assert_eq!(history_playlists.len(), 2);
        assert!(history_playlists.iter().all(|row| row.source_table == 11));
        assert_eq!(history_entries.len(), 2);
        assert!(history_entries.iter().all(|row| row.source_table == 12));
    }

    #[test]
    fn select_history_rows_falls_back_to_t17_t18_when_t11_t12_absent() {
        let playlists = vec![PdbHistoryPlaylistRow {
            id: 7,
            name: "HISTORY 007".to_string(),
            source_table: 17,
        }];
        let entries = vec![
            PdbHistoryEntryRow {
                track_id: Some(101),
                playlist_id: 7,
                entry_index: 1,
                source_table: 18,
            },
            PdbHistoryEntryRow {
                track_id: Some(102),
                playlist_id: 7,
                entry_index: 2,
                source_table: 18,
            },
        ];

        let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
        assert_eq!(history_playlists.len(), 1);
        assert_eq!(history_playlists[0].source_table, 17);
        assert_eq!(history_entries.len(), 2);
        assert!(history_entries.iter().all(|row| row.source_table == 18));
    }

    #[test]
    fn select_history_rows_prefers_runtime_t11_t12_even_when_t17_t18_have_seed_volume() {
        let mut playlists = Vec::new();
        for id in 1..=27u32 {
            playlists.push(PdbHistoryPlaylistRow {
                id,
                name: format!("HISTORY {id:03}"),
                source_table: 17,
            });
        }
        playlists.push(PdbHistoryPlaylistRow {
            id: 100,
            name: "HISTORY 100".to_string(),
            source_table: 11,
        });

        let mut entries = Vec::new();
        for id in 1..=27u32 {
            entries.push(PdbHistoryEntryRow {
                track_id: Some(1000 + id),
                playlist_id: id,
                entry_index: 1,
                source_table: 18,
            });
        }
        entries.push(PdbHistoryEntryRow {
            track_id: Some(2000),
            playlist_id: 100,
            entry_index: 1,
            source_table: 12,
        });

        let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
        assert_eq!(history_playlists.len(), 1);
        assert_eq!(history_playlists[0].source_table, 11);
        assert_eq!(history_entries.len(), 1);
        assert_eq!(history_entries[0].source_table, 12);
    }

    #[test]
    fn select_history_rows_uses_t11_t12_when_t17_t18_are_empty() {
        let playlists = vec![PdbHistoryPlaylistRow {
            id: 7,
            name: "HISTORY 007".to_string(),
            source_table: 11,
        }];
        let entries = vec![PdbHistoryEntryRow {
            track_id: Some(101),
            playlist_id: 7,
            entry_index: 1,
            source_table: 12,
        }];

        let (history_playlists, history_entries) = select_history_rows(&playlists, &entries);
        assert_eq!(history_playlists.len(), 1);
        assert_eq!(history_playlists[0].source_table, 11);
        assert_eq!(history_entries.len(), 1);
        assert_eq!(history_entries[0].source_table, 12);
    }

    // --- filter_named_history_playlists ---

    #[test]
    fn filter_named_history_playlists_drops_blank_seed_rows() {
        // Shaped like the real byte-level data confirmed on several unrelated
        // fresh/never-played rekordbox exports: a block of blank-name
        // template rows shipped alongside one real session.
        let playlists = vec![
            PdbHistoryPlaylistRow {
                id: 65537,
                name: String::new(),
                source_table: 17,
            },
            PdbHistoryPlaylistRow {
                id: 393221,
                name: "\u{1}".to_string(),
                source_table: 17,
            },
            PdbHistoryPlaylistRow {
                id: 10,
                name: "HISTORY 001".to_string(),
                source_table: 17,
            },
        ];

        let filtered = filter_named_history_playlists(playlists);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, 10);
    }

    #[test]
    fn filter_named_history_playlists_drops_all_seed_only_export() {
        // A fresh/never-played export: every t17 row is a blank template
        // slot, no real session recorded yet.
        let playlists = (0..22)
            .map(|id| PdbHistoryPlaylistRow {
                id,
                name: String::new(),
                source_table: 17,
            })
            .collect::<Vec<_>>();

        assert!(filter_named_history_playlists(playlists).is_empty());
    }

    #[test]
    fn filter_named_history_playlists_keeps_all_real_sessions() {
        let playlists = vec![
            PdbHistoryPlaylistRow {
                id: 1,
                name: "HISTORY 001".to_string(),
                source_table: 17,
            },
            PdbHistoryPlaylistRow {
                id: 2,
                name: "HISTORY 002".to_string(),
                source_table: 17,
            },
        ];

        assert_eq!(filter_named_history_playlists(playlists).len(), 2);
    }

    #[test]
    fn slow_stage_threshold_ms_equals_baseline_at_zero_items() {
        assert_eq!(slow_stage_threshold_ms(0), SLOW_USB_STAGE_MS);
    }

    #[test]
    fn slow_stage_threshold_ms_scales_linearly_above_baseline() {
        let baseline = slow_stage_threshold_ms(0);
        let at_1000 = slow_stage_threshold_ms(1_000);
        let at_2000 = slow_stage_threshold_ms(2_000);
        assert!(at_1000 > baseline, "threshold should grow with item count");
        // Linear: doubling the item count should double the allowance added on top of baseline.
        assert_eq!((at_2000 - baseline), (at_1000 - baseline) * 2);
    }

    // --- build_usb_track_index ---

    #[allow(clippy::too_many_arguments)]
    fn make_indexed_pdb_track(
        id: u32,
        title: &str,
        artist_id: u32,
        album_id: u32,
        key_id: u32,
        artwork_id: u32,
        track_number: u32,
        tempo_x100: u32,
        track_file_path: &str,
        anlz_path: &str,
    ) -> PdbTrackRow {
        PdbTrackRow {
            header_flags: 0,
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: Some(4096),
            master_content_id: None,
            master_db_id: None,
            id,
            artist_id,
            album_id,
            artwork_id,
            key_id,
            genre_id: 0,
            composer_id: 0,
            label_id: 0,
            remixer_id: 0,
            color_id: 0,
            rating: 0,
            bitrate_kbps: None,
            track_number,
            tempo_x100,
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(200),
            file_type: None,
            isrc: None,
            date_added: None,
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info: None,
            autoload_hotcues: None,
            title: title.to_string(),
            anlz_path: anlz_path.to_string(),
            track_file_path: track_file_path.to_string(),
        }
    }

    #[test]
    fn build_usb_track_index_resolves_metadata_and_paths() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let usb_root = tmp.path();
        let parsed = crate::pdb_reader::ParsedPdb {
            tracks: vec![make_indexed_pdb_track(
                1,
                "Song One",
                10,
                20,
                30,
                40,
                5,
                12500,
                "/Contents/Artist/Song One.mp3",
                "/PIONEER/USBANLZ/P001/ANLZ0000.DAT",
            )],
            artists: HashMap::from([(10u32, "DJ Test".to_string())]),
            albums: HashMap::from([(20u32, "Test Album".to_string())]),
            keys: HashMap::from([(30u32, "8A".to_string())]),
            artworks: HashMap::from([(40u32, "/PIONEER/Artwork/a.jpg".to_string())]),
            ..Default::default()
        };

        let index = build_usb_track_index(&parsed, usb_root);
        let track = index.get(&1).expect("track present");
        assert_eq!(track.title, "Song One");
        assert_eq!(track.artist, "DJ Test");
        assert_eq!(track.album.as_deref(), Some("Test Album"));
        assert_eq!(track.key.as_deref(), Some("8A"));
        assert_eq!(track.track_number, Some(5));
        assert_eq!(track.bpm, Some(125.0));
        assert!(track.artwork_path.is_some());
        assert!(track.file_path.ends_with("Song One.mp3"));
        assert_eq!(track.format_ext.as_deref(), Some("mp3"));
        assert!(track.usb_analysis_path.is_some());
        assert_eq!(
            track.usb_analysis_path_raw.as_deref(),
            Some("/PIONEER/USBANLZ/P001/ANLZ0000.DAT")
        );
        assert_eq!(track.duration_ms, Some(200_000));
    }

    #[test]
    fn build_usb_track_index_falls_back_when_title_artist_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let usb_root = tmp.path();
        let parsed = crate::pdb_reader::ParsedPdb {
            tracks: vec![make_indexed_pdb_track(
                2,
                "",
                999,
                0,
                0,
                0,
                0,
                0,
                "relative/song.mp3",
                "",
            )],
            ..Default::default()
        };

        let index = build_usb_track_index(&parsed, usb_root);
        let track = index.get(&2).expect("track present");
        assert_eq!(track.title, "Unknown Title");
        assert_eq!(track.artist, "Unknown Artist");
        assert_eq!(track.album, None);
        assert_eq!(track.track_number, None);
        assert_eq!(track.bpm, None);
        assert!(track.usb_analysis_path.is_none());
    }

    // --- cleanup_empty_dirs_recursive ---

    #[test]
    fn cleanup_empty_dirs_recursive_removes_empty_but_keeps_non_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("empty/nested")).unwrap();
        std::fs::create_dir_all(root.join("kept")).unwrap();
        std::fs::write(root.join("kept/file.txt"), b"data").unwrap();

        cleanup_empty_dirs_recursive(root);

        assert!(!root.join("empty").exists());
        assert!(root.join("kept").exists());
        assert!(root.join("kept/file.txt").exists());
    }

    // --- push_usb_stage_timing / push_usb_stage_timing_with_threshold ---

    #[test]
    fn push_usb_stage_timing_with_threshold_flags_slow_stage_at_zero_threshold() {
        let mut started = std::time::Instant::now();
        assert!(
            push_usb_stage_timing_with_threshold("test-stage", &mut started, 0),
            "elapsed time always exceeds a zero threshold"
        );
    }

    #[test]
    fn push_usb_stage_timing_with_threshold_not_slow_below_threshold() {
        let mut started = std::time::Instant::now();
        assert!(!push_usb_stage_timing_with_threshold(
            "test-stage",
            &mut started,
            60_000
        ));
    }

    // --- BackendService USB integration tests ---

    fn test_service() -> (tempfile::TempDir, crate::service::BackendService) {
        let dir = tempfile::tempdir().expect("service data dir");
        let service = crate::service::BackendService::new(dir.path()).expect("backend service");
        (dir, service)
    }

    fn test_usb_root() -> (tempfile::TempDir, std::path::PathBuf) {
        let td = tempfile::tempdir().expect("tempdir");
        let usb_root = td.path().join("USB_TEST");
        std::fs::create_dir_all(&usb_root).expect("create usb root");
        crate::service::initialize_usb(&usb_root).expect("initialize usb");
        (td, usb_root)
    }

    #[test]
    fn selecting_a_usb_its_diagnostics_and_playlists_share_one_pdb_parse_and_edb_read() {
        let (_data, service) = test_service();
        let (_usb, usb_root) = test_usb_root();
        let root_text = usb_root.to_string_lossy().to_string();
        let cached = |service: &crate::service::BackendService| {
            service
                .parsed_pdb_cache
                .lock()
                .expect("cache lock")
                .as_ref()
                .map(|(_, _, staged)| std::sync::Arc::clone(staged))
                .expect("a cached parse")
        };
        let cached_edb = |service: &crate::service::BackendService| {
            service
                .edb_library_cache
                .lock()
                .expect("cache lock")
                .as_ref()
                .map(|(_, _, library)| std::sync::Arc::clone(library))
                .expect("a cached eDB read")
        };

        let selected = service
            .select_usb(crate::models::SelectUsbRequest {
                path: root_text.clone(),
            })
            .expect("select usb");
        assert!(selected.usb.valid);
        // The playlist state reads the names from the parse.
        service.playlists_state().expect("playlists state");
        let first = cached(&service);
        let first_edb = cached_edb(&service);

        service
            .run_usb_diagnostics(crate::models::RunUsbDiagnosticsRequest {
                usb_root: Some(root_text.clone()),
            })
            .expect("diagnostics");
        service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(root_text.clone()),
            })
            .expect("fetch playlists");
        assert!(std::sync::Arc::ptr_eq(&first, &cached(&service)));
        assert!(std::sync::Arc::ptr_eq(&first_edb, &cached_edb(&service)));

        // A write to the stick drops it: the next reader parses the new file.
        service.invalidate_usb_parse_cache();
        service
            .run_usb_diagnostics(crate::models::RunUsbDiagnosticsRequest {
                usb_root: Some(root_text),
            })
            .expect("diagnostics again");
        assert!(!std::sync::Arc::ptr_eq(&first, &cached(&service)));
        assert!(!std::sync::Arc::ptr_eq(&first_edb, &cached_edb(&service)));
    }

    fn make_export_track(id: &str, title: &str, filename: &str) -> crate::edb::ExportTrackData {
        crate::edb::ExportTrackData {
            cues: Vec::new(),
            id: id.to_string(),
            title: title.to_string(),
            artist: "Artist".to_string(),
            album: Some("Album".to_string()),
            track_number: Some(1),
            bpm: Some(120.0),
            key: Some("8A".to_string()),
            file_path: format!("/source/{filename}"),
            file_name: filename.to_string(),
            file_timestamp_at: None,
            file_size_bytes: Some(4_000_000),
            sample_rate_hz: None,
            bit_depth: None,
            bitrate_kbps: None,
            disc_number: None,
            subtitle: None,
            comment: None,
            title_for_search: None,
            kuvo_delivery_comment: None,
            dj_play_count: None,
            rating: None,
            color_id: None,
            artist_id_lyricist: None,
            artist_id_original_artist: None,
            artist_id_remixer: None,
            artist_id_composer: None,
            genre_id: None,
            genre: None,
            label_id: None,
            isrc: None,
            release_year: None,
            release_date: None,
            recorded_date: None,
            file_type: None,
            artwork_path: None,
            waveform_peaks_path: None,
            duration_ms: Some(200_000),
            first_beat_ms: None,
            beatgrid_from_rekordbox: false,
            position: 0,
            transcode_format: None,
        }
    }

    fn make_export_manifest(
        playlist_id: &str,
        playlist_name: &str,
        usb_root: &Path,
        tracks: &[(&str, &str, &str)],
    ) -> crate::service::export_helpers::ExportManifest {
        crate::service::export_helpers::ExportManifest {
            generated_at: "2024-01-01".to_string(),
            playlist_id: playlist_id.to_string(),
            playlist_name: playlist_name.to_string(),
            usb_root: usb_root.to_string_lossy().to_string(),
            options: crate::models::ExportToUsbOptions {
                include_artwork: false,
                include_analysis: false,
                prune_stale: false,
                ..Default::default()
            },
            exported_tracks: tracks.len(),
            skipped_tracks: 0,
            warnings: Vec::new(),
            tracks: tracks
                .iter()
                .enumerate()
                .map(
                    |(i, (id, title, filename))| crate::edb::ExportManifestTrack {
                        date_added: None,
                        date_created: None,
                        label: None,
                        remixer: None,
                        composer: None,
                        from_library: true,
                        pdb_track_id: None,
                        edb_content_id: None,
                        first_beat_ms: None,
                        cues: Vec::new(),
                        id: id.to_string(),
                        master_db_id: None,
                        master_content_id: None,
                        content_link: None,
                        position: i + 1,
                        track_number: Some(1),
                        title: title.to_string(),
                        artist: "Artist".to_string(),
                        album: Some("Album".to_string()),
                        bpm: Some(120.0),
                        key: Some("8A".to_string()),
                        source_path: format!("/source/{filename}"),
                        exported_path: format!("/Contents/Artist/Album/{filename}"),
                        file_timestamp_at: None,
                        file_size_bytes: Some(4_000_000),
                        sample_rate_hz: None,
                        bit_depth: None,
                        bitrate_kbps: None,
                        disc_number: None,
                        subtitle: None,
                        comment: None,
                        title_for_search: None,
                        kuvo_delivery_comment: None,
                        dj_play_count: None,
                        rating: None,
                        color_id: None,
                        artist_id_lyricist: None,
                        artist_id_original_artist: None,
                        artist_id_remixer: None,
                        artist_id_composer: None,
                        genre_id: None,
                        genre: None,
                        label_id: None,
                        isrc: None,
                        release_year: None,
                        release_date: None,
                        recorded_date: None,
                        file_type: None,
                        owns_exported_media: true,
                        owns_artwork: false,
                        owns_waveform: false,
                        artwork_path: None,
                        waveform_path: None,
                        duration_ms: Some(200_000),
                    },
                )
                .collect(),
        }
    }

    fn seeded_playlist_usb_with_tracks(
        tracks: &[(&str, &str, &str)],
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let (dir, usb_root) = test_usb_root();
        let playlist = crate::edb::ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "My Playlist".to_string(),
            tracks: tracks
                .iter()
                .map(|(id, title, filename)| make_export_track(id, title, filename))
                .collect(),
        };
        let manifest = make_export_manifest("pl-1", "My Playlist", &usb_root, tracks);
        crate::service::export_helpers::write_pdb(
            &usb_root, &playlist, &manifest, true, None, None,
        )
        .expect("write pdb");
        (dir, usb_root)
    }

    fn seeded_playlist_usb() -> (tempfile::TempDir, std::path::PathBuf) {
        seeded_playlist_usb_with_tracks(&[("t1", "Song A", "a.mp3")])
    }

    #[test]
    fn validate_usb_root_empty_path_is_invalid() {
        let (_dir, service) = test_service();
        let result = service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: "   ".to_string(),
            })
            .expect("validate");
        assert!(!result.valid);
        assert!(!result.has_write_access);
        assert!(result.normalized_root.is_none());
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.import.path-empty")
        );
    }

    #[test]
    fn validate_usb_root_missing_path_is_invalid() {
        let (_dir, service) = test_service();
        let missing = std::env::temp_dir().join(format!("does-not-exist-{}", uuid::Uuid::now_v7()));
        let result = service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: missing.to_string_lossy().to_string(),
            })
            .expect("validate");
        assert!(!result.valid);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.import.path-not-found")
        );
    }

    #[test]
    fn validate_usb_root_bare_directory_reports_missing_pieces() {
        let (_dir, service) = test_service();
        let usb_dir = tempfile::tempdir().expect("usb dir");
        let result = service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: usb_dir.path().to_string_lossy().to_string(),
            })
            .expect("validate");
        assert!(!result.valid);
        assert!(!result.has_vendor_root);
        assert!(!result.has_contents);
        assert!(!result.has_pdb);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.import.missing-vendor-root")
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.import.missing-contents")
        );
    }

    #[test]
    fn validate_usb_root_initialized_usb_is_valid_and_registers_device() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let result = service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: usb_root.to_string_lossy().to_string(),
            })
            .expect("validate");
        assert!(result.valid);
        assert!(result.has_vendor_root);
        assert!(result.has_contents);
        assert!(result.has_pdb);
        assert!(result.has_edb);
        assert!(result.normalized_root.is_some());

        let devices = service.list_usb_devices().expect("list devices");
        assert_eq!(devices.items.len(), 1);
        assert!(devices.items[0].mounted);
    }

    #[test]
    fn validate_usb_root_stages_edb_and_verifies_it_for_the_connect() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));

        let result = service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: usb_root.to_string_lossy().to_string(),
            })
            .expect("validate");
        assert!(result.valid);

        // Mutate the eDB directly on the "USB" after connecting. A later
        // passive `stage_edb` call must trust the connect-time staging
        // `validate_usb_root` already did and not pick up the change --
        // proving staging happened once, at connect, rather than being
        // deferred to (and re-checked by) the next read.
        let edb_path = crate::service::usb_vendor_compat::vendor_edb_path(&usb_root);
        let original = std::fs::read(&edb_path).expect("read seeded edb");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&edb_path, b"changed after connect").expect("mutate edb");

        let staged = crate::service::usb_staging::stage_edb(&usb_root).expect("stage edb");
        assert_eq!(
            std::fs::read(&staged).expect("read staged edb"),
            original,
            "validate_usb_root should already have staged/verified the eDB, so a later \
             passive stage call must not pick up an out-of-band USB change"
        );
    }

    #[test]
    fn prune_usb_device_soft_deletes_and_removes_from_listing() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: usb_root.to_string_lossy().to_string(),
            })
            .expect("validate");
        let devices = service.list_usb_devices().expect("list devices");
        let id = devices.items[0].id.clone();

        let pruned = service
            .prune_usb_device(crate::models::PruneUsbDeviceRequest { id: id.clone() })
            .expect("prune");
        assert!(pruned.pruned);
        assert!(pruned.devices.is_empty(), "the list comes back without it");

        let pruned_again = service
            .prune_usb_device(crate::models::PruneUsbDeviceRequest { id })
            .expect("prune again");
        assert!(
            !pruned_again.pruned,
            "already-pruned device should be a no-op"
        );
    }

    #[test]
    fn set_usb_device_name_round_trips_through_marker_and_label() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let path = usb_root.to_string_lossy().to_string();

        let before = service
            .get_usb_device_name(crate::models::GetUsbDeviceNameRequest {
                usb_root: path.clone(),
            })
            .expect("get before naming");
        assert_eq!(before.name, None);

        let saved = service
            .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
                usb_root: path.clone(),
                name: "  Club Stick  ".to_string(),
            })
            .expect("set name");
        assert_eq!(saved.name, "Club Stick");
        assert!(saved.updated);
        assert_eq!(saved.status_text, "Drive renamed to Club Stick.");

        let same = service
            .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
                usb_root: path.clone(),
                name: "Club Stick".to_string(),
            })
            .expect("save the same name");
        assert!(!same.updated);
        assert_eq!(same.status_text, "Drive name unchanged.");

        let after = service
            .get_usb_device_name(crate::models::GetUsbDeviceNameRequest { usb_root: path })
            .expect("get after naming");
        assert_eq!(after.name, Some("Club Stick".to_string()));

        let devices = service.list_usb_devices().expect("list devices");
        assert_eq!(devices.items[0].label.as_deref(), Some("Club Stick"));
    }

    #[test]
    fn get_usb_device_name_handles_an_already_known_but_unnamed_device() {
        // Regression test: unlike a device get_usb_device_name has never
        // seen before (no usb_devices row at all, so the SQL row-mapping
        // closure never runs), a device already tracked via validate_usb_root
        // (the normal "connect a USB" flow) has a real row with label = NULL.
        // Reading that NULL column as a plain `String` instead of
        // `Option<String>` errors ("Invalid column type Null"), which is
        // exactly the real-world case a brand-new device never hits.
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let path = usb_root.to_string_lossy().to_string();

        service
            .validate_usb_root(crate::models::ValidateUsbRootRequest { path: path.clone() })
            .expect("validate creates the usb_devices row");
        let devices = service.list_usb_devices().expect("list devices");
        assert_eq!(devices.items.len(), 1);
        assert_eq!(
            devices.items[0].label, None,
            "freshly seen device must be unnamed"
        );

        let result = service
            .get_usb_device_name(crate::models::GetUsbDeviceNameRequest { usb_root: path })
            .expect("must not error on a NULL label column");
        assert_eq!(result.name, None);
    }

    #[test]
    fn set_usb_device_name_rejects_empty_name() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let err = service
            .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                name: "   ".to_string(),
            })
            .expect_err("empty name must be rejected");
        assert!(
            matches!(err, crate::error::BackendError::Validation(message) if message.render_english() == "Enter a name for this drive.")
        );
    }

    #[test]
    fn set_usb_device_name_rejects_name_already_used_by_another_drive() {
        let (_dir, service) = test_service();
        let (_usb_dir_a, usb_root_a) = test_usb_root();
        let (_usb_dir_b, usb_root_b) = test_usb_root();

        service
            .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
                usb_root: usb_root_a.to_string_lossy().to_string(),
                name: "Club Stick".to_string(),
            })
            .expect("name first drive");

        let err = service
            .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
                usb_root: usb_root_b.to_string_lossy().to_string(),
                name: "Club Stick".to_string(),
            })
            .expect_err("duplicate name must be rejected");
        assert!(matches!(err, crate::error::BackendError::Validation(_)));

        // Renaming the same drive to the name it already holds is not a collision.
        service
            .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
                usb_root: usb_root_a.to_string_lossy().to_string(),
                name: "Club Stick".to_string(),
            })
            .expect("re-setting own name must succeed");
    }

    fn get_name(
        service: &crate::service::BackendService,
        usb_root: &Path,
    ) -> crate::models::GetUsbDeviceNameData {
        service
            .get_usb_device_name(crate::models::GetUsbDeviceNameRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
            })
            .expect("get name")
    }

    fn set_name(
        service: &crate::service::BackendService,
        usb_root: &Path,
        name: &str,
    ) -> crate::error::BackendResult<()> {
        service
            .set_usb_device_name(crate::models::SetUsbDeviceNameRequest {
                usb_root: usb_root.to_string_lossy().to_string(),
                name: name.to_string(),
            })
            .map(|_| ())
    }

    /// Another stick with the same volume label: same mount path, no marker.
    fn swap_in_unnamed_drive_at(usb_root: &Path) {
        std::fs::remove_dir_all(crate::service::usb_identity::app_marker_dir(usb_root))
            .expect("remove marker");
    }

    #[test]
    fn second_drive_at_the_same_mount_path_is_unnamed_and_cannot_take_the_first_ones_name() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        set_name(&service, &usb_root, "Club Stick").expect("name first drive");

        swap_in_unnamed_drive_at(&usb_root);
        let second = get_name(&service, &usb_root);
        assert_eq!(second.name, None, "must not inherit the first drive's name");
        assert!(
            crate::service::usb_identity::read_drive_name(&usb_root).is_none(),
            "reading the name must not write one onto the drive"
        );

        let err = set_name(&service, &usb_root, "Club Stick")
            .expect_err("the first drive's name is taken");
        assert!(matches!(err, crate::error::BackendError::Validation(_)));
        set_name(&service, &usb_root, "Booth Stick").expect("a new name is fine");
        assert_eq!(
            get_name(&service, &usb_root).name.as_deref(),
            Some("Booth Stick")
        );
    }

    #[test]
    fn set_usb_device_name_rejects_a_name_that_only_differs_in_case_or_punctuation() {
        let (_dir, service) = test_service();
        let (_usb_dir_a, usb_root_a) = test_usb_root();
        let (_usb_dir_b, usb_root_b) = test_usb_root();
        set_name(&service, &usb_root_a, "Club Stick").expect("name first drive");

        let err = set_name(&service, &usb_root_b, "club-stick")
            .expect_err("same cache key as Club Stick");
        assert!(matches!(err, crate::error::BackendError::Validation(_)));

        set_name(&service, &usb_root_a, "CLUB STICK").expect("own name in another case");
        assert_eq!(
            get_name(&service, &usb_root_a).name.as_deref(),
            Some("CLUB STICK")
        );
    }

    #[test]
    fn renaming_a_drive_moves_its_cache_and_frees_the_old_name() {
        let (_dir, service) = test_service();
        let cache = tempfile::tempdir().expect("cache dir");
        let _cache_guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let (_usb_dir_a, usb_root_a) = test_usb_root();
        let (_usb_dir_b, usb_root_b) = test_usb_root();
        set_name(&service, &usb_root_a, "Club Stick").expect("name drive");
        let archived = cache.path().join("club-stick/backups/export_old.pdb");
        std::fs::create_dir_all(archived.parent().unwrap()).unwrap();
        std::fs::write(&archived, b"pdb").unwrap();

        set_name(&service, &usb_root_a, "Booth Stick").expect("rename");

        assert!(!cache.path().join("club-stick").exists());
        assert_eq!(
            std::fs::read(cache.path().join("booth-stick/backups/export_old.pdb")).unwrap(),
            b"pdb"
        );
        set_name(&service, &usb_root_b, "Club Stick").expect("the old name is free again");
        let err =
            set_name(&service, &usb_root_b, "Booth Stick").expect_err("the new name is taken");
        assert!(matches!(err, crate::error::BackendError::Validation(_)));
    }

    #[test]
    fn a_name_read_from_the_drive_counts_as_taken() {
        // Named on another computer: only the marker knows the name.
        let (_dir, service) = test_service();
        let (_usb_dir_a, usb_root_a) = test_usb_root();
        let (_usb_dir_b, usb_root_b) = test_usb_root();
        crate::service::usb_identity::write_drive_name(&usb_root_a, "Club Stick").unwrap();

        assert_eq!(
            get_name(&service, &usb_root_a).name.as_deref(),
            Some("Club Stick")
        );
        let err =
            set_name(&service, &usb_root_b, "Club Stick").expect_err("name registered when read");
        assert!(matches!(err, crate::error::BackendError::Validation(_)));
    }

    /// What this computer knows about the drive named `name` no longer
    /// matches the drive at hand: as if it were a copy on a different-size
    /// stick.
    fn remember_other_size_for(service: &crate::service::BackendService, name: &str) {
        let conn = service.db.connect().expect("connect");
        crate::service::usb_utils::set_registered_drive(&conn, name, None, Some(1)).expect("size");
    }

    #[test]
    fn a_drive_with_another_drives_name_and_a_different_size_counts_as_unnamed() {
        let (_dir, service) = test_service();
        let cache = tempfile::tempdir().expect("cache dir");
        let _cache_guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let (_usb_dir, usb_root) = test_usb_root();
        set_name(&service, &usb_root, "Club Stick").expect("name drive");
        assert!(crate::service::usb_staging::backups_dir_for(&usb_root).is_some());
        remember_other_size_for(&service, "Club Stick");

        let data = get_name(&service, &usb_root);
        assert_eq!(data.name, None);
        assert_eq!(data.name_taken_by.as_deref(), Some("Club Stick"));
        assert!(
            crate::service::usb_staging::backups_dir_for(&usb_root).is_none(),
            "no backups are archived under the other drive's name"
        );
        let devices_conn = service.db.connect().expect("connect");
        assert_eq!(
            crate::service::usb_drive_name(&devices_conn, &usb_root),
            None
        );
        let devices = service.list_usb_devices().expect("list devices");
        assert_eq!(
            devices.items[0].label.as_deref(),
            Some("Club Stick"),
            "a carried name is not written to the path's label"
        );
    }

    #[test]
    fn a_drive_with_another_drives_name_gets_its_own_without_touching_the_others() {
        let (_dir, service) = test_service();
        let cache = tempfile::tempdir().expect("cache dir");
        let _cache_guard =
            crate::service::usb_staging::set_cache_root_for_test(Some(cache.path().to_path_buf()));
        let (_usb_dir, usb_root) = test_usb_root();
        set_name(&service, &usb_root, "Club Stick").expect("name drive");
        std::fs::create_dir_all(cache.path().join("club-stick/backups")).unwrap();
        remember_other_size_for(&service, "Club Stick");
        get_name(&service, &usb_root);

        set_name(&service, &usb_root, "Booth Stick").expect("own name");

        assert!(
            cache.path().join("club-stick/backups").is_dir(),
            "the other drive keeps its cache"
        );
        assert_eq!(
            get_name(&service, &usb_root).name.as_deref(),
            Some("Booth Stick")
        );
        let (_other_dir, other_root) = test_usb_root();
        let err = set_name(&service, &other_root, "Club Stick")
            .expect_err("the other drive keeps its name");
        assert!(matches!(err, crate::error::BackendError::Validation(_)));
    }

    #[test]
    fn a_drive_with_another_drives_name_can_take_it_back() {
        // Not a copy after all (say, a resized partition): naming it the
        // same again makes it the drive that name is checked against.
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        set_name(&service, &usb_root, "Club Stick").expect("name drive");
        remember_other_size_for(&service, "Club Stick");
        assert_eq!(get_name(&service, &usb_root).name, None);

        set_name(&service, &usb_root, "Club Stick").expect("take the name back");

        let data = get_name(&service, &usb_root);
        assert_eq!(data.name.as_deref(), Some("Club Stick"));
        assert_eq!(data.name_taken_by, None);
    }

    #[test]
    fn renaming_a_drive_renames_it_on_its_playlists_last_export_lines() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        set_name(&service, &usb_root, "Club Stick").expect("name drive");
        let conn = service.db.connect().expect("db");
        conn.execute_batch(
            "INSERT INTO playlists (id, name, source, last_exported_at, last_exported_usb_root, last_exported_usb_name, created_at, updated_at)
               VALUES ('p1', 'Set', 'local', 'x', '/run/media/dj/STICK', 'Club Stick', '1', '1'),
                      ('p2', 'Other', 'local', 'x', '/run/media/dj/STICK', 'Booth Stick', '2', '2');",
        )
        .expect("seed");

        set_name(&service, &usb_root, "Main Stick").expect("rename drive");

        let names: Vec<(String, Option<String>)> = conn
            .prepare("SELECT id, last_exported_usb_name FROM playlists ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            names,
            vec![
                ("p1".to_string(), Some("Main Stick".to_string())),
                ("p2".to_string(), Some("Booth Stick".to_string())),
            ],
            "only exports to the renamed drive follow its name"
        );
    }

    #[test]
    fn the_mount_path_label_follows_a_name_changed_outside_the_app() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        set_name(&service, &usb_root, "Club Stick").expect("name drive");
        crate::service::usb_identity::write_drive_name(&usb_root, "Booth Stick").unwrap();

        assert_eq!(
            get_name(&service, &usb_root).name.as_deref(),
            Some("Booth Stick")
        );

        let devices = service.list_usb_devices().expect("list devices");
        assert_eq!(devices.items[0].label.as_deref(), Some("Booth Stick"));
    }

    #[test]
    fn pruning_a_drive_frees_its_name() {
        let (_dir, service) = test_service();
        let (_usb_dir_a, usb_root_a) = test_usb_root();
        let (_usb_dir_b, usb_root_b) = test_usb_root();
        set_name(&service, &usb_root_a, "Club Stick").expect("name drive");
        let id = service.list_usb_devices().expect("list").items[0]
            .id
            .clone();

        service
            .prune_usb_device(crate::models::PruneUsbDeviceRequest { id })
            .expect("prune");

        set_name(&service, &usb_root_b, "Club Stick").expect("name is free after pruning");
    }

    #[test]
    fn fetch_usb_playlists_returns_playlist_with_track() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();

        let result = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("fetch usb playlists");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].name, "My Playlist");
        assert_eq!(result.items[0].track_count, 1);
        assert_eq!(result.items[0].tracks[0].title, "Song A");
        assert_eq!(result.stats.indexed_tracks, 1);
    }

    #[test]
    fn fetch_usb_playlists_computes_total_duration_server_side() {
        let (_dir, usb_root) = seeded_playlist_usb_with_tracks(&[
            ("t1", "Song A", "a.mp3"),
            ("t2", "Song B", "b.mp3"),
        ]);
        let (_dir2, service) = test_service();

        let result = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("fetch usb playlists");

        assert_eq!(result.items.len(), 1);
        // make_export_track() gives every seeded track a fixed 200_000ms
        // duration, so two tracks should sum to 400_000ms with both known.
        assert_eq!(result.items[0].total_duration_ms, 400_000);
        assert_eq!(result.items[0].duration_unknown_count, 0);
    }

    #[test]
    fn fetch_usb_playlist_tracks_paginates_filters_sorts_and_hydrates_the_page() {
        let (_dir, usb_root) = seeded_playlist_usb_with_tracks(&[
            ("t1", "Charlie", "c.mp3"),
            ("t2", "Alpha", "a.flac"),
            ("t3", "Bravo", "b.wav"),
        ]);
        let (_dir2, service) = test_service();
        let root = usb_root.to_string_lossy().to_string();
        let list = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(root.clone()),
            })
            .expect("fetch playlists");
        let playlist_id = list.items[0].id.clone();

        let req =
            |over: crate::models::FetchUsbTracksRequest| crate::models::FetchUsbTracksRequest {
                usb_root: Some(root.clone()),
                id: playlist_id.clone(),
                ..over
            };

        // Page 1 of 2 in PDB entry order; whole-list totals.
        let page1 = service
            .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
                limit: 2,
                ..Default::default()
            }))
            .expect("page 1");
        assert_eq!(page1.total, 3);
        assert_eq!(page1.items.len(), 2);
        assert!(page1.has_more);
        assert_eq!(page1.total_duration_ms, 600_000);
        // The page is hydrated: format is populated (from Stage A) ...
        assert_eq!(page1.items[0].format_ext.as_deref(), Some("mp3"));
        // ... and browsing creates no local rows, so none has a local id yet.
        assert!(page1.items.iter().all(|t| t.local_track_id.is_none()));

        let page2 = service
            .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
                limit: 2,
                cursor: page1.next_cursor.clone(),
                ..Default::default()
            }))
            .expect("page 2");
        assert_eq!(page2.items.len(), 1);
        assert!(!page2.has_more);

        // An offset jumps straight to a position, with no cursor needed.
        let jumped = service
            .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
                limit: 1,
                offset: Some(1),
                ..Default::default()
            }))
            .expect("offset page");
        assert_eq!(jumped.items.len(), 1);
        assert_eq!(jumped.items[0].id, page1.items[1].id);
        assert!(jumped.has_more);

        // Server-side title sort spans the whole playlist.
        let sorted = service
            .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
                sort_by: Some("title".to_string()),
                ..Default::default()
            }))
            .expect("sorted");
        assert_eq!(
            sorted
                .items
                .iter()
                .map(|t| t.title.clone())
                .collect::<Vec<_>>(),
            ["Alpha", "Bravo", "Charlie"]
        );

        // A cursor is rejected once the sort changes.
        let err = service
            .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
                cursor: page1.next_cursor,
                sort_by: Some("title".to_string()),
                ..Default::default()
            }))
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::Internal(_)));

        // Query filters server-side.
        let filtered = service
            .fetch_usb_playlist_tracks(req(crate::models::FetchUsbTracksRequest {
                query: "alph".to_string(),
                ..Default::default()
            }))
            .expect("filtered");
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.items.len(), 1);
        assert_eq!(filtered.items[0].title, "Alpha");
    }

    #[test]
    fn fetch_usb_playlist_tracks_is_read_only_and_finds_tracks_added_since() {
        let (_dir, usb_root) = seeded_playlist_usb_with_tracks(&[
            ("t1", "Charlie", "c.mp3"),
            ("t2", "Alpha", "a.flac"),
        ]);
        let (_dir2, service) = test_service();
        let root = usb_root.to_string_lossy().to_string();
        let list = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(root.clone()),
            })
            .expect("fetch playlists");
        let fetch = || {
            service
                .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
                    usb_root: Some(root.clone()),
                    id: list.items[0].id.clone(),
                    ..Default::default()
                })
                .expect("fetch tracks")
        };
        let count = |table: &str| -> i64 {
            let conn = service.db.connect().expect("connect");
            conn.query_row(&format!("SELECT COUNT(1) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count")
        };

        let page = fetch();
        assert_eq!(count("tracks"), 0, "browsing must not create track rows");
        assert_eq!(
            count("track_usb_links"),
            0,
            "browsing must not create USB links"
        );
        assert!(page.items.iter().all(|t| t.local_track_id.is_none()));

        // Adding one to a playlist creates its local row; the next browse
        // finds it through the USB link, still without writing.
        let added = &page.items[0];
        let local_id = service
            .materialize_usb_add_candidate(
                &crate::models::AddTrackCandidate {
                    title: added.title.clone(),
                    artist: added.artist.clone(),
                    album: added.album.clone(),
                    file_path: Some(added.file_path.clone()),
                    usb_root: Some(root.clone()),
                    ..Default::default()
                },
                None,
            )
            .expect("materialize")
            .expect("local id");
        let tracks_after_add = count("tracks");

        let page = fetch();
        assert_eq!(count("tracks"), tracks_after_add);
        let found = page
            .items
            .iter()
            .find(|t| t.file_path == added.file_path)
            .expect("row");
        assert_eq!(found.local_track_id.as_deref(), Some(local_id.as_str()));
        assert!(
            page.items
                .iter()
                .filter(|t| t.file_path != added.file_path)
                .all(|t| t.local_track_id.is_none())
        );
    }

    /// A row matched to a local track shows that track's own artwork:
    /// local ids are resolved before the page is hydrated.
    #[test]
    fn adding_a_usb_track_keeps_what_the_stick_knows_about_it() {
        let (_dir, usb_root) = test_usb_root();
        let tracks = [("t1", "Song", "song.mp3")];
        let playlist = crate::edb::ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "My Playlist".to_string(),
            tracks: vec![make_export_track("t1", "Song", "song.mp3")],
        };
        let mut manifest = make_export_manifest("pl-1", "My Playlist", &usb_root, &tracks);
        manifest.tracks[0].release_year = Some(2021);
        manifest.tracks[0].genre_id = Some(1);
        manifest.tracks[0].genre = Some("Techno".to_string());
        manifest.tracks[0].owns_artwork = true;
        manifest.tracks[0].artwork_path = Some("/PIONEER/Artwork/00001/b0000001.jpg".to_string());
        crate::service::export_helpers::write_pdb(
            &usb_root, &playlist, &manifest, true, None, None,
        )
        .expect("write pdb");

        let (_dir2, service) = test_service();
        let root = usb_root.to_string_lossy().to_string();
        let list = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(root.clone()),
            })
            .expect("fetch playlists");
        let row = service
            .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
                usb_root: Some(root.clone()),
                id: list.items[0].id.clone(),
                ..Default::default()
            })
            .expect("fetch tracks")
            .items
            .remove(0);
        assert!(row.duration_ms.is_some(), "the stick must know the length");
        assert_eq!(row.year, Some(2021));

        // As the row's "+" sends it: the candidate has none of these fields.
        let add = || {
            service
                .materialize_usb_add_candidate(
                    &crate::models::AddTrackCandidate {
                        title: row.title.clone(),
                        artist: row.artist.clone(),
                        album: row.album.clone(),
                        bpm: row.bpm,
                        file_path: Some(row.file_path.clone()),
                        usb_root: Some(root.clone()),
                        usb_analysis_path: Some("/stick/PIONEER/USBANLZ/ANLZ0000.DAT".to_string()),
                        ..Default::default()
                    },
                    None,
                )
                .expect("materialize")
                .expect("local id")
        };
        let assert_complete = |local_id: String| {
            let local = service
                .get_tracks_by_ids_with_previews(crate::models::GetTracksByIdsRequest {
                    track_ids: vec![local_id],
                })
                .expect("local row")
                .items
                .remove(0);
            assert_eq!(local.duration_ms, row.duration_ms);
            assert_eq!(local.year, row.year);
            assert_eq!(local.genre, row.genre);
            assert_eq!(local.artwork_path, row.artwork_path);
            assert!(
                local.analysis_ready,
                "BPM, waveform and length are all there"
            );
        };
        let local_id = add();
        assert_complete(local_id.clone());

        // A row an earlier version added without them is completed by adding
        // the track again.
        service
            .db
            .connect()
            .expect("connect")
            .execute(
                "UPDATE tracks SET duration_ms = NULL, release_year = NULL, genre = NULL, artwork_path = NULL WHERE id = ?1",
                rusqlite::params![local_id],
            )
            .expect("strip row");
        assert_eq!(add(), local_id);
        assert_complete(local_id);
    }

    #[test]
    fn fetch_usb_playlist_tracks_takes_matched_tracks_artwork_from_the_library() {
        let (_dir, usb_root) = seeded_playlist_usb_with_tracks(&[
            ("t1", "Charlie", "c.mp3"),
            ("t2", "Alpha", "a.flac"),
        ]);
        let (data_dir, service) = test_service();
        let root = usb_root.to_string_lossy().to_string();
        let list = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(root.clone()),
            })
            .expect("fetch playlists");
        let fetch = || {
            service
                .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
                    usb_root: Some(root.clone()),
                    id: list.items[0].id.clone(),
                    ..Default::default()
                })
                .expect("fetch tracks")
        };

        let page = fetch();
        assert!(page.items.iter().all(|t| t.artwork_data_url.is_none()));

        let added = &page.items[0];
        let local_id = service
            .materialize_usb_add_candidate(
                &crate::models::AddTrackCandidate {
                    title: added.title.clone(),
                    artist: added.artist.clone(),
                    album: added.album.clone(),
                    file_path: Some(added.file_path.clone()),
                    usb_root: Some(root.clone()),
                    ..Default::default()
                },
                None,
            )
            .expect("materialize")
            .expect("local id");
        let local_art = data_dir.path().join("local-art.png");
        std::fs::write(&local_art, b"\x89PNG local").expect("write local art");
        service
            .db
            .connect()
            .expect("connect")
            .execute(
                "UPDATE tracks SET artwork_path = ?1 WHERE id = ?2",
                rusqlite::params![local_art.to_string_lossy(), local_id],
            )
            .expect("set local artwork");

        let page = fetch();
        let matched = page
            .items
            .iter()
            .find(|t| t.file_path == added.file_path)
            .expect("matched row");
        assert_eq!(matched.local_track_id.as_deref(), Some(local_id.as_str()));
        assert!(
            matched
                .artwork_data_url
                .as_deref()
                .is_some_and(|url| url.starts_with("data:image/png;base64,"))
        );
        assert!(
            page.items
                .iter()
                .filter(|t| t.file_path != added.file_path)
                .all(|t| t.artwork_data_url.is_none())
        );
    }

    /// Regression: overlapping page loads (the previous playlist's still
    /// running when the next is selected) failed with "database is locked".
    #[test]
    fn fetch_usb_playlist_tracks_survives_concurrent_calls() {
        let (_dir, usb_root) = seeded_playlist_usb_with_tracks(&[
            ("t1", "Charlie", "c.mp3"),
            ("t2", "Alpha", "a.flac"),
            ("t3", "Bravo", "b.wav"),
        ]);
        let (_dir2, service) = test_service();
        let root = usb_root.to_string_lossy().to_string();
        let list = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(root.clone()),
            })
            .expect("fetch playlists");
        let playlist_id = list.items[0].id.clone();
        let mut handles = Vec::new();
        for _ in 0..8 {
            let service = service.clone();
            let root = root.clone();
            let playlist_id = playlist_id.clone();
            handles.push(std::thread::spawn(move || {
                let mut errs = Vec::new();
                for _ in 0..50 {
                    if let Err(e) =
                        service.fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
                            usb_root: Some(root.clone()),
                            id: playlist_id.clone(),
                            ..Default::default()
                        })
                    {
                        errs.push(e.english());
                    }
                }
                errs
            }));
        }
        let errs: Vec<String> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        assert!(
            errs.is_empty(),
            "{} errors, first: {:?}",
            errs.len(),
            errs.first()
        );
    }

    #[test]
    fn fetch_usb_playlist_tracks_rejects_unknown_playlist_id() {
        let (_dir, usb_root) = seeded_playlist_usb();
        let (_dir2, service) = test_service();
        let err = service
            .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                id: "usb-pl-does-not-exist".to_string(),
                ..Default::default()
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::NotFound(_)));
    }

    // --- USB order edit (`usb_order_edit`) ---

    fn root_text(usb_root: &Path) -> Option<String> {
        Some(usb_root.to_string_lossy().to_string())
    }

    /// A stick with one playlist, "My Playlist", in both databases, holding
    /// a.mp3, b.mp3 and c.mp3 in that order.
    fn order_edit_usb() -> (tempfile::TempDir, std::path::PathBuf) {
        let tracks = [
            ("t1", "Song A", "a.mp3"),
            ("t2", "Song B", "b.mp3"),
            ("t3", "Song C", "c.mp3"),
        ];
        let (dir, usb_root) = seeded_playlist_usb_with_tracks(&tracks);
        let playlist = crate::edb::ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "My Playlist".to_string(),
            tracks: tracks
                .iter()
                .map(|(id, title, filename)| make_export_track(id, title, filename))
                .collect(),
        };
        let manifest = make_export_manifest("pl-1", "My Playlist", &usb_root, &tracks);
        crate::service::export_helpers::write_edb_playlist(&usb_root, &playlist, &manifest, true)
            .expect("write edb playlist");
        // That writes the staged copy only (a no-op without staging).
        crate::service::usb_staging::write_back_if_changed(
            &usb_root,
            crate::service::usb_staging::DbKind::Edb,
        )
        .expect("write the edb back");
        (dir, usb_root)
    }

    fn file_name_of(path: &str) -> String {
        path.replace('\\', "/")
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string()
    }

    /// The playlist's files in the order the USB view shows them.
    fn view_files(
        service: &crate::service::BackendService,
        usb_root: &Path,
        playlist_id: &str,
    ) -> Vec<(String, String)> {
        service
            .fetch_usb_playlist_tracks(crate::models::FetchUsbTracksRequest {
                usb_root: root_text(usb_root),
                id: playlist_id.to_string(),
                ..Default::default()
            })
            .expect("fetch tracks")
            .items
            .into_iter()
            .map(|track| (file_name_of(track.identity_path()), track.id))
            .collect()
    }

    fn view_names(view: &[(String, String)]) -> Vec<&str> {
        view.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// The files of the USB's PDB playlist `name`, by `entry_index`, read from
    /// the stick's own file.
    fn usb_pdb_files(usb_root: &Path, name: &str) -> Vec<String> {
        let parsed = crate::pdb_reader::parse_pdb(
            &crate::service::usb_vendor_compat::vendor_pdb_path(usb_root),
        )
        .expect("parse usb pdb");
        let playlist = parsed
            .playlist_tree
            .iter()
            .find(|row| row.name == name && !row.row_is_folder)
            .expect("pdb playlist");
        let mut entries: Vec<_> = parsed
            .playlist_entries
            .iter()
            .filter(|entry| entry.playlist_id == playlist.id)
            .collect();
        entries.sort_by_key(|entry| entry.entry_index);
        entries
            .iter()
            .map(|entry| {
                let track = parsed
                    .tracks
                    .iter()
                    .find(|track| track.id == entry.track_id)
                    .expect("pdb track");
                file_name_of(&track.track_file_path)
            })
            .collect()
    }

    /// The files of the USB's eDB playlist `name`, by `sequenceNo`, read from
    /// the stick's own file alone.
    fn usb_edb_files(usb_root: &Path, name: &str) -> Vec<String> {
        let conn = crate::edb::open_edb(
            &crate::service::usb_vendor_compat::vendor_edb_path(usb_root),
            &mut Vec::new(),
        )
        .expect("open usb edb");
        let mut stmt = conn
            .prepare(
                "SELECT c.path FROM playlist_content pc
                 JOIN playlist p ON p.playlist_id = pc.playlist_id
                 JOIN content c ON c.content_id = pc.content_id
                 WHERE p.name = ?1 AND p.attribute = 0 ORDER BY pc.sequenceNo",
            )
            .expect("prepare");
        stmt.query_map([name], |row| row.get::<_, String>(0))
            .expect("query")
            .map(|path| file_name_of(&path.expect("path")))
            .collect()
    }

    fn begin_order_edit(service: &crate::service::BackendService, usb_root: &Path) {
        service
            .begin_usb_order_edit(crate::models::UsbOrderEditRequest {
                usb_root: root_text(usb_root),
            })
            .expect("begin order edit");
    }

    fn commit_order_edit(
        service: &crate::service::BackendService,
        usb_root: &Path,
    ) -> crate::error::BackendResult<crate::models::UsbOrderEditData> {
        service.commit_usb_order_edit(crate::models::UsbOrderEditRequest {
            usb_root: root_text(usb_root),
        })
    }

    /// Moves the row of `file` to before the row of `before` (the end: `None`).
    fn move_file(
        service: &crate::service::BackendService,
        usb_root: &Path,
        playlist_id: &str,
        file: &str,
        before: Option<&str>,
    ) -> crate::error::BackendResult<crate::models::UsbOrderEditData> {
        let view = view_files(service, usb_root, playlist_id);
        let id_of = |name: &str| {
            view.iter()
                .find(|(file, _)| file == name)
                .map(|(_, id)| id.clone())
                .expect("row of file")
        };
        service.move_usb_playlist_track(crate::models::MoveUsbPlaylistTrackRequest {
            usb_root: root_text(usb_root),
            playlist_id: playlist_id.to_string(),
            move_track_id: id_of(file),
            before_track_id: before.map(id_of),
        })
    }

    fn only_playlist_id(service: &crate::service::BackendService, usb_root: &Path) -> String {
        service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: root_text(usb_root),
            })
            .expect("fetch playlists")
            .items[0]
            .id
            .clone()
    }

    /// Puts the playlists in `ids`' order with one move each, in an order edit.
    fn edit_playlist_order(
        service: &crate::service::BackendService,
        usb_root: &Path,
        ids: &[String],
    ) {
        begin_order_edit(service, usb_root);
        for id in ids {
            service
                .move_usb_playlist(crate::models::MoveUsbPlaylistRequest {
                    usb_root: root_text(usb_root),
                    playlist_id: id.clone(),
                    before_playlist_id: None,
                })
                .expect("move playlist");
        }
        commit_order_edit(service, usb_root).expect("commit order edit");
    }

    /// A staged stick, connected (`validate_usb_root`) as the app connects one.
    fn connected_order_edit_usb(
        service: &crate::service::BackendService,
    ) -> (tempfile::TempDir, std::path::PathBuf, String) {
        let (dir, usb_root) = order_edit_usb();
        service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: usb_root.to_string_lossy().to_string(),
            })
            .expect("validate usb");
        let playlist_id = only_playlist_id(service, &usb_root);
        (dir, usb_root, playlist_id)
    }

    fn usb_db_bytes(usb_root: &Path) -> (Vec<u8>, Vec<u8>) {
        (
            std::fs::read(crate::service::usb_vendor_compat::vendor_pdb_path(usb_root))
                .expect("usb pdb"),
            std::fs::read(crate::service::usb_vendor_compat::vendor_edb_path(usb_root))
                .expect("usb edb"),
        )
    }

    #[test]
    fn usb_order_edit_writes_back_to_usb_when_staging_enabled() {
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);

        begin_order_edit(&service, &usb_root);
        move_file(&service, &usb_root, &playlist_id, "c.mp3", Some("a.mp3")).expect("move");
        assert_eq!(
            view_names(&view_files(&service, &usb_root, &playlist_id)),
            ["c.mp3", "a.mp3", "b.mp3"],
            "the view shows the pending order"
        );
        assert_eq!(
            usb_pdb_files(&usb_root, "My Playlist"),
            ["a.mp3", "b.mp3", "c.mp3"]
        );
        commit_order_edit(&service, &usb_root).expect("commit");

        // Read straight from the stick's files: the staged copies don't count,
        // and the eDB must hold the order without its write-ahead log.
        let expected = ["c.mp3", "a.mp3", "b.mp3"];
        assert_eq!(usb_pdb_files(&usb_root, "My Playlist"), expected);
        assert_eq!(usb_edb_files(&usb_root, "My Playlist"), expected);
        assert_eq!(
            view_names(&view_files(&service, &usb_root, &playlist_id)),
            expected
        );
    }

    #[test]
    fn usb_order_edit_writes_one_playlist_order_to_both_databases() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let root = root_text(&usb_root);
        let names_in_order = |service: &crate::service::BackendService| {
            service
                .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                    usb_root: root.clone(),
                })
                .expect("fetch playlists")
                .items
        };

        // "A" and "B" are in both databases, "C" only in the eDB.
        for (id, name, in_pdb) in [
            ("pl-a", "A", true),
            ("pl-b", "B", true),
            ("pl-c", "C", false),
        ] {
            let tracks = [("t1", "Song", "a.mp3")];
            let playlist = crate::edb::ExportPlaylistData {
                id: id.to_string(),
                name: name.to_string(),
                tracks: vec![make_export_track("t1", "Song", "a.mp3")],
            };
            let manifest = make_export_manifest(id, name, &usb_root, &tracks);
            if in_pdb {
                crate::service::export_helpers::write_pdb(
                    &usb_root, &playlist, &manifest, true, None, None,
                )
                .expect("write pdb playlist");
            }
            crate::service::export_helpers::write_edb_playlist(
                &usb_root, &playlist, &manifest, true,
            )
            .expect("write edb playlist");
        }

        let items = names_in_order(&service);
        let id_of = |name: &str| {
            items
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.id.clone())
                .expect("playlist id")
        };
        assert!(id_of("C").starts_with("usb-pl-name-"));

        for order in [["C", "B", "A"], ["B", "C", "A"], ["A", "B", "C"]] {
            let ids: Vec<String> = order.iter().map(|name| id_of(name)).collect();
            edit_playlist_order(&service, &usb_root, &ids);

            let read: Vec<String> = names_in_order(&service)
                .into_iter()
                .map(|p| p.name)
                .collect();
            assert_eq!(read, order);

            let conn = crate::edb::open_edb_rw(&usb_root).expect("open edb");
            for (position, name) in order.iter().enumerate() {
                let sequence: i64 = conn
                    .query_row(
                        "SELECT sequenceNo FROM playlist WHERE name = ?1 AND attribute = 0",
                        [name],
                        |row| row.get(0),
                    )
                    .expect("edb row");
                assert_eq!(sequence, position as i64, "eDB place of {name}");
            }
        }
    }

    #[test]
    fn usb_order_edit_saves_order_with_only_edb_playlists() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let root = root_text(&usb_root);
        for (id, name) in [("pl-a", "A"), ("pl-b", "B")] {
            let tracks = [("t1", "Song", "a.mp3")];
            let playlist = crate::edb::ExportPlaylistData {
                id: id.to_string(),
                name: name.to_string(),
                tracks: vec![make_export_track("t1", "Song", "a.mp3")],
            };
            let manifest = make_export_manifest(id, name, &usb_root, &tracks);
            crate::service::export_helpers::write_edb_playlist(
                &usb_root, &playlist, &manifest, true,
            )
            .expect("write edb playlist");
        }
        let fetch = || {
            service
                .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                    usb_root: root.clone(),
                })
                .expect("fetch playlists")
                .items
        };
        let mut ids: Vec<String> = fetch().iter().map(|p| p.id.clone()).collect();
        ids.reverse();
        edit_playlist_order(&service, &usb_root, &ids);

        let read: Vec<String> = fetch().iter().map(|p| p.id.clone()).collect();
        assert_eq!(read, ids);
    }

    #[test]
    fn usb_order_edit_conflict_in_either_database_stops_the_save_before_writing() {
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));
        for changed in [
            crate::service::usb_vendor_compat::vendor_pdb_path,
            crate::service::usb_vendor_compat::vendor_edb_path,
        ] {
            let (_dir, service) = test_service();
            let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
            begin_order_edit(&service, &usb_root);
            move_file(&service, &usb_root, &playlist_id, "c.mp3", Some("a.mp3")).expect("move");

            // Something else writes the stick after it was staged.
            let path = changed(&usb_root);
            let bytes = std::fs::read(&path).expect("usb file");
            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::write(&path, &bytes).expect("rewrite usb file");
            let before = usb_db_bytes(&usb_root);

            let err = commit_order_edit(&service, &usb_root).expect_err("conflict");
            assert!(
                err.english().contains("changed since it was last read"),
                "{}",
                err.english()
            );
            assert_eq!(usb_db_bytes(&usb_root), before, "neither database written");
            let state = service.usb_order_edit_state(&usb_root);
            assert!(state.active && state.changed, "the pending order stays");
        }
    }

    #[test]
    fn usb_order_edit_failed_write_back_keeps_the_order_for_a_retry() {
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
        begin_order_edit(&service, &usb_root);
        move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

        // The PDB, written first, can't be written: nothing is.
        let blocker = crate::service::usb_staging::write_back_temp_path_for_test(
            &usb_root,
            crate::service::usb_staging::DbKind::Pdb,
        );
        std::fs::create_dir_all(&blocker).expect("block the pdb write-back");
        let before = usb_db_bytes(&usb_root);
        commit_order_edit(&service, &usb_root).expect_err("write-back fails");
        assert_eq!(usb_db_bytes(&usb_root), before);
        assert!(service.usb_order_edit_state(&usb_root).changed);
        assert_eq!(
            view_names(&view_files(&service, &usb_root, &playlist_id)),
            ["b.mp3", "c.mp3", "a.mp3"],
            "the view still shows the pending order"
        );

        std::fs::remove_dir(&blocker).expect("unblock");
        commit_order_edit(&service, &usb_root).expect("retry");
        let expected = ["b.mp3", "c.mp3", "a.mp3"];
        assert_eq!(usb_pdb_files(&usb_root, "My Playlist"), expected);
        assert_eq!(usb_edb_files(&usb_root, "My Playlist"), expected);
        assert!(!service.usb_order_edit_state(&usb_root).active);
    }

    #[test]
    fn usb_order_edit_partial_save_is_reported_and_cancel_leaves_the_usb_as_is() {
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
        begin_order_edit(&service, &usb_root);
        move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

        // The PDB is written, then the eDB can't be.
        let blocker = crate::service::usb_staging::write_back_temp_path_for_test(
            &usb_root,
            crate::service::usb_staging::DbKind::Edb,
        );
        std::fs::create_dir_all(&blocker).expect("block the edb write-back");
        let err = commit_order_edit(&service, &usb_root).expect_err("partial save");
        assert!(
            err.english().contains("saved to the PDB only"),
            "{}",
            err.english()
        );
        let saved = ["b.mp3", "c.mp3", "a.mp3"];
        assert_eq!(usb_pdb_files(&usb_root, "My Playlist"), saved);
        assert_eq!(
            usb_edb_files(&usb_root, "My Playlist"),
            ["a.mp3", "b.mp3", "c.mp3"]
        );

        let cancelled = service
            .cancel_usb_order_edit(crate::models::UsbOrderEditRequest {
                usb_root: root_text(&usb_root),
            })
            .expect("cancel");
        assert!(cancelled.status_text.contains("stays on the USB"));
        std::fs::remove_dir(&blocker).expect("unblock");

        // The failed save's staged eDB was dropped: a later writer has nothing to write back.
        assert!(
            !crate::service::usb_staging::write_back_if_changed(
                &usb_root,
                crate::service::usb_staging::DbKind::Edb
            )
            .expect("write back")
        );
        assert_eq!(usb_pdb_files(&usb_root, "My Playlist"), saved);
        assert_eq!(
            usb_edb_files(&usb_root, "My Playlist"),
            ["a.mp3", "b.mp3", "c.mp3"]
        );
    }

    #[test]
    fn usb_order_edit_failure_after_a_disconnect_drops_the_staged_copies() {
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
        let staged = [
            crate::service::usb_staging::stage_pdb(&usb_root).expect("staged pdb"),
            crate::service::usb_staging::stage_edb(&usb_root).expect("staged edb"),
        ];
        begin_order_edit(&service, &usb_root);
        move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

        // The stick goes away once the staged copies are patched.
        let unplug = |_: usize, _: usize, message: &str| {
            if message == crate::i18n::tr::usb_progress_order_edit_write_back().render() {
                std::fs::remove_dir_all(&usb_root).expect("unplug");
                std::fs::write(&usb_root, b"").expect("nothing to write to");
            }
        };
        service
            .commit_usb_order_edit_with_progress(
                crate::models::UsbOrderEditRequest {
                    usb_root: root_text(&usb_root),
                },
                unplug,
            )
            .expect_err("the stick is gone");
        for path in &staged {
            assert!(!path.exists(), "{} dropped", path.display());
        }
        service
            .cancel_usb_order_edit(crate::models::UsbOrderEditRequest {
                usb_root: root_text(&usb_root),
            })
            .expect("cancel");
        assert!(!service.usb_order_edit_state(&usb_root).active);
        std::fs::remove_file(&usb_root).expect("tidy up");
    }

    #[test]
    fn usb_order_edit_incomplete_edb_checkpoint_stops_the_save() {
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root, playlist_id) = connected_order_edit_usb(&service);
        begin_order_edit(&service, &usb_root);
        move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

        // A reader holding a snapshot of the staged eDB keeps its log from
        // being checkpointed.
        let reader = crate::edb::open_edb_rw(&usb_root).expect("open staged edb");
        reader
            .execute_batch("PRAGMA journal_mode=WAL;")
            .expect("wal");
        reader
            .busy_timeout(std::time::Duration::from_millis(50))
            .expect("timeout");
        reader.execute_batch("BEGIN").expect("begin");
        let _: i64 = reader
            .query_row("SELECT count(*) FROM playlist_content", [], |row| {
                row.get(0)
            })
            .expect("read");
        let before = usb_db_bytes(&usb_root);

        let err = commit_order_edit(&service, &usb_root).expect_err("checkpoint incomplete");
        assert!(
            err.english().contains("couldn't finish writing"),
            "{}",
            err.english()
        );
        assert_eq!(usb_db_bytes(&usb_root), before, "nothing written back");
        assert!(service.usb_order_edit_state(&usb_root).changed);
        drop(reader);
    }

    #[test]
    fn usb_order_edit_that_comes_out_different_in_the_two_databases_is_not_saved() {
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = order_edit_usb();
        // The eDB files "Song A" under another path than the PDB does (what the
        // strict parity repair fixes): the edit's order, keyed by path, can't
        // place it in the PDB, where it ends up last.
        let conn = crate::edb::open_edb_rw(&usb_root).expect("open edb");
        conn.execute(
            "UPDATE content SET path = replace(path, 'a.mp3', 'a-moved.mp3') WHERE path LIKE '%a.mp3'",
            [],
        )
        .expect("move a.mp3 in the edb");
        drop(conn);
        crate::service::usb_staging::write_back_if_changed(
            &usb_root,
            crate::service::usb_staging::DbKind::Edb,
        )
        .expect("write the edb back");
        service
            .validate_usb_root(crate::models::ValidateUsbRootRequest {
                path: usb_root.to_string_lossy().to_string(),
            })
            .expect("validate usb");
        let playlist_id = only_playlist_id(&service, &usb_root);

        begin_order_edit(&service, &usb_root);
        move_file(
            &service,
            &usb_root,
            &playlist_id,
            "c.mp3",
            Some("a-moved.mp3"),
        )
        .expect("move");
        let before = usb_db_bytes(&usb_root);
        let err = commit_order_edit(&service, &usb_root).expect_err("orders differ");
        let message = err.english();
        assert!(
            message.contains("My Playlist")
                && message.contains("Upgrade Export Data To Strict Parity"),
            "{message}"
        );
        assert_eq!(usb_db_bytes(&usb_root), before, "nothing written back");
        assert!(
            service.usb_order_edit_state(&usb_root).changed,
            "the edit stays on"
        );
    }

    #[test]
    fn usb_order_edit_refuses_changes_while_it_saves() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = order_edit_usb();
        let (_other_dir, other_root) = test_usb_root();
        let playlist_id = only_playlist_id(&service, &usb_root);
        begin_order_edit(&service, &usb_root);
        move_file(&service, &usb_root, &playlist_id, "a.mp3", None).expect("move");

        let during = service.clone();
        let root = root_text(&usb_root);
        let mut checked = false;
        let check = |_: usize, _: usize, message: &str| {
            if message != crate::i18n::tr::usb_progress_backing_up_usb_databases().render() {
                return;
            }
            checked = true;
            let edit = crate::models::UsbOrderEditRequest {
                usb_root: root.clone(),
            };
            assert!(during.begin_usb_order_edit(edit.clone()).is_err());
            assert!(
                during
                    .move_usb_playlist(crate::models::MoveUsbPlaylistRequest {
                        usb_root: root.clone(),
                        playlist_id: playlist_id.clone(),
                        before_playlist_id: None,
                    })
                    .is_err()
            );
            assert!(
                during
                    .sort_usb_playlist_tracks(crate::models::SortUsbPlaylistTracksRequest {
                        usb_root: root.clone(),
                        playlist_id: playlist_id.clone(),
                        sort_by: Some("title".to_string()),
                        sort_dir: None,
                    })
                    .is_err()
            );
            assert!(during.cancel_usb_order_edit(edit.clone()).is_err());
            assert!(during.commit_usb_order_edit(edit).is_err());
            assert!(
                during
                    .select_usb(crate::models::SelectUsbRequest {
                        path: other_root.to_string_lossy().to_string(),
                    })
                    .is_err(),
                "the selected USB can't change"
            );
            assert!(
                matches!(
                    during.usb_write_lock.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "the save holds the USB write lock"
            );
        };
        service
            .commit_usb_order_edit_with_progress(
                crate::models::UsbOrderEditRequest {
                    usb_root: root_text(&usb_root),
                },
                check,
            )
            .expect("commit");
        assert!(checked);
        assert_eq!(
            usb_pdb_files(&usb_root, "My Playlist"),
            ["b.mp3", "c.mp3", "a.mp3"]
        );
    }

    #[test]
    fn remove_usb_playlist_removes_from_pdb_and_edb() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();

        let result = service
            .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                playlist_id: None,
                playlist_name: "My Playlist".to_string(),
            })
            .expect("remove usb playlist");

        assert_eq!(result.removed_from_pdb, 1);

        let after = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("fetch after remove");
        assert!(after.items.iter().all(|p| p.name != "My Playlist"));
    }

    #[test]
    fn remove_usb_playlist_writes_back_edb_to_usb_when_staging_enabled() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();

        // Seed the eDB counterpart directly, before staging is enabled, so
        // it lands on the raw USB path (not a staged cache copy).
        let tracks: [(&str, &str, &str); 1] = [("t1", "Song A", "a.mp3")];
        let playlist = crate::edb::ExportPlaylistData {
            id: "pl-1".to_string(),
            name: "My Playlist".to_string(),
            tracks: tracks
                .iter()
                .map(|(id, title, filename)| make_export_track(id, title, filename))
                .collect(),
        };
        let manifest = make_export_manifest("pl-1", "My Playlist", &usb_root, &tracks);
        crate::service::export_helpers::write_edb_playlist(&usb_root, &playlist, &manifest, true)
            .expect("seed edb playlist");

        // Enable staging now: remove_usb_playlist will operate on a local
        // HDD cache copy of the eDB, not the USB-mounted file directly.
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb_staging::set_cache_root_for_test(Some(cache_dir));

        let result = service
            .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                playlist_id: None,
                playlist_name: "My Playlist".to_string(),
            })
            .expect("remove usb playlist");
        assert_eq!(result.removed_from_edb, 1);

        // Bypass staging entirely and open the raw USB-mounted eDB file --
        // the file the physical drive actually holds. Before the write-back
        // fix, the eDB deletion only ever landed in the local staged cache
        // copy and never reached this path.
        let raw_edb_path = crate::service::usb_vendor_compat::vendor_edb_path(&usb_root);
        let mut open_warnings = Vec::new();
        let conn =
            crate::edb::open_edb(&raw_edb_path, &mut open_warnings).expect("open raw usb edb");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM playlist WHERE name = ?1",
                rusqlite::params!["My Playlist"],
                |row| row.get(0),
            )
            .expect("count playlists");
        assert_eq!(
            count, 0,
            "removed playlist must be written back to the raw USB eDB, not just the staged cache copy"
        );
    }

    #[test]
    fn remove_usb_playlist_rejects_empty_name() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let err = service
            .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                playlist_id: None,
                playlist_name: "   ".to_string(),
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::Internal(_)));
    }

    #[test]
    fn remove_usb_playlist_not_found_returns_error() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let err = service
            .remove_usb_playlist(crate::models::RemoveUsbPlaylistRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                playlist_id: None,
                playlist_name: "Does Not Exist".to_string(),
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::NotFound(_)));
    }

    #[test]
    fn inspect_usb_track_finds_by_id_from_pdb() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();

        let fetched = service
            .fetch_usb_playlists(crate::models::FetchUsbPlaylistsRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
            })
            .expect("fetch playlists");
        let row = fetched.items[0].tracks[0].clone();

        let inspected = service
            .inspect_usb_track(crate::models::InspectUsbTrackRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                track_id: row.id.clone(),
                file_path: None,
                title: None,
                artist: None,
            })
            .expect("inspect usb track");
        assert_eq!(inspected.track.title, "Song A");
        // The whole row: same file as the playlist row, and inspected.
        assert_eq!(inspected.track.usb_media_path, row.usb_media_path);
        assert!(!inspected.track.needs_hydration);
    }

    #[test]
    fn inspect_usb_track_errors_for_unknown_id() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = seeded_playlist_usb();

        let err = service
            .inspect_usb_track(crate::models::InspectUsbTrackRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                track_id: "999999".to_string(),
                file_path: None,
                title: None,
                artist: None,
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::Validation(_)));
    }

    #[test]
    fn inspect_usb_track_rejects_non_numeric_id() {
        let (_dir, service) = test_service();
        let (_usb_dir, usb_root) = test_usb_root();
        let err = service
            .inspect_usb_track(crate::models::InspectUsbTrackRequest {
                usb_root: Some(usb_root.to_string_lossy().to_string()),
                track_id: "not-a-number".to_string(),
                file_path: None,
                title: None,
                artist: None,
            })
            .unwrap_err();
        assert!(matches!(err, crate::error::BackendError::Internal(_)));
    }

    #[test]
    fn fetch_usb_histories_returns_empty_when_pdb_missing() {
        let (_dir, service) = test_service();
        let usb_dir = tempfile::tempdir().expect("usb dir");

        let result = service
            .fetch_usb_histories(crate::models::FetchUsbHistoriesRequest {
                usb_root: Some(usb_dir.path().to_string_lossy().to_string()),
            })
            .expect("fetch usb histories");

        assert!(result.items.is_empty());
        assert_eq!(result.counts.imported_playlists, 0);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "usb.histories.pdb-not-found")
        );
    }

    #[test]
    fn merge_orphaned_usb_placeholder_tracks_reassigns_playlist_and_links() {
        let (_dir, service) = test_service();
        let conn = service.db.connect().expect("connect");
        let usb_root_path = "/mnt/usb-device";
        crate::service::usb_utils::upsert_usb_device(
            &conn,
            Path::new(usb_root_path),
            false,
            &now(),
        )
        .expect("seed usb device");

        let fingerprint = build_track_match_fingerprint("Song", "Artist", None);
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, created_at, updated_at)
             VALUES ('local-1', 'Song', 'Artist', '/library/song.mp3', 200000, 5000, ?1, datetime('now'), datetime('now'))",
            rusqlite::params![fingerprint],
        )
        .expect("insert local track");
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, duration_ms, file_size_bytes, match_fingerprint, created_at, updated_at)
             VALUES ('placeholder-1', 'Song', 'Artist', '/mnt/usb-device/Contents/song.mp3', 200000, 5000, ?1, datetime('now'), datetime('now'))",
            rusqlite::params![fingerprint],
        )
        .expect("insert placeholder track");
        drop(conn);

        let playlist = service
            .create_playlist(crate::models::CreatePlaylistRequest {
                name: "PL".to_string(),
            })
            .expect("create playlist");
        let conn = service.db.connect().expect("connect");
        conn.execute(
            "INSERT INTO playlist_tracks (id, playlist_id, track_id, position, added_at)
             VALUES ('pt-1', ?1, 'placeholder-1', 0, datetime('now'))",
            rusqlite::params![playlist.playlist_id],
        )
        .expect("link playlist track");
        drop(conn);

        let result = service
            .merge_orphaned_usb_placeholder_tracks()
            .expect("merge placeholders");
        assert_eq!(result.merged, 1);

        let conn = service.db.connect().expect("connect");
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE id = 'placeholder-1'",
                [],
                |r| r.get(0),
            )
            .expect("count placeholder");
        assert_eq!(remaining, 0);
        let playlist_track_id: String = conn
            .query_row(
                "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1",
                rusqlite::params![playlist.playlist_id],
                |r| r.get(0),
            )
            .expect("playlist track id");
        assert_eq!(playlist_track_id, "local-1");
    }
}
