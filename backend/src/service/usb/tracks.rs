//! USB track rows: built from the PDB and eDB, matched to library tracks,
//! hydrated and paged, and the single-track inspector.

use super::display_files::artwork_path_to_data_url;
use super::display_files::{DisplayReadCounts, UsbDisplaySource};
use super::identity::{UsbAudioFormats, usb_device_id_for_root};
use super::root::{resolve_usb_root, resolve_usb_side_path_lexical_text};
use super::staging::{UsbDatabases, UsbDatabasesExt};
use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::logging::{self, Level};
use crate::models::{
    GetSelectionDurationData, InspectUsbTrackData, InspectUsbTrackRequest, UsbTrack, WarningEntry,
};
use crate::pdb_reader::ParsedPdb;
use crate::service::analysis::normalize_text;
use crate::service::anlz_read::load_waveform_preview_from_analysis_path;
use crate::service::key_notation::KeyNotation;
use crate::service::track_list::{duration_totals, selection_duration};
use crate::service::{
    BackendService, browse_path_matches_root, build_track_match_fingerprint,
    find_confident_fingerprint_match, now, untainted_usb_root_paths,
};
use crate::utils::LeftOut;
use crate::utils::{counted, track_list_summary};
use rusqlite::{OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use uuid::Uuid;

pub(super) fn build_usb_track_index(
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

/// Groups a parsed PDB's tracks by id so callers can look a track up in
/// O(1) instead of doing `parsed.tracks.iter().filter(|t| t.id == id)` (an
/// O(pdb_size) scan) once per id being resolved -- the latter turns a batch
/// of N ids into an O(N * pdb_size) scan, which is what made hydrating a
/// large USB playlist slow regardless of how the batch was chunked. `Vec`-
/// valued because a PDB can legitimately contain more than one row for the
/// same id; `resolve_usb_track_from_sources` picks the best-scoring one.
pub(super) fn build_pdb_track_index(
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
pub(super) fn hydrate_usb_track_in_place(
    track: &mut UsbTrack,
    notation: KeyNotation,
    formats: &UsbAudioFormats,
    files: Option<&UsbDisplaySource>,
) -> DisplayReadCounts {
    let counts = files.map(|files| files.fill(track)).unwrap_or_default();
    // USB rows only carry `format_ext`, so this flags an extension players
    // don't read, or a format the stick's USB audio formats leave unticked.
    track.format_compat = crate::service::format_compat::compute_usb_format_compat(
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
        crate::service::key_notation::key_display_fields(track.key.as_deref(), notation);
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
pub(super) fn paginate_usb_tracks(
    command: &str,
    scope_id: &str,
    mut tracks: Vec<UsbTrack>,
    req: &crate::models::FetchUsbTracksRequest,
    warnings: Vec<WarningEntry>,
) -> BackendResult<crate::models::FetchUsbTracksData> {
    let (list_track_count, list_size_bytes) = crate::service::list_totals(&tracks);
    let query = crate::service::TrackQuery::new(&req.query);
    query.retain(&mut tracks);
    let (total_duration_ms, duration_unknown_count) = duration_totals(&tracks);
    let total = tracks.len();

    let sort = crate::service::TrackSort::new(req.sort_by.as_deref(), req.sort_dir.as_deref());
    sort.apply(&mut tracks);

    let signature = crate::service::build_track_cursor_signature(&[
        "track_cursor_v1",
        command,
        scope_id,
        query.as_str(),
        sort.by,
        sort.dir,
    ]);
    let start = match req.offset {
        Some(offset) => offset,
        None => {
            crate::service::decode_offset_cursor(req.cursor.as_deref(), &signature)?.unwrap_or(0)
        }
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
    let next_cursor =
        has_more.then(|| crate::service::encode_offset_cursor(&signature, next_offset));

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
            crate::utils::LeftOut::NoLength,
        ),
        warnings,
    })
}

/// Hydrates a page from `paginate_usb_tracks`. Reading each row's waveform +
/// artwork off the stick is the slow part (seconds per page on Windows).
/// Once a newer load of this list has started, the frontend discards this
/// page, so stop reading: abandoned loads would otherwise queue up on the
/// USB ahead of the one on screen.
pub(super) fn hydrate_usb_page(
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
pub(super) fn collect_usb_rows<'a>(
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

/// Hints used to disambiguate PDB rows that share a track id (rare, but the
/// PDB doesn't enforce uniqueness). Bundled into one struct rather than
/// three separate params to leave room on `resolve_usb_track_from_sources`
/// for `include_artwork_data_url` without tripping
/// `clippy::too_many_arguments`.
pub(super) struct TrackHints<'a> {
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
pub(super) fn resolve_usb_track_from_sources(
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

impl BackendService {
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
    /// track's own files first (see `usb::display_files`), so this runs
    /// before the page is hydrated.
    pub(super) fn resolve_usb_track_page_local_ids(
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
        let usb_device_id =
            crate::service::usb::identity::upsert_usb_device(&tx, &usb_root, false, &now_ts)?;
        let usb_root_paths = untainted_usb_root_paths(&tx)?;
        self.materialize_usb_track_row(&tx, &mut track, &now_ts, &usb_device_id, &usb_root_paths)?;
        if let (Some(id), Some(path)) = (track.local_track_id.as_deref(), analysis_path.as_deref())
        {
            crate::service::cues::import_anlz_cues_for_track(
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
    pub(super) fn usb_row_of_file(&self, usb_root: &Path, file_path: &str) -> Option<UsbTrack> {
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
    pub(super) fn materialize_usb_track_row(
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
                if !crate::service::track_matching::placeholder_merge_candidate(
                    local_path,
                    placeholder_path,
                    *local_duration,
                    *placeholder_duration,
                    *local_size,
                    *placeholder_size,
                ) {
                    continue;
                }

                crate::service::library::merge_track_into(&tx, placeholder_id, local_id)?;
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

    /// Summed length of the ticked tracks of a USB playlist (`history:
    /// false`) or history, from the same cached parse their track lists
    /// page through.
    pub(in crate::service) fn usb_selection_duration(
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
            Ok(selection_duration(
                &history.tracks,
                track_ids,
                |_| true,
                LeftOut::NoLength,
            ))
        } else {
            let resolved = self.usb_playlists_resolved(&usb_root)?;
            let playlist = resolved
                .items
                .iter()
                .find(|playlist| playlist.id == id)
                .ok_or_else(|| BackendError::NotFound(tr::error_usb_playlist_not_found(id)))?;
            Ok(selection_duration(
                &playlist.tracks,
                track_ids,
                |_| true,
                LeftOut::NoLength,
            ))
        }
    }
}
