//! USB History: resolving the PDB's history playlists into dated sessions,
//! and paging a session's tracks.

use super::root::resolve_usb_root;
use super::staging::{UsbDatabases, UsbDatabasesExt};
use super::text::sanitize_text;
use super::tracks::{build_usb_track_index, hydrate_usb_page, paginate_usb_tracks};
use super::{push_usb_stage_timing, push_usb_stage_timing_with_threshold, slow_stage_threshold_ms};
use crate::edb::open_edb_from_usb_root;
use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::logging::{self, Level};
use crate::models::{
    FetchUsbHistoriesData, FetchUsbHistoriesRequest, FetchUsbTracksData, FetchUsbTracksRequest,
    UsbHistory, UsbHistoryCounts, UsbTrack, WarningEntry,
};
use crate::pdb_reader::{ParsedPdb, PdbHistoryEntryRow, PdbHistoryPlaylistRow, PdbHistoryRow};
use crate::service::track_list::duration_totals;
use crate::service::{BackendService, export_log};
use crate::utils::warning_text;
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

pub(super) fn select_history_rows(
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
pub(super) fn filter_named_history_playlists(
    playlists: Vec<PdbHistoryPlaylistRow>,
) -> Vec<PdbHistoryPlaylistRow> {
    playlists
        .into_iter()
        .filter(|row| is_named_history_playlist(&row.name))
        .collect()
}

/// One database's history session with its tracks resolved.
#[derive(Debug, Clone)]
pub(super) struct HistorySession {
    name: String,
    /// The "NNN" of "HISTORY NNN".
    name_number: Option<u32>,
    pub(super) tracks: Vec<UsbTrack>,
}

/// One database's named history sessions by session number (the row ID:
/// rekordbox and the app's eDB history sync give a session the same one in
/// both databases), each entry's track looked up in `track_by_id`, that same
/// database's tracks.
pub(super) fn resolve_history_sessions(
    playlists: &[PdbHistoryPlaylistRow],
    entries: &[PdbHistoryEntryRow],
    track_by_id: &HashMap<u32, UsbTrack>,
) -> BTreeMap<u32, HistorySession> {
    let playlists = filter_named_history_playlists(playlists.to_vec());
    let known_history_ids = playlists
        .iter()
        .map(|row| normalize_packed_id(row.id))
        .collect::<HashSet<_>>();
    let mut entries_by_history = entries.iter().fold(
        HashMap::<u32, Vec<&PdbHistoryEntryRow>>::new(),
        |mut acc, e| {
            let decoded =
                decode_history_playlist_id(e.playlist_id, e.entry_index, &known_history_ids)
                    .unwrap_or_else(|| normalize_packed_id(e.playlist_id));
            acc.entry(decoded).or_default().push(e);
            acc
        },
    );
    playlists
        .iter()
        .map(|row| {
            let row_id = normalize_packed_id(row.id);
            let mut entries = entries_by_history.remove(&row_id).unwrap_or_default();
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
                        .unwrap_or_else(|| unknown_history_track(track_id))
                })
                .collect::<Vec<_>>();
            // Never empty: only "HISTORY ..." sessions get here.
            let name = sanitize_history_name(&row.name);
            let name_number = parse_history_name_numeric_id(&name);
            (
                row_id,
                HistorySession {
                    name,
                    name_number,
                    tracks,
                },
            )
        })
        .collect()
}

/// Both databases' sessions as one list, matched by session number, each
/// with whether both databases have it and the database its tracks come
/// from: the PDB's when it has tracks,
/// else the eDB's when that has any, else whichever has the session.
pub(super) fn merge_history_sessions(
    mut pdb: BTreeMap<u32, HistorySession>,
    mut edb: BTreeMap<u32, HistorySession>,
) -> BTreeMap<u32, (HistorySession, &'static str, bool)> {
    let numbers = pdb
        .keys()
        .chain(edb.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    numbers
        .into_iter()
        .filter_map(|number| {
            let picked = match (pdb.remove(&number), edb.remove(&number)) {
                (Some(p), Some(e)) if p.tracks.is_empty() && !e.tracks.is_empty() => {
                    (e, "eDB", true)
                }
                (Some(p), e) => (p, "PDB", e.is_some()),
                (None, Some(e)) => (e, "eDB", false),
                (None, None) => return None,
            };
            Some((number, picked))
        })
        .collect()
}

/// The PDB's history dates (t19 rows) by session number.
pub(super) fn history_dates_by_number(rows: &[PdbHistoryRow]) -> HashMap<u32, String> {
    rows.iter().fold(HashMap::new(), |mut acc, row| {
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
    })
}

/// A history entry whose track isn't in its database.
pub(super) fn unknown_history_track(track_id: u32) -> UsbTrack {
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
    }
}

pub(super) fn normalize_date_created(value: &str) -> Option<NaiveDate> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let ten = trimmed.chars().take(10).collect::<String>();
    NaiveDate::parse_from_str(&ten, "%Y-%m-%d").ok()
}

/// Dates the undated sessions from their tracks' added dates, carried
/// forward in session order. A session's track IDs are looked up in its own
/// database's dates (`pdb_dates` or `edb_dates`, by `source`).
pub(super) fn apply_history_dates_from_track_date_created(
    histories: &mut [UsbHistory],
    pdb_dates: &HashMap<u32, String>,
    edb_dates: &HashMap<u32, String>,
) {
    if histories.is_empty() || (pdb_dates.is_empty() && edb_dates.is_empty()) {
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

        let date_created_by_track_id = if history.source == "eDB" {
            edb_dates
        } else {
            pdb_dates
        };
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

/// A USB History read with its panel totals and status line, built when answering: the parse
/// cache holds the data only, so a cached read answers in the current language.
pub(super) fn with_usb_history_texts(data: &FetchUsbHistoriesData) -> FetchUsbHistoriesData {
    let mut data = data.clone();
    data.summary_text =
        tr::usb_history_summary(data.counts.imported_playlists, data.counts.imported_tracks)
            .render();
    data.status_text = tr::usb_history_loaded(data.items.len()).render();
    data.warning_text = warning_text(data.warning_count);
    for history in &mut data.items {
        // Which database has the session: how and when players write history
        // to each one isn't known.
        let databases = if history.in_both {
            tr::usb_history_source_both().render()
        } else {
            history.source.clone()
        };
        history.heading_text = tr::usb_history_heading(&history.name, databases).render();
    }
    data
}

pub(super) fn build_history_track_date_index(
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

impl BackendService {
    pub fn fetch_usb_histories(
        &self,
        req: FetchUsbHistoriesRequest,
    ) -> BackendResult<FetchUsbHistoriesData> {
        self.fetch_usb_histories_with_progress(req, |_, _, _| {})
    }

    pub fn fetch_usb_histories_with_progress<F>(
        &self,
        req: FetchUsbHistoriesRequest,
        mut on_progress: F,
    ) -> BackendResult<FetchUsbHistoriesData>
    where
        F: crate::progress::ProgressFn,
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

        let databases = UsbDatabases::present(&usb_root)?;
        let staged = databases
            .pdb()
            .map(|pdb_path| self.parsed_staged_pdb(pdb_path))
            .transpose()?;
        let pdb: Option<&ParsedPdb> = staged.as_deref().map(|staged| &*staged.parsed);
        push_usb_stage_timing("parse PDB", &mut stage_started);
        on_progress(
            30,
            100,
            &tr::usb_progress_reading_supplemental_databases().render(),
        );
        let mut warnings = Vec::<WarningEntry>::new();
        // One eDB connection for every read below: each open re-runs
        // SQLCipher key negotiation, which can dominate on a large eDB.
        let edb_conn = open_edb_from_usb_root(&usb_root, &mut warnings);
        if pdb.is_none() && edb_conn.is_none() {
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
        let edb = edb_conn
            .as_ref()
            .and_then(|conn| self.staged_edb(&usb_root, Some(conn), &mut warnings));
        let edb_track_by_id = edb
            .as_deref()
            .map(|library| library.resolved_tracks(&usb_root))
            .unwrap_or_default();
        let (edb_history_playlists, edb_history_entries) = match edb_conn.as_ref() {
            Some(conn) => crate::edb::read_edb_history(conn)?,
            None => (Vec::new(), Vec::new()),
        };
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
        push_usb_stage_timing_with_threshold(
            "read supplemental databases",
            &mut stage_started,
            slow_stage_threshold_ms(edb_track_by_id.len()),
        );

        on_progress(50, 100, &tr::usb_progress_building_track_index().render());
        let pdb_track_by_id = pdb
            .map(|parsed| build_usb_track_index(parsed, &usb_root))
            .unwrap_or_default();
        push_usb_stage_timing("build track index", &mut stage_started);

        on_progress(
            70,
            100,
            &tr::usb_progress_resolving_history_entries().render(),
        );
        // Each database's sessions resolve against that database's own
        // tracks: the two can number the same track differently.
        let (pdb_history_playlists, pdb_history_entries) = pdb
            .map(|parsed| select_history_rows(&parsed.history_playlists, &parsed.history_entries))
            .unwrap_or_default();
        let pdb_sessions = resolve_history_sessions(
            &pdb_history_playlists,
            &pdb_history_entries,
            &pdb_track_by_id,
        );
        let edb_sessions = resolve_history_sessions(
            &edb_history_playlists,
            &edb_history_entries,
            &edb_track_by_id,
        );
        let history_date_by_num = pdb
            .map(|parsed| history_dates_by_number(&parsed.history_rows))
            .unwrap_or_default();
        let mut items = merge_history_sessions(pdb_sessions, edb_sessions)
            .into_iter()
            .map(|(number, (session, source, in_both))| {
                let created_at = history_date_by_num
                    .get(&number)
                    .or_else(|| {
                        session
                            .name_number
                            .and_then(|n| history_date_by_num.get(&n))
                    })
                    .cloned();
                let (total_duration_ms, duration_unknown_count) = duration_totals(&session.tracks);
                UsbHistory {
                    id: format!("usb-h-{number}"),
                    name: session.name,
                    created_at,
                    can_export_tracklist: !session.tracks.is_empty(),
                    track_count: session.tracks.len(),
                    tracks: session.tracks,
                    total_duration_ms,
                    duration_unknown_count,
                    source: source.to_string(),
                    in_both,
                    // Built when answering: see `with_usb_history_texts`.
                    heading_text: String::new(),
                }
            })
            .collect::<Vec<_>>();
        push_usb_stage_timing("resolve history entries", &mut stage_started);

        export_log::apply_history_dates_from_export_log(&mut items, export_log.as_ref());
        let pdb_dates = pdb
            .map(|parsed| build_history_track_date_index(&parsed.tracks))
            .unwrap_or_default();
        let edb_dates = edb
            .as_deref()
            .map(|library| library.date_created.clone())
            .unwrap_or_default();
        apply_history_dates_from_track_date_created(&mut items, &pdb_dates, &edb_dates);
        // Newest first. Only now: the date carry-forward above needs ascending order.
        items.reverse();

        on_progress(
            90,
            100,
            &tr::usb_progress_finalizing_history_import().render(),
        );
        if let Some(parsed) = pdb {
            for message in &parsed.warnings {
                logging::debug("usb-import", message);
            }
        }
        logging::debug(
            "usb-import",
            format!(
                "history import: {} sessions from PDB ({} entries), {} from eDB ({} entries)",
                items.iter().filter(|h| h.source == "PDB").count(),
                pdb_history_entries.len(),
                items.iter().filter(|h| h.source == "eDB").count(),
                edb_history_entries.len(),
            ),
        );
        logging::debug(
            "usb-import",
            format!("USB root in use: {}", usb_root.display()),
        );
        // Per-track materialization is deferred to `fetch_usb_history_tracks`
        // (viewed page) and the add-to-playlist path -- see the note in
        // `fetch_usb_playlists_with_progress`.

        let pdb_rows = |source_table: u32| {
            pdb.map(|parsed| {
                parsed
                    .history_playlists
                    .iter()
                    .filter(|row| row.source_table == source_table)
                    .count()
            })
            .unwrap_or(0)
        };
        let pdb_entries = |source_table: u32| {
            pdb.map(|parsed| {
                parsed
                    .history_entries
                    .iter()
                    .filter(|row| row.source_table == source_table)
                    .count()
            })
            .unwrap_or(0)
        };
        let imported_tracks: usize = items.iter().map(|history| history.tracks.len()).sum();
        let data = Arc::new(FetchUsbHistoriesData {
            counts: UsbHistoryCounts {
                imported_playlists: items.len(),
                imported_tracks,
                pdb_t11_playlists: pdb_rows(11),
                pdb_t12_entries: pdb_entries(12),
                pdb_t17_playlists: pdb_rows(17),
                pdb_t18_entries: pdb_entries(18),
                edb_history_rows: edb_history_playlists.len(),
                edb_history_content_rows: edb_history_entries.len(),
            },
            items,
            // Built when answering: see `with_usb_history_texts`.
            summary_text: String::new(),
            status_text: String::new(),
            warning_count: logging::count_warnings(&warnings),
            warning_text: String::new(),
            warnings,
        });
        if let Ok(mut guard) = self.usb_histories_cache.lock() {
            *guard = Some((cache_key, Arc::clone(&data)));
        }
        Ok(with_usb_history_texts(&data))
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
}

pub(crate) fn sanitize_history_name(value: &str) -> String {
    sanitize_text(value)
}

/// Real rekordbox-recorded history sessions are always named `"HISTORY NNN"`.
/// Fresh/never-played exports ship `t17`/`t18` with a fixed block of blank
/// template/seed rows (no name, no track link) alongside this real data;
/// this predicate is the same rule `derive_history_sync_payload` in
/// `repair/history.rs` already uses to exclude them.
pub(crate) fn is_named_history_playlist(name: &str) -> bool {
    name.starts_with("HISTORY ")
}

pub(crate) fn history_entry_sort_key(raw: u32) -> u32 {
    let low = raw & 0xFFFF;
    if low != 0 { low } else { raw }
}

pub(crate) fn normalize_packed_id(raw: u32) -> u32 {
    let lo = raw & 0xFFFF;
    if lo != 0 { lo } else { raw }
}

pub(crate) fn parse_history_slot_id(value: &str) -> Option<u32> {
    let digits = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok().filter(|v| *v > 0)
}

pub(crate) fn parse_history_name_numeric_id(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    if !trimmed.to_ascii_uppercase().starts_with("HISTORY") {
        return None;
    }
    parse_history_slot_id(trimmed)
}

pub(crate) fn decode_history_track_id(
    raw_playlist_field: u32,
    raw_second_field: u32,
) -> Option<u32> {
    let track_from_playlist_hi = (raw_playlist_field >> 16) & 0xFFFF;
    if track_from_playlist_hi != 0 {
        return Some(track_from_playlist_hi);
    }

    let track_from_second_hi = (raw_second_field >> 8) & 0xFFFF;
    if track_from_second_hi != 0 {
        return Some(track_from_second_hi);
    }

    None
}

pub(crate) fn decode_history_playlist_id(
    raw_playlist_field: u32,
    raw_second_field: u32,
    known_history_ids: &HashSet<u32>,
) -> Option<u32> {
    let low = raw_playlist_field & 0xFFFF;
    let high = (raw_playlist_field >> 16) & 0xFFFF;
    let from_second = (raw_second_field >> 8) & 0xFFFF;

    let candidates = [low, high, from_second];
    candidates
        .into_iter()
        .find(|&candidate| candidate != 0 && known_history_ids.contains(&candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_history_slot_id_extracts_numeric_value() {
        assert_eq!(parse_history_slot_id("001"), Some(1));
        assert_eq!(parse_history_slot_id("history 042"), Some(42));
        assert_eq!(parse_history_slot_id("no-digits"), None);
    }

    #[test]
    fn parse_history_name_numeric_id_only_accepts_history_names() {
        assert_eq!(parse_history_name_numeric_id("HISTORY 007"), Some(7));
        assert_eq!(parse_history_name_numeric_id("history 7"), Some(7));
        assert_eq!(parse_history_name_numeric_id("Session 007"), None);
    }

    #[test]
    fn is_named_history_playlist_accepts_real_session_name() {
        assert!(is_named_history_playlist("HISTORY 001"));
    }

    #[test]
    fn is_named_history_playlist_rejects_blank_name() {
        assert!(!is_named_history_playlist(""));
    }

    #[test]
    fn is_named_history_playlist_rejects_non_history_name() {
        assert!(!is_named_history_playlist("Not History"));
    }

    #[test]
    fn decode_track_id_from_high_word() {
        // Track ID 42 in high word of playlist field
        let raw_playlist = 42u32 << 16;
        assert_eq!(decode_history_track_id(raw_playlist, 0), Some(42));
    }

    #[test]
    fn decode_track_id_from_second_field() {
        // High word of playlist is 0, falls back to second field
        let raw_second = 99u32 << 8;
        assert_eq!(decode_history_track_id(0, raw_second), Some(99));
    }

    #[test]
    fn decode_track_id_both_zero() {
        assert_eq!(decode_history_track_id(0, 0), None);
    }

    #[test]
    fn decode_track_id_prefers_playlist_high_word() {
        let raw_playlist = 10u32 << 16;
        let raw_second = 20u32 << 8;
        assert_eq!(decode_history_track_id(raw_playlist, raw_second), Some(10));
    }

    #[test]
    fn decode_playlist_id_matches_known() {
        let mut known = HashSet::new();
        known.insert(5u32);
        // Low word = 5
        assert_eq!(decode_history_playlist_id(5, 0, &known), Some(5));
    }

    #[test]
    fn decode_playlist_id_tries_high_word() {
        let mut known = HashSet::new();
        known.insert(7u32);
        let raw_playlist = 7u32 << 16;
        assert_eq!(decode_history_playlist_id(raw_playlist, 0, &known), Some(7));
    }

    #[test]
    fn decode_playlist_id_tries_second_field() {
        let mut known = HashSet::new();
        known.insert(12u32);
        let raw_second = 12u32 << 8;
        assert_eq!(decode_history_playlist_id(0, raw_second, &known), Some(12));
    }

    #[test]
    fn decode_playlist_id_no_match() {
        let mut known = HashSet::new();
        known.insert(99u32);
        assert_eq!(decode_history_playlist_id(1, 2, &known), None);
    }

    #[test]
    fn decode_playlist_id_zero_not_matched() {
        let mut known = HashSet::new();
        known.insert(0u32);
        // Zero candidates are skipped
        assert_eq!(decode_history_playlist_id(0, 0, &known), None);
    }

    #[test]
    fn parse_slot_id_extracts_digits() {
        assert_eq!(parse_history_slot_id("HISTORY 001"), Some(1));
        assert_eq!(parse_history_slot_id("slot-42-end"), Some(42));
    }

    #[test]
    fn parse_slot_id_empty() {
        assert_eq!(parse_history_slot_id("no digits here"), None);
    }

    #[test]
    fn parse_slot_id_zero_rejected() {
        assert_eq!(parse_history_slot_id("slot 000"), None);
    }

    #[test]
    fn parse_history_name_valid() {
        assert_eq!(parse_history_name_numeric_id("HISTORY 001"), Some(1));
        assert_eq!(parse_history_name_numeric_id("history 42"), Some(42));
    }

    #[test]
    fn parse_history_name_not_history_prefix() {
        assert_eq!(parse_history_name_numeric_id("playlist 1"), None);
    }

    #[test]
    fn history_sort_key_uses_low_word() {
        assert_eq!(history_entry_sort_key(0x0005_0003), 3);
    }

    #[test]
    fn history_sort_key_falls_back_to_full() {
        assert_eq!(history_entry_sort_key(0x0005_0000), 0x0005_0000);
    }

    #[test]
    fn normalize_packed_id_uses_low_word() {
        assert_eq!(normalize_packed_id(0x0001_0042), 0x0042);
    }

    #[test]
    fn normalize_packed_id_low_zero_uses_full() {
        assert_eq!(normalize_packed_id(0x0001_0000), 0x0001_0000);
    }
}
