//! USB diagnostics and parity report functions.

use std::collections::{HashMap, HashSet};

use crate::edb::{
    ExportDbPlaylist, open_edb_from_usb_root, try_read_playlists_with_metadata_from_edb,
    try_read_playlists_with_metadata_from_edb_db_only,
};
use crate::error::{BackendError, BackendResult};
use crate::models::{
    DiagCheck, DiagCountsSummary, DiagSection, DiagStatus, DiagSummaryRow, PlayerCounterSnapshot,
    PlayerPageSignal, PlayerTableSignal, PlaylistDiagEntry, RunUsbDiagnosticsData,
    RunUsbDiagnosticsRequest, RunUsbParityReportData, RunUsbParityReportRequest,
    UsbParityPlaylistDetail, UsbTrack, WarningEntry,
};
use crate::pdb_reader::parse_pdb;
use crate::pdb_reader::parse_pdb_with_diagnostics;
use crate::utils::{
    collect_chain_lenient as collect_chain, page_offset, read_u8_at, read_u16_le_at,
    read_u32_le_at, table_ptr_fields,
};

use super::BackendService;
use super::export_helpers::{load_table_columns, table_exists};
use super::usb_utils::{
    canonicalize_playlist_name, collect_contents_audio_files, repair_utf8_mojibake,
    resolve_usb_root,
};
use super::usb_vendor_compat::{vendor_edb_path, vendor_pdb_path};

fn first_data_page_signal(
    bytes: &[u8],
    page_size: usize,
    chain: &[u32],
) -> Option<PlayerPageSignal> {
    let page = *chain.get(1)?;
    let off = page_offset(page, page_size)?;
    let rowpf_off = off + page_size - 4;
    let tranrf_off = off + page_size - 2;
    Some(PlayerPageSignal {
        page,
        seq: read_u32_le_at(bytes, off + 0x10).unwrap_or(0),
        nrs: read_u8_at(bytes, off + 0x18).unwrap_or(0),
        u3: read_u8_at(bytes, off + 0x19).unwrap_or(0),
        num_rl: read_u16_le_at(bytes, off + 0x22).unwrap_or(0),
        rowpf0: read_u16_le_at(bytes, rowpf_off).unwrap_or(0),
        tranrf0: read_u16_le_at(bytes, tranrf_off).unwrap_or(0),
    })
}

fn table_signal(bytes: &[u8], page_size: usize, table_type: u32) -> Option<PlayerTableSignal> {
    let (ec, first, last) = table_ptr_fields(bytes, table_type)?;
    let chain = collect_chain(bytes, page_size, first, last);
    let data_page = first_data_page_signal(bytes, page_size, &chain);
    Some(PlayerTableSignal {
        table_type,
        ec,
        first,
        last,
        chain_len: chain.len(),
        data_page,
    })
}

fn is_known_pdb_header_compatibility_value(value: u32) -> bool {
    matches!(value, 1 | 5)
}

fn read_pdb_header_compatibility_value(path: &std::path::Path) -> Option<u32> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 0x14];
    file.read_exact(&mut header).ok()?;
    read_u32_le_at(&header, 0x10)
}

fn previous_pdb_header_compatibility_value(pdb_path: &std::path::Path) -> Option<(u32, String)> {
    let previous_dir = pdb_path.parent()?.join("backups");
    let mut candidates = Vec::<(String, u32)>::new();
    for entry in std::fs::read_dir(previous_dir).ok()? {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.starts_with("export_")
            || path.extension().and_then(|ext| ext.to_str()) != Some("pdb")
        {
            continue;
        }
        let Some(value) = read_pdb_header_compatibility_value(&path) else {
            continue;
        };
        if is_known_pdb_header_compatibility_value(value) {
            candidates.push((file_name.to_string(), value));
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.pop().map(|(name, value)| (value, name))
}

fn compute_player_counter_snapshot(
    usb_root: &std::path::Path,
    parsed: &crate::pdb_reader::ParsedPdb,
) -> Option<PlayerCounterSnapshot> {
    let pdb_path = vendor_pdb_path(usb_root);
    let bytes = std::fs::read(&pdb_path).ok()?;
    let page_size = read_u32_le_at(&bytes, 4)? as usize;
    if page_size == 0 {
        return None;
    }

    let t00_tracks = parsed.tracks.len();
    let t08_entries = parsed.playlist_entries.len();
    let playlist_count_candidate = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.row_is_folder)
        .count();

    let t11 = table_signal(&bytes, page_size, 11)?;
    let t12 = table_signal(&bytes, page_size, 12)?;
    let t17 = table_signal(&bytes, page_size, 17)?;
    let t18 = table_signal(&bytes, page_size, 18)?;
    let t19 = table_signal(&bytes, page_size, 19)?;

    let t19_shape_suggests_runtime = t19.chain_len >= 3
        || t19
            .data_page
            .as_ref()
            .map(|p| p.nrs > 1 || p.num_rl > 0 || p.rowpf0 != 0x0001)
            .unwrap_or(false);
    let song_count_candidate = if t19_shape_suggests_runtime {
        t00_tracks
    } else {
        0
    };

    let baseline_init_like = t11.ec == 24
        && t11.first == 23
        && t11.last == 23
        && t12.ec == 26
        && t12.first == 25
        && t12.last == 25
        && t17.ec == 44
        && t17.first == 35
        && t17.last == 36
        && t18.ec == 45
        && t18.first == 37
        && t18.last == 38
        && t19.ec == 41
        && t19.first == 39
        && t19.last == 40;

    let shape_mode = if t19_shape_suggests_runtime {
        "runtime".to_string()
    } else if baseline_init_like {
        "baseline-init".to_string()
    } else {
        "unknown".to_string()
    };

    let confidence = if t19_shape_suggests_runtime {
        "high".to_string()
    } else if t00_tracks > 0 {
        "medium".to_string()
    } else {
        "low".to_string()
    };

    Some(PlayerCounterSnapshot {
        playlist_count_candidate,
        song_count_candidate,
        confidence,
        shape_mode,
        baseline_init_like,
        t00_tracks,
        t08_entries,
        t11,
        t12,
        t17,
        t18,
        t19,
    })
}

fn diagnostics_warning_entry(message: String) -> WarningEntry {
    let lower = message.to_lowercase();
    let (level, code) = if message.starts_with("slow-media suspected:") {
        ("warn", "usb.diagnostics.slow-media")
    } else if message.starts_with("PDB header compatibility field") {
        ("warn", "usb.diagnostics.pdb-header-compatibility")
    } else if message.starts_with("history-only track:") {
        ("info", "usb.diagnostics.history-only")
    } else if message.starts_with("unindexed audio file:") {
        ("info", "usb.diagnostics.unindexed-audio")
    } else if message.starts_with("missing-audio reference:") {
        ("warn", "usb.diagnostics.missing-audio")
    } else if message.starts_with("PDB and eDB menus disagree") {
        ("warn", "usb.diagnostics.player-menu-divergence")
    } else if message.contains("stale b-tree index") {
        ("error", "usb.diagnostics.pdb-stale-sentinel-btree")
    } else if message.contains("playlist_tree page") && message.contains("wrong shape") {
        ("warn", "usb.diagnostics.pdb-wrong-playlist-tree-shape")
    } else if message.contains("tombstoned row") && message.contains("non-zero id") {
        ("warn", "usb.diagnostics.pdb-tombstoned-ids")
    } else if message.contains("track page") && message.contains("wrong u5/num_rl shape") {
        ("warn", "usb.diagnostics.pdb-wrong-page-shape")
    } else if message.contains("wrong u5/num_rl shape") {
        ("error", "usb.diagnostics.pdb-wrong-page-shape")
    } else if message.contains("ACTV flag in multi-page chain") {
        ("warn", "usb.diagnostics.pdb-t00-multipage-active")
    } else if message.contains("empty_candidate pointing to another table") {
        ("error", "usb.diagnostics.pdb-ec-data-page-conflict")
    } else if lower.contains("malformed") || lower.contains("corrupt") {
        ("warn", "usb.diagnostics.malformed")
    } else if lower.contains("failed") || lower.contains("error") {
        ("error", "usb.diagnostics.error")
    } else {
        ("info", "usb.diagnostics.info")
    };
    WarningEntry {
        level: level.to_string(),
        code: code.to_string(),
        message,
        source: "usb-diagnostics".to_string(),
    }
}

pub(crate) fn collect_edb_indexed_paths(
    usb_root: &std::path::Path,
    warnings: &mut Vec<String>,
) -> HashSet<String> {
    let Some(conn) = open_edb_from_usb_root(usb_root, warnings) else {
        return HashSet::new();
    };
    if !table_exists(&conn, "content") {
        return HashSet::new();
    }
    let Ok(content_columns) = load_table_columns(&conn, "content") else {
        warnings.push("eDB content column scan failed".to_string());
        return HashSet::new();
    };
    let has_column = |name: &str| content_columns.iter().any(|column| column == name);
    let (query_sql, mode) = if has_column("path") {
        (
            r#"
            SELECT DISTINCT COALESCE(c.path, '')
            FROM content c
            "#,
            "path",
        )
    } else if has_column("FolderPath") && has_column("FileNameL") {
        (
            r#"
            SELECT DISTINCT
              COALESCE(c.FolderPath, ''),
              COALESCE(c.FileNameL, '')
            FROM content c
            "#,
            "folder_file",
        )
    } else {
        warnings.push(
            "eDB content schema missing path columns (expected path or FolderPath/FileNameL)"
                .to_string(),
        );
        return HashSet::new();
    };
    let mut stmt = match conn.prepare(query_sql) {
        Ok(stmt) => stmt,
        Err(err) => {
            warnings.push(format!("eDB playlist path query prepare failed: {err}"));
            return HashSet::new();
        }
    };
    let mut out = HashSet::<String>::new();
    let rows = match stmt.query_map([], |row| match mode {
        "path" => Ok((row.get::<_, String>(0).unwrap_or_default(), String::new())),
        _ => Ok((
            row.get::<_, String>(0).unwrap_or_default(),
            row.get::<_, String>(1).unwrap_or_default(),
        )),
    }) {
        Ok(rows) => rows,
        Err(err) => {
            warnings.push(format!("eDB playlist path query failed: {err}"));
            return HashSet::new();
        }
    };
    for item in rows {
        let Ok((folder, file)) = item else { continue };
        let full = if mode == "path" {
            folder.trim().to_string()
        } else {
            let folder = folder.trim();
            let file = file.trim();
            if folder.is_empty() || file.is_empty() {
                continue;
            }
            format!("{}/{}", folder.trim_end_matches('/'), file)
        };
        if full.is_empty() {
            continue;
        }
        // Keep this stage fast on slow USB media: normalize by logical
        // /Contents/... path shape without per-track filesystem probes.
        // Existence is already derived from the one-pass Contents scan.
        let normalized = normalize_path_for_contents_match(&full);
        if !normalized.is_empty() {
            out.insert(normalized);
        }
    }
    out
}

fn collect_edb_content_paths_exact(
    usb_root: &std::path::Path,
    warnings: &mut Vec<String>,
) -> HashSet<String> {
    let Some(conn) = open_edb_from_usb_root(usb_root, warnings) else {
        return HashSet::new();
    };
    if !table_exists(&conn, "content") {
        return HashSet::new();
    }
    let Ok(mut stmt) = conn.prepare(
        "SELECT COALESCE(path, '') FROM content WHERE path IS NOT NULL AND trim(path) <> ''",
    ) else {
        return HashSet::new();
    };
    stmt.query_map([], |row| row.get::<_, String>(0))
        .ok()
        .map(|rows| {
            rows.flatten()
                .map(|p| normalize_pdb_path_for_edb_lookup(&p))
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub(crate) struct StrictRawCoverageParity {
    pub missing_count: usize,
    pub extra_count: usize,
    pub status: DiagStatus,
    pub detail: String,
}

pub(crate) fn evaluate_strict_raw_coverage_parity(
    missing_count: usize,
    extra_count: usize,
    total_indexed: usize,
) -> StrictRawCoverageParity {
    let warn_threshold = std::cmp::max(5, total_indexed / 20);
    let missing_status = if missing_count == 0 {
        DiagStatus::Pass
    } else if missing_count <= warn_threshold {
        DiagStatus::Warn
    } else {
        DiagStatus::Fail
    };
    let extra_status = if extra_count == 0 {
        DiagStatus::Pass
    } else if extra_count <= warn_threshold {
        DiagStatus::Warn
    } else {
        DiagStatus::Fail
    };
    let status = DiagStatus::worst(&missing_status, &extra_status);
    let detail = match (missing_count, extra_count) {
        (0, 0) => format!("all {total_indexed} indexed audio files present, no unindexed files"),
        (m, 0) => format!("{m} of {total_indexed} indexed audio file(s) missing from USB"),
        (0, e) => format!(
            "{e} audio file(s) on USB not referenced in any playlist (total indexed={total_indexed})"
        ),
        (m, e) => {
            format!("{m} of {total_indexed} indexed file(s) missing; {e} unindexed file(s) on USB")
        }
    };
    StrictRawCoverageParity {
        missing_count,
        extra_count,
        status,
        detail,
    }
}

pub(crate) fn collect_strict_indexed_paths(
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: &HashMap<String, ExportDbPlaylist>,
    usb_root: &std::path::Path,
    warnings: &mut Vec<String>,
) -> HashSet<String> {
    // All PDB tracks regardless of playlist/history membership
    let pdb_track_paths = parsed
        .tracks
        .iter()
        .map(|track| normalize_pdb_path_for_edb_lookup(&track.track_file_path))
        .filter(|path| !path.is_empty());
    // All eDB content paths without alias stripping — exact filenames needed for disk check
    let edb_all_paths = collect_edb_content_paths_exact(usb_root, warnings).into_iter();
    // eDB playlist paths kept for any track whose path differs from the content table
    let edb_playlist_paths = edb_playlists
        .values()
        .flat_map(|playlist| playlist.tracks.iter())
        .map(|track| normalize_pdb_path_for_edb_lookup(track.identity_path()))
        .filter(|path| !path.is_empty());
    pdb_track_paths
        .chain(edb_all_paths)
        .chain(edb_playlist_paths)
        .collect()
}

const REFERENCE_ONLY_EDB_FIELDS: &[&str] = &[
    "artist_id_lyricist",
    "artist_id_originalArtist",
    "artist_id_remixer",
    "artist_id_composer",
    "label_id",
    "rating",
    "color_id",
];

#[derive(Debug, Default, Clone)]
pub(crate) struct ReferenceOnlyEdbFieldUsage {
    playlist_linked_tracks: usize,
    populated_fields: Vec<String>,
}

fn scan_reference_only_edb_field_usage(
    usb_root: &std::path::Path,
    warnings: &mut Vec<String>,
) -> ReferenceOnlyEdbFieldUsage {
    let Some(conn) = open_edb_from_usb_root(usb_root, warnings) else {
        return ReferenceOnlyEdbFieldUsage::default();
    };
    if !table_exists(&conn, "content") || !table_exists(&conn, "playlist_content") {
        return ReferenceOnlyEdbFieldUsage::default();
    }
    let Ok(columns) = load_table_columns(&conn, "content") else {
        return ReferenceOnlyEdbFieldUsage::default();
    };

    let mut populated_fields = Vec::<String>::new();
    let mut max_playlist_linked_tracks = 0usize;
    for field in REFERENCE_ONLY_EDB_FIELDS {
        if !columns.iter().any(|column| column == field) {
            continue;
        }
        let sql = format!(
            "SELECT COUNT(DISTINCT pc.content_id)
             FROM playlist_content pc
             JOIN content c ON c.content_id = pc.content_id
             WHERE c.{field} IS NOT NULL
               AND trim(CAST(c.{field} AS TEXT)) <> ''
               AND CAST(c.{field} AS TEXT) <> '0'"
        );
        let count = conn
            .query_row(&sql, [], |row| row.get::<_, i64>(0))
            .ok()
            .unwrap_or(0)
            .max(0) as usize;
        if count > 0 {
            populated_fields.push((*field).to_string());
            max_playlist_linked_tracks = max_playlist_linked_tracks.max(count);
        }
    }
    populated_fields.sort();
    populated_fields.dedup();
    ReferenceOnlyEdbFieldUsage {
        playlist_linked_tracks: max_playlist_linked_tracks,
        populated_fields,
    }
}

impl BackendService {
    pub fn run_usb_diagnostics(
        &self,
        req: RunUsbDiagnosticsRequest,
    ) -> BackendResult<RunUsbDiagnosticsData> {
        self.run_usb_diagnostics_with_progress(req, |_, _, _| {})
    }

    pub fn run_usb_diagnostics_with_progress<F>(
        &self,
        req: RunUsbDiagnosticsRequest,
        mut on_progress: F,
    ) -> BackendResult<RunUsbDiagnosticsData>
    where
        F: FnMut(usize, usize, &str),
    {
        const SLOW_USB_STAGE_MS: u128 = 3_000;
        let start = std::time::Instant::now();
        let mut stage_started = std::time::Instant::now();
        let mut note_stage = |name: &str, raw_warnings: &mut Vec<String>| {
            let elapsed = stage_started.elapsed().as_millis();
            raw_warnings.push(format!("stage timing: {name}: {elapsed}ms"));
            if elapsed >= SLOW_USB_STAGE_MS {
                raw_warnings.push(format!(
                    "slow-media suspected: stage '{name}' took {elapsed}ms"
                ));
            }
            stage_started = std::time::Instant::now();
        };
        on_progress(2, 100, "USB: Resolving root");
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut raw_warnings = Vec::<String>::new();
        raw_warnings.push(format!("USB root: {}", usb_root.display()));
        note_stage("resolve usb root", &mut raw_warnings);

        let pdb_path = vendor_pdb_path(&usb_root);

        // --- 1. PDB Integrity ---
        on_progress(10, 100, "USB: Checking PDB integrity");
        let (pdb_integrity, parsed_opt) = diagnose_pdb_integrity(&pdb_path, &mut raw_warnings);
        note_stage("pdb integrity", &mut raw_warnings);

        // --- 2. DB Access ---
        on_progress(30, 100, "USB: Checking database access");
        let mut edb_access = diagnose_edb_access(&usb_root, &mut raw_warnings);
        let edb_playlists =
            try_read_playlists_with_metadata_from_edb_db_only(&usb_root, &mut raw_warnings);
        let edb_playlist_tracks = edb_playlists.as_ref().map(|m| {
            m.iter()
                .map(|(name, playlist)| (name.clone(), playlist.tracks.clone()))
                .collect::<HashMap<_, _>>()
        });

        // eDB history counts
        {
            let mut edb_warnings = Vec::new();
            let edb_conn = open_edb_from_usb_root(&usb_root, &mut edb_warnings);
            let (edb_h, edb_hc) = if let Some(conn) = edb_conn.as_ref() {
                let h = if table_exists(conn, "history") {
                    conn.query_row("SELECT COUNT(*) FROM history", [], |r| r.get::<_, i64>(0))
                        .unwrap_or(0)
                        .max(0) as usize
                } else {
                    0
                };
                let hc = if table_exists(conn, "history_content") {
                    conn.query_row("SELECT COUNT(*) FROM history_content", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap_or(0)
                    .max(0) as usize
                } else {
                    0
                };
                (h, hc)
            } else {
                (0, 0)
            };
            if edb_h > 0 || edb_hc > 0 {
                edb_access.checks.push(DiagCheck {
                    label: "eDB history".to_string(),
                    status: DiagStatus::Pass,
                    detail: format!("{edb_h} playlists, {edb_hc} entries"),
                    link: None,
                });
            } else {
                edb_access.checks.push(DiagCheck {
                    label: "eDB history".to_string(),
                    status: DiagStatus::Pass,
                    detail: "empty".to_string(),
                    link: None,
                });
            }
        }

        note_stage("db access", &mut raw_warnings);

        // --- 3. Contents Integrity ---
        on_progress(50, 100, "USB: Checking contents integrity (DB-only)");
        let pdb_indexed_paths = parsed_opt
            .as_ref()
            .map(|p| {
                let parsed = &p.0;
                parsed
                    .tracks
                    .iter()
                    .map(|t| normalize_path_for_contents_match(&t.track_file_path))
                    .filter(|p| !p.is_empty())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let edb_indexed_paths = collect_edb_indexed_paths(&usb_root, &mut raw_warnings);

        // Identify tracks that exist in PDB history playlists but not in any regular playlist.
        // These are written by players during playback and are expected to be absent from eDB.
        let history_only_pdb_paths = if let Some((parsed, _)) = parsed_opt.as_ref() {
            let regular_ids: HashSet<u32> =
                parsed.playlist_entries.iter().map(|e| e.track_id).collect();
            let history_only_ids: HashSet<u32> = parsed
                .history_entries
                .iter()
                .filter_map(|e| e.track_id)
                .filter(|id| !regular_ids.contains(id))
                .collect();
            let hp_name: HashMap<u32, &str> = parsed
                .history_playlists
                .iter()
                .map(|hp| (hp.id, hp.name.as_str()))
                .collect();
            let mut out = HashSet::new();
            for track in &parsed.tracks {
                if !history_only_ids.contains(&track.id) {
                    continue;
                }
                let norm = normalize_path_for_contents_match(&track.track_file_path);
                if norm.is_empty() {
                    continue;
                }
                let pl_names: std::collections::BTreeSet<&str> = parsed
                    .history_entries
                    .iter()
                    .filter(|e| e.track_id == Some(track.id))
                    .filter_map(|e| hp_name.get(&e.playlist_id).copied())
                    .collect();
                let pl_str = if pl_names.is_empty() {
                    "unknown history playlist".to_string()
                } else {
                    pl_names.into_iter().collect::<Vec<_>>().join(", ")
                };
                raw_warnings.push(format!("history-only track: {norm} ({pl_str})"));
                out.insert(norm);
            }
            out
        } else {
            HashSet::new()
        };

        let true_pdb_only_count = pdb_indexed_paths
            .difference(&edb_indexed_paths)
            .filter(|p| !history_only_pdb_paths.contains(*p))
            .count();
        let history_only_count = pdb_indexed_paths
            .difference(&edb_indexed_paths)
            .filter(|p| history_only_pdb_paths.contains(*p))
            .count();
        let edb_only_count = edb_indexed_paths.difference(&pdb_indexed_paths).count();
        let contents_integrity = diagnose_contents_integrity_db_only(
            pdb_indexed_paths.len(),
            edb_indexed_paths.len(),
            true_pdb_only_count,
            edb_only_count,
            history_only_count,
        );
        note_stage("contents integrity", &mut raw_warnings);

        // --- 4. Analysis Integrity ---
        on_progress(70, 100, "USB: Checking analysis references");
        let analysis_integrity = diagnose_analysis_integrity(
            parsed_opt.as_ref().map(|p| &p.0),
            edb_playlist_tracks.as_ref(),
        );
        note_stage("analysis integrity", &mut raw_warnings);

        // --- 5. Playlist Resolution ---
        on_progress(85, 100, "USB: Checking playlist resolution");
        let (playlist_resolution, playlist_details) =
            diagnose_playlist_resolution_with_edb_internal(
                parsed_opt.as_ref().map(|p| &p.0),
                edb_playlist_tracks.as_ref(),
                |done, total, message| {
                    let pct = if total == 0 {
                        98
                    } else {
                        85 + ((done * 13) / total.max(1)).min(13)
                    };
                    on_progress(pct, 100, message);
                },
            );
        note_stage("playlist resolution", &mut raw_warnings);

        {
            let mut menu_warnings = Vec::<String>::new();
            if let Ok((_current, _available, divergence)) =
                crate::service::repair::load_usb_player_menu_config_public(
                    &usb_root,
                    &mut menu_warnings,
                )
                && !divergence.is_empty()
            {
                raw_warnings.push(format!(
                    "PDB and eDB menus disagree ({} active menu items missing from PDB)",
                    divergence.in_edb_visible_only.len()
                ));
            }
            for warning in menu_warnings {
                raw_warnings.push(warning);
            }
            note_stage("player menu divergence", &mut raw_warnings);
        }

        let overall_status = DiagStatus::worst_of(&[
            &pdb_integrity.status,
            &edb_access.status,
            &contents_integrity.status,
            &analysis_integrity.status,
            &playlist_resolution.status,
        ]);

        Ok(RunUsbDiagnosticsData {
            overall_status,
            pdb_integrity,
            edb_access,
            contents_integrity,
            analysis_integrity,
            playlist_resolution,
            playlist_details,
            cdj_counter_snapshot: parsed_opt
                .as_ref()
                .and_then(|(parsed, _)| compute_player_counter_snapshot(&usb_root, parsed)),
            warnings: raw_warnings
                .iter()
                .cloned()
                .map(diagnostics_warning_entry)
                .collect(),
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    pub fn run_usb_parity_report(
        &self,
        req: RunUsbParityReportRequest,
    ) -> BackendResult<RunUsbParityReportData> {
        self.run_usb_parity_report_with_progress(req, |_, _, _| {})
    }

    pub fn run_usb_parity_report_with_progress<F>(
        &self,
        req: RunUsbParityReportRequest,
        mut on_progress: F,
    ) -> BackendResult<RunUsbParityReportData>
    where
        F: FnMut(usize, usize, &str),
    {
        const SLOW_USB_STAGE_MS: u128 = 3_000;
        let start = std::time::Instant::now();
        let mut stage_started = std::time::Instant::now();
        let mut note_stage = |name: &str, raw_warnings: &mut Vec<String>| {
            let elapsed = stage_started.elapsed().as_millis();
            raw_warnings.push(format!("stage timing: {name}: {elapsed}ms"));
            if elapsed >= SLOW_USB_STAGE_MS {
                raw_warnings.push(format!(
                    "slow-media suspected: stage '{name}' took {elapsed}ms"
                ));
            }
            stage_started = std::time::Instant::now();
        };
        on_progress(2, 100, "USB: Resolving root");
        let usb_root = resolve_usb_root(req.usb_root.as_deref())?;
        let mut raw_warnings = vec![format!("USB root: {}", usb_root.display())];
        note_stage("resolve usb root", &mut raw_warnings);

        on_progress(20, 100, "USB: Parsing PDB");
        let pdb_path = vendor_pdb_path(&usb_root);
        let parsed = parse_pdb(&pdb_path)?;
        raw_warnings.extend(parsed.warnings.clone());
        note_stage("parse PDB", &mut raw_warnings);

        on_progress(45, 100, "USB: Reading eDB");
        let edb_playlists = try_read_playlists_with_metadata_from_edb(&usb_root, &mut raw_warnings)
            .ok_or_else(|| {
                BackendError::Internal(
                    "parity report requires readable eDB playlist data".to_string(),
                )
            })?;
        let reference_only_edb_fields =
            scan_reference_only_edb_field_usage(&usb_root, &mut raw_warnings);
        note_stage("read eDB", &mut raw_warnings);

        on_progress(58, 100, "USB: Checking indexed audio file presence");
        let actual_files: HashSet<String> = collect_contents_audio_files(&usb_root)
            .into_iter()
            .collect();
        let indexed_paths =
            collect_strict_indexed_paths(&parsed, &edb_playlists, &usb_root, &mut raw_warnings);
        let missing_count = indexed_paths
            .iter()
            .filter(|p| !actual_files.contains(*p))
            .count();
        let mut extra_paths: Vec<String> = actual_files
            .iter()
            .filter(|p| !indexed_paths.contains(*p))
            .cloned()
            .collect();
        extra_paths.sort();
        let extra_count = extra_paths.len();
        for path in &extra_paths {
            raw_warnings.push(format!("unindexed audio file: {path}"));
        }
        let strict_raw_coverage =
            evaluate_strict_raw_coverage_parity(missing_count, extra_count, indexed_paths.len());
        note_stage("strict raw coverage check", &mut raw_warnings);

        on_progress(70, 100, "USB: Comparing playlist sources");
        let (checks, summary_rows, playlist_details, overall_status) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists,
            &reference_only_edb_fields,
            Some(strict_raw_coverage),
        );
        note_stage("compare playlist sources", &mut raw_warnings);

        on_progress(95, 100, "USB: Finalizing parity report");
        note_stage("finalize", &mut raw_warnings);
        Ok(RunUsbParityReportData {
            overall_status,
            checks,
            summary_rows,
            playlist_details,
            warnings: raw_warnings
                .iter()
                .cloned()
                .map(diagnostics_warning_entry)
                .collect(),
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

pub(crate) fn diagnose_pdb_integrity(
    pdb_path: &std::path::Path,
    raw_warnings: &mut Vec<String>,
) -> (
    DiagSection,
    Option<(
        crate::pdb_reader::ParsedPdb,
        crate::pdb_reader::PdbDiagnostics,
    )>,
) {
    let mut checks = Vec::new();

    if !pdb_path.exists() {
        checks.push(DiagCheck {
            label: "PDB exists".to_string(),
            status: DiagStatus::Fail,
            detail: format!("File not found: {}", pdb_path.display()),
            link: None,
        });
        return (
            DiagSection {
                title: "PDB Integrity".to_string(),
                status: DiagStatus::Fail,
                checks,
                counts: None,
            },
            None,
        );
    }
    checks.push(DiagCheck {
        label: "PDB exists".to_string(),
        status: DiagStatus::Pass,
        detail: "Found".to_string(),
        link: None,
    });

    let (parsed, diag) = match parse_pdb_with_diagnostics(pdb_path) {
        Ok(result) => result,
        Err(err) => {
            checks.push(DiagCheck {
                label: "PDB parseable".to_string(),
                status: DiagStatus::Fail,
                detail: format!("Parse error: {err}"),
                link: None,
            });
            return (
                DiagSection {
                    title: "PDB Integrity".to_string(),
                    status: DiagStatus::Fail,
                    checks,
                    counts: None,
                },
                None,
            );
        }
    };

    checks.push(DiagCheck {
        label: "PDB parseable".to_string(),
        status: DiagStatus::Pass,
        detail: format!(
            "{} tracks, {} artists, {} albums, {} genres, {} labels, {} keys, {} artworks",
            parsed.tracks.len(),
            parsed.artists.len(),
            parsed.albums.len(),
            parsed.genres.len(),
            parsed.labels.len(),
            parsed.keys.len(),
            parsed.artworks.len(),
        ),
        link: None,
    });

    if let Some(header_compat) = read_pdb_header_compatibility_value(pdb_path) {
        let previous_compat = previous_pdb_header_compatibility_value(pdb_path);
        let drift_target = previous_compat
            .as_ref()
            .and_then(|(value, name)| (header_compat != *value).then_some((*value, name)));
        let invalid_current = !is_known_pdb_header_compatibility_value(header_compat);
        let header_compat_status = if drift_target.is_some() || invalid_current {
            DiagStatus::Warn
        } else {
            DiagStatus::Pass
        };
        let detail = if let Some((target, name)) = drift_target {
            format!(
                "value {header_compat} at bytes 0x10..0x14; previous local PDB snapshot {name} has {target}"
            )
        } else if invalid_current {
            format!("value {header_compat} at bytes 0x10..0x14; expected a known-compatible value")
        } else {
            format!("value {header_compat} at bytes 0x10..0x14")
        };
        if matches!(header_compat_status, DiagStatus::Warn) {
            raw_warnings.push(format!(
                "PDB header compatibility field is {header_compat}; run repair_pdb_header_compatibility_field to restore a compatible value."
            ));
        }
        checks.push(DiagCheck {
            label: "PDB header compatibility".to_string(),
            status: header_compat_status,
            detail,
            link: None,
        });
    }

    {
        let bad_u5 = super::repair::detect_pdb_sentinel_u5_on_data_pages(pdb_path);
        let bad_flags = super::repair::detect_pdb_wrong_page_flags(pdb_path);
        let bad_tranrf = super::repair::detect_pdb_zero_tranrf_all_tables(pdb_path);
        if !bad_u5.is_empty() {
            raw_warnings.push(format!(
                "{} PDB data page(s) with sentinel u5=0x1FFF (may be rejected by some DJ software and player firmware); run repair_pdb_sentinel_u5_on_data_pages to fix.",
                bad_u5.len()
            ));
        }
        if !bad_flags.is_empty() {
            raw_warnings.push(format!(
                "{} PDB data page(s) with wrong page_flags byte (may be rejected by some DJ software and player firmware); run repair_pdb_wrong_page_flags to fix.",
                bad_flags.len()
            ));
        }
        if !bad_tranrf.is_empty() {
            raw_warnings.push(format!(
                "{} data page(s) with tranrf=0 on last-row group and active rows (player may reject database); run repair_pdb_zero_tranrf_on_track_pages to fix.",
                bad_tranrf.len()
            ));
        }
        let any_bad = !bad_u5.is_empty() || !bad_flags.is_empty() || !bad_tranrf.is_empty();
        let mut parts = Vec::new();
        if !bad_u5.is_empty() {
            parts.push(format!("{} page(s) with sentinel u5=0x1FFF", bad_u5.len()));
        }
        if !bad_flags.is_empty() {
            parts.push(format!("{} page(s) with wrong page_flags", bad_flags.len()));
        }
        if !bad_tranrf.is_empty() {
            parts.push(format!(
                "{} data page(s) with tranrf=0 on last-row group",
                bad_tranrf.len()
            ));
        }
        let detail = if parts.is_empty() {
            "u5, page_flags, and tranrf are valid on all data pages".to_string()
        } else {
            parts.join("; ") + " — may affect desktop DJ software compatibility; repair available"
        };
        checks.push(DiagCheck {
            label: "PDB page headers".to_string(),
            status: if any_bad {
                DiagStatus::Warn
            } else {
                DiagStatus::Pass
            },
            detail,
            link: None,
        });
    }

    {
        let stale_btree = super::repair::detect_pdb_stale_sentinel_btree(pdb_path);
        let wrong_pl_tree = super::repair::detect_pdb_wrong_playlist_tree_shape(pdb_path);
        let tombstoned = super::repair::detect_pdb_tombstoned_playlist_tree_ids(pdb_path);
        let wrong_track_u5 = super::repair::detect_pdb_wrong_track_u5(pdb_path);
        let wrong_history = super::repair::detect_pdb_wrong_history_page_shape(pdb_path);
        let t00_multipage_active = super::repair::detect_pdb_t00_multipage_active_pages(pdb_path);
        let ec_conflicts = super::repair::detect_pdb_ec_data_page_conflicts(pdb_path);

        // Hard failures: conditions that cause player or DJ software rejection.
        let mut fail_parts = Vec::new();
        // Soft warnings: player-tolerant conditions that may appear in existing
        // PDBs after multiple desktop-library re-exports (non-terminal ACTV pages,
        // tombstone ID collisions from re-export). Still offered as optional repairs.
        let mut warn_parts = Vec::new();

        if !stale_btree.is_empty() {
            raw_warnings.push(format!(
                "{} sentinel page(s) with stale b-tree index (may be rejected by some DJ software); run repair_pdb_stale_sentinel_btree to fix.",
                stale_btree.len()
            ));
            warn_parts.push(format!("{} sentinel(s) stale b-tree", stale_btree.len()));
        }
        if !wrong_pl_tree.is_empty() {
            raw_warnings.push(format!(
                "{} playlist_tree page(s) with wrong shape (u5/num_rl); run repair_pdb_wrong_playlist_tree_shape to fix.",
                wrong_pl_tree.len()
            ));
            warn_parts.push(format!(
                "{} playlist_tree page(s) wrong shape",
                wrong_pl_tree.len()
            ));
        }
        if !tombstoned.is_empty() {
            raw_warnings.push(format!(
                "{} tombstoned row(s) with non-zero id; run repair_pdb_tombstoned_playlist_tree_ids to fix.",
                tombstoned.len()
            ));
            warn_parts.push(format!(
                "{} tombstoned row(s) non-zero id",
                tombstoned.len()
            ));
        }
        if !wrong_track_u5.is_empty() {
            raw_warnings.push(format!(
                "{} track page(s) with wrong u5/num_rl shape; run repair_pdb_wrong_track_u5_num_rl to fix.",
                wrong_track_u5.len()
            ));
            warn_parts.push(format!(
                "{} track page(s) wrong shape",
                wrong_track_u5.len()
            ));
        }
        if !wrong_history.is_empty() {
            raw_warnings.push(format!(
                "{} history page(s) with wrong u5/num_rl shape; run repair_pdb_wrong_history_page_shape to fix.",
                wrong_history.len()
            ));
            warn_parts.push(format!(
                "{} history page(s) wrong shape",
                wrong_history.len()
            ));
        }
        if !t00_multipage_active.is_empty() {
            raw_warnings.push(format!(
                "{} tt=0 track page(s) with ACTV flag in multi-page chain (rejected); run repair_pdb_t00_multipage_active_pages to fix.",
                t00_multipage_active.len()
            ));
            warn_parts.push(format!(
                "{} track page(s) ACTV in multi-page chain",
                t00_multipage_active.len()
            ));
        }
        if !ec_conflicts.is_empty() {
            raw_warnings.push(format!(
                "{} table(s) with empty_candidate pointing to another table's data page \
                 (may cause write-pointer conflict rejection); run repair_pdb_ec_data_page_conflict to fix.",
                ec_conflicts.len()
            ));
            fail_parts.push(format!(
                "{} table(s) write-pointer conflict",
                ec_conflicts.len()
            ));
        }
        let integrity_status = if !fail_parts.is_empty() {
            DiagStatus::Fail
        } else if !warn_parts.is_empty() {
            DiagStatus::Warn
        } else {
            DiagStatus::Pass
        };
        let detail = if fail_parts.is_empty() && warn_parts.is_empty() {
            "sentinel b-tree, page shapes, row IDs, and write pointers OK".to_string()
        } else {
            let mut segments = Vec::new();
            if !fail_parts.is_empty() {
                segments.push(fail_parts.join("; ") + " — run repair_usb_diagnostics to fix");
            }
            if !warn_parts.is_empty() {
                segments.push(
                    warn_parts.join("; ")
                        + " — player-tolerant; repair available for full desktop DJ software compatibility",
                );
            }
            segments.join(" | ")
        };
        checks.push(DiagCheck {
            label: "PDB structural integrity".to_string(),
            status: integrity_status,
            detail,
            link: None,
        });
    }

    checks.push(DiagCheck {
        label: "PDB playlists".to_string(),
        status: DiagStatus::Pass,
        detail: format!(
            "{} tree nodes, {} entries",
            parsed.playlist_tree.len(),
            parsed.playlist_entries.len(),
        ),
        link: None,
    });

    // History playlists in PDB
    {
        let hp = parsed.history_playlists.len();
        let he = parsed.history_entries.len();
        let hr = parsed.history_rows.len();
        if hp > 0 || he > 0 || hr > 0 {
            checks.push(DiagCheck {
                label: "PDB history".to_string(),
                status: DiagStatus::Pass,
                detail: format!("{hp} playlists, {he} entries, {hr} rows"),
                link: None,
            });
        } else {
            checks.push(DiagCheck {
                label: "PDB history".to_string(),
                status: DiagStatus::Pass,
                detail: "empty".to_string(),
                link: None,
            });
        }
    }

    // num_rl=8191 is a valid external-library sentinel; treat as compatibility info.
    checks.push(DiagCheck {
        label: "num_rl=8191 pages".to_string(),
        status: DiagStatus::Pass,
        detail: format!(
            "{} of {} pages (using nrs fallback)",
            diag.pages_with_num_rl_8191, diag.total_pages
        ),
        link: None,
    });

    // nrs u8 wrapping is expected on some pages; recovery is part of normal parsing.
    checks.push(DiagCheck {
        label: "nrs wrapping".to_string(),
        status: DiagStatus::Pass,
        detail: format!(
            "{} pages with row count exceeding nrs header",
            diag.nrs_wrapping_pages
        ),
        link: None,
    });

    // Orphaned playlist entries
    let track_ids: HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
    let orphaned: Vec<u32> = parsed
        .playlist_entries
        .iter()
        .filter(|e| !track_ids.contains(&e.track_id))
        .map(|e| e.track_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let orphan_status = if orphaned.is_empty() {
        DiagStatus::Pass
    } else {
        DiagStatus::Warn
    };
    checks.push(DiagCheck {
        label: "Orphaned entries".to_string(),
        status: orphan_status,
        detail: format!(
            "{} entries reference {} track IDs not in PDB",
            parsed
                .playlist_entries
                .iter()
                .filter(|e| !track_ids.contains(&e.track_id))
                .count(),
            orphaned.len()
        ),
        link: None,
    });

    // Parse warnings
    if !parsed.warnings.is_empty() {
        raw_warnings.extend(parsed.warnings.iter().cloned());
        checks.push(DiagCheck {
            label: "Parse warnings".to_string(),
            status: DiagStatus::Warn,
            detail: format!("{} warnings during parsing", parsed.warnings.len()),
            link: None,
        });
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    (
        DiagSection {
            title: "PDB Integrity".to_string(),
            status,
            checks,
            counts: None,
        },
        Some((parsed, diag)),
    )
}

pub(crate) fn diagnose_edb_access(
    usb_root: &std::path::Path,
    raw_warnings: &mut Vec<String>,
) -> DiagSection {
    let mut checks = Vec::new();

    let edb_path = vendor_edb_path(usb_root);
    if !edb_path.exists() {
        checks.push(DiagCheck {
            label: "eDB".to_string(),
            status: DiagStatus::Warn,
            detail: "File not found".to_string(),
            link: None,
        });
    } else {
        let mut edb_warnings = Vec::new();
        let conn = open_edb_from_usb_root(usb_root, &mut edb_warnings);
        let (status, detail) = if conn.is_some() {
            let key_msg = edb_warnings
                .last()
                .cloned()
                .unwrap_or_else(|| "Opened successfully".to_string());
            (DiagStatus::Pass, key_msg)
        } else {
            let msg = edb_warnings
                .last()
                .cloned()
                .unwrap_or_else(|| "Unable to open".to_string());
            (DiagStatus::Fail, msg)
        };
        raw_warnings.extend(edb_warnings);
        checks.push(DiagCheck {
            label: "eDB".to_string(),
            status,
            detail,
            link: None,
        });
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        title: "Database Access".to_string(),
        status,
        checks,
        counts: None,
    }
}

#[cfg(test)]
pub(crate) fn diagnose_contents_integrity(
    contents_count: usize,
    indexed_count: usize,
) -> DiagSection {
    let mut checks = Vec::new();

    checks.push(DiagCheck {
        label: "Audio files".to_string(),
        status: DiagStatus::Pass,
        detail: format!("{contents_count} audio files found"),
        link: None,
    });

    checks.push(DiagCheck {
        label: "Indexed tracks".to_string(),
        status: DiagStatus::Pass,
        detail: format!("{indexed_count} indexed track path(s) in PDB/eDB"),
        link: None,
    });

    let mismatch = contents_count as i64 - indexed_count as i64;
    let mismatch_status = if mismatch == 0 {
        DiagStatus::Pass
    } else if mismatch > 0 {
        DiagStatus::Warn
    } else if mismatch.unsigned_abs() <= 5 {
        DiagStatus::Warn
    } else {
        DiagStatus::Fail
    };
    checks.push(DiagCheck {
        label: "Count match".to_string(),
        status: mismatch_status,
        detail: if mismatch == 0 {
            "Exact match".to_string()
        } else if mismatch > 0 {
            format!("{mismatch} audio files not playlist-referenced in PDB/eDB")
        } else {
            format!(
                "{} DB entries have no matching audio file",
                mismatch.unsigned_abs()
            )
        },
        link: None,
    });

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        title: "Contents Integrity".to_string(),
        status,
        checks,
        counts: Some(DiagCountsSummary {
            contents_count,
            indexed_count,
            mismatch_count: mismatch,
        }),
    }
}

pub(crate) fn diagnose_contents_integrity_db_only(
    pdb_indexed_count: usize,
    edb_indexed_count: usize,
    pdb_only_count: usize,
    edb_only_count: usize,
    history_only_count: usize,
) -> DiagSection {
    let mut checks = Vec::new();

    checks.push(DiagCheck {
        label: "PDB indexed paths".to_string(),
        status: DiagStatus::Pass,
        detail: format!("{pdb_indexed_count} canonical track path(s)"),
        link: None,
    });
    checks.push(DiagCheck {
        label: "eDB indexed paths".to_string(),
        status: DiagStatus::Pass,
        detail: format!("{edb_indexed_count} canonical track path(s)"),
        link: None,
    });

    let disagreement_status = if pdb_only_count == 0 && edb_only_count == 0 {
        DiagStatus::Pass
    } else {
        DiagStatus::Warn
    };
    let history_suffix = if history_only_count > 0 {
        format!("; {history_only_count} history-only track(s)")
    } else {
        String::new()
    };
    let disagreement_detail = if pdb_only_count == 0
        && edb_only_count == 0
        && history_only_count == 0
    {
        "PDB/eDB canonical-path sets agree".to_string()
    } else if pdb_only_count == 0 && edb_only_count == 0 {
        format!("{history_only_count} history-only track(s)")
    } else if pdb_only_count > 0 && edb_only_count > 0 {
        format!(
            "{pdb_only_count} PDB-only path(s); {edb_only_count} eDB-only path(s){history_suffix}"
        )
    } else if pdb_only_count > 0 {
        format!("{pdb_only_count} PDB-only path(s){history_suffix}")
    } else {
        format!("{edb_only_count} eDB-only path(s){history_suffix}")
    };
    let agreement_link = if history_only_count > 0 {
        Some("event-log".to_string())
    } else {
        None
    };
    checks.push(DiagCheck {
        label: "PDB/eDB path agreement".to_string(),
        status: disagreement_status,
        detail: disagreement_detail,
        link: agreement_link,
    });

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        title: "Contents Integrity".to_string(),
        status,
        checks,
        counts: Some(DiagCountsSummary {
            contents_count: pdb_indexed_count,
            indexed_count: edb_indexed_count,
            mismatch_count: pdb_only_count as i64 - edb_only_count as i64,
        }),
    }
}

pub(crate) fn diagnose_analysis_integrity(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
    edb_playlist_tracks: Option<&HashMap<String, Vec<UsbTrack>>>,
) -> DiagSection {
    let mut checks = Vec::new();

    if let Some(parsed) = parsed {
        let total_tracks = parsed.tracks.len();
        let mut path_refs = HashSet::<String>::new();
        let mut tracks_with_ref = 0usize;
        for track in &parsed.tracks {
            let path = normalize_analysis_path_for_identity(&track.anlz_path);
            if path.is_empty() {
                continue;
            }
            tracks_with_ref += 1;
            path_refs.insert(path);
        }
        let missing = total_tracks.saturating_sub(tracks_with_ref);
        checks.push(DiagCheck {
            label: "PDB analysis refs".to_string(),
            status: if missing == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            detail: format!(
                "{tracks_with_ref}/{total_tracks} track row(s) have analysis paths; {} unique bundle reference(s)",
                path_refs.len()
            ),
            link: None,
        });
    } else {
        checks.push(DiagCheck {
            label: "PDB analysis refs".to_string(),
            status: DiagStatus::Fail,
            detail: "No PDB data available".to_string(),
            link: None,
        });
    }

    if let Some(edb_playlist_tracks) = edb_playlist_tracks {
        let mut analysis_by_content_id = HashMap::<String, String>::new();
        let mut total_playlist_entries = 0usize;
        for tracks in edb_playlist_tracks.values() {
            total_playlist_entries += tracks.len();
            for track in tracks {
                let key = if track.id.trim().is_empty() {
                    track_identity_key(track.identity_path(), &track.title, &track.artist, None)
                } else {
                    track.id.clone()
                };
                analysis_by_content_id.entry(key).or_insert_with(|| {
                    normalize_analysis_path_for_identity(
                        track
                            .usb_analysis_path_raw
                            .as_deref()
                            .or(track.usb_analysis_path.as_deref())
                            .unwrap_or_default(),
                    )
                });
            }
        }
        let unique_content_rows = analysis_by_content_id.len();
        let refs = analysis_by_content_id
            .values()
            .filter(|path| !path.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        let unique_refs = refs.iter().cloned().collect::<HashSet<_>>().len();
        let missing = unique_content_rows.saturating_sub(refs.len());
        checks.push(DiagCheck {
            label: "eDB analysis refs".to_string(),
            status: if missing == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            detail: format!(
                "{}/{} playlist-linked content row(s) have analysis paths; {} unique bundle reference(s) from {} playlist entries",
                refs.len(),
                unique_content_rows,
                unique_refs,
                total_playlist_entries
            ),
            link: None,
        });
    } else {
        checks.push(DiagCheck {
            label: "eDB analysis refs".to_string(),
            status: DiagStatus::Warn,
            detail: "No eDB playlist data available".to_string(),
            link: None,
        });
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    DiagSection {
        title: "Analysis Files".to_string(),
        status,
        checks,
        counts: None,
    }
}

#[cfg(test)]
pub(crate) fn diagnose_playlist_resolution(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
) -> (DiagSection, Vec<PlaylistDiagEntry>) {
    diagnose_playlist_resolution_with_db(parsed, None)
}

#[cfg(test)]
pub(crate) fn diagnose_playlist_resolution_with_db(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
    edb_playlist_tracks: Option<&HashMap<String, Vec<UsbTrack>>>,
) -> (DiagSection, Vec<PlaylistDiagEntry>) {
    diagnose_playlist_resolution_with_edb_internal(parsed, edb_playlist_tracks, |_, _, _| {})
}

pub(crate) fn diagnose_playlist_resolution_with_edb_internal<F>(
    parsed: Option<&crate::pdb_reader::ParsedPdb>,
    edb_playlist_tracks: Option<&HashMap<String, Vec<UsbTrack>>>,
    mut on_playlist_progress: F,
) -> (DiagSection, Vec<PlaylistDiagEntry>)
where
    F: FnMut(usize, usize, &str),
{
    let mut checks = Vec::new();
    let mut playlist_details = Vec::new();

    let parsed = match parsed {
        Some(p) => p,
        None => {
            checks.push(DiagCheck {
                label: "Playlist data".to_string(),
                status: DiagStatus::Fail,
                detail: "No PDB data available".to_string(),
                link: None,
            });
            return (
                DiagSection {
                    title: "Playlist Resolution".to_string(),
                    status: DiagStatus::Fail,
                    checks,
                    counts: None,
                },
                playlist_details,
            );
        }
    };

    let track_ids: HashSet<u32> = parsed.tracks.iter().map(|t| t.id).collect();
    let pdb_track_key_by_id = parsed
        .tracks
        .iter()
        .map(|t| {
            let artist = parsed
                .artists
                .get(&t.artist_id)
                .map(String::as_str)
                .unwrap_or("");
            let key = track_identity_key(
                &t.track_file_path,
                &t.title,
                artist,
                Some(&t.id.to_string()),
            );
            (t.id, key)
        })
        .collect::<HashMap<_, _>>();
    let mut entries_by_playlist: HashMap<u32, Vec<&crate::pdb_reader::PdbPlaylistEntryRow>> =
        HashMap::new();
    for entry in &parsed.playlist_entries {
        entries_by_playlist
            .entry(entry.playlist_id)
            .or_default()
            .push(entry);
    }

    let mut leaves: Vec<&crate::pdb_reader::PdbPlaylistTreeRow> = parsed
        .playlist_tree
        .iter()
        .filter(|n| !n.row_is_folder)
        .collect();
    leaves.sort_by_key(|n| n.sort_order);
    let mut grouped_leaves = Vec::<(String, String, u32, Vec<u32>)>::new();
    let mut grouped_leaf_idx = HashMap::<String, usize>::new();
    for leaf in &leaves {
        let canonical = canonicalize_playlist_name(&leaf.name);
        if let Some(existing_idx) = grouped_leaf_idx.get(&canonical).copied() {
            grouped_leaves[existing_idx].3.push(leaf.id);
            continue;
        }
        grouped_leaf_idx.insert(canonical.clone(), grouped_leaves.len());
        grouped_leaves.push((canonical, leaf.name.clone(), leaf.sort_order, vec![leaf.id]));
    }
    grouped_leaves.sort_by_key(|(_, _, sort_order, _)| *sort_order);
    let pdb_leaf_name_keys = grouped_leaves
        .iter()
        .map(|(canonical, _, _, _)| canonical.clone())
        .collect::<HashSet<_>>();
    let edb_playlist_tracks_canonical = edb_playlist_tracks.map(|m| {
        m.iter()
            .map(|(name, tracks)| (canonicalize_playlist_name(name), tracks))
            .collect::<HashMap<_, _>>()
    });

    let mut all_pass = true;
    let mut any_fail = false;
    let mut cross_source_matches_total = 0usize;
    let mut cross_source_pdb_total = 0usize;
    let mut cross_source_edb_total = 0usize;

    for (idx, (_canonical_name, display_name, _sort_order, playlist_ids)) in
        grouped_leaves.iter().enumerate()
    {
        let done = idx + 1;
        let total = grouped_leaves.len();
        on_playlist_progress(
            done,
            total,
            &format!("Resolving playlist {done}/{total}: {display_name}"),
        );
        let mut unique_track_ids = HashSet::<u32>::new();
        let mut pdb_ids = HashSet::<String>::new();
        for playlist_id in playlist_ids {
            if let Some(entries) = entries_by_playlist.get(playlist_id) {
                for entry in entries {
                    unique_track_ids.insert(entry.track_id);
                    let identity = pdb_track_key_by_id
                        .get(&entry.track_id)
                        .cloned()
                        .unwrap_or_else(|| format!("id:{}", entry.track_id));
                    pdb_ids.insert(identity);
                }
            }
        }
        let total = unique_track_ids.len();
        let resolved = unique_track_ids
            .iter()
            .filter(|track_id| track_ids.contains(track_id))
            .count();
        let edb_tracks = edb_playlist_tracks
            .and_then(|m| {
                m.get(display_name).or_else(|| {
                    let key = canonicalize_playlist_name(display_name);
                    edb_playlist_tracks_canonical
                        .as_ref()
                        .and_then(|cm| cm.get(&key).copied())
                })
            })
            .cloned()
            .unwrap_or_default();
        let edb_ids = edb_tracks
            .iter()
            .map(|t| track_identity_key(t.identity_path(), &t.title, &t.artist, Some(&t.id)))
            .collect::<HashSet<String>>();
        let matched_entries = pdb_ids.intersection(&edb_ids).count();
        let pdb_entries = pdb_ids.len();
        let edb_entries = edb_ids.len();
        let pdb_match_rate = if pdb_entries > 0 {
            matched_entries as f64 / pdb_entries as f64
        } else {
            1.0
        };
        let edb_match_rate = if edb_entries > 0 {
            matched_entries as f64 / edb_entries as f64
        } else {
            1.0
        };
        cross_source_matches_total += matched_entries;
        cross_source_pdb_total += pdb_entries;
        cross_source_edb_total += edb_entries;
        let rate = if total > 0 {
            resolved as f64 / total as f64
        } else {
            1.0
        };
        let status = if rate >= 1.0 {
            DiagStatus::Pass
        } else if rate >= 0.8 {
            DiagStatus::Warn
        } else {
            DiagStatus::Fail
        };
        if !matches!(status, DiagStatus::Pass) {
            all_pass = false;
        }
        if matches!(status, DiagStatus::Fail) {
            any_fail = true;
        }
        playlist_details.push(PlaylistDiagEntry {
            name: display_name.clone(),
            total_entries: total,
            resolved_entries: resolved,
            resolution_rate: (rate * 1000.0).round() / 1000.0,
            status,
            pdb_entries,
            edb_entries,
            matched_entries,
            pdb_match_rate: (pdb_match_rate * 1000.0).round() / 1000.0,
            edb_match_rate: (edb_match_rate * 1000.0).round() / 1000.0,
        });
    }

    let mut edb_only_playlist_count = 0usize;
    if let Some(edb_map) = edb_playlist_tracks {
        for (edb_name, edb_tracks) in edb_map {
            let key = canonicalize_playlist_name(edb_name);
            if pdb_leaf_name_keys.contains(&key) {
                continue;
            }
            edb_only_playlist_count += 1;
            all_pass = false;
            any_fail = true;
            let edb_ids = edb_tracks
                .iter()
                .map(|t| track_identity_key(t.identity_path(), &t.title, &t.artist, Some(&t.id)))
                .collect::<HashSet<String>>();
            let edb_entries = edb_ids.len();
            cross_source_edb_total += edb_entries;
            playlist_details.push(PlaylistDiagEntry {
                name: edb_name.clone(),
                total_entries: edb_entries,
                resolved_entries: 0,
                resolution_rate: 0.0,
                status: DiagStatus::Fail,
                pdb_entries: 0,
                edb_entries,
                matched_entries: 0,
                pdb_match_rate: 0.0,
                edb_match_rate: 0.0,
            });
            checks.push(DiagCheck {
                label: edb_name.clone(),
                status: DiagStatus::Fail,
                detail: format!("present in eDB only ({edb_entries} entries), missing from PDB"),
                link: None,
            });
        }
    }

    let total_entries: usize = playlist_details.iter().map(|p| p.total_entries).sum();
    let total_resolved: usize = playlist_details.iter().map(|p| p.resolved_entries).sum();
    let overall_rate = if total_entries > 0 {
        total_resolved as f64 / total_entries as f64
    } else {
        1.0
    };

    checks.push(DiagCheck {
        label: "Overall resolution".to_string(),
        status: if all_pass {
            DiagStatus::Pass
        } else if any_fail {
            DiagStatus::Fail
        } else {
            DiagStatus::Warn
        },
        detail: format!(
            "{total_resolved}/{total_entries} entries resolve ({:.1}%) across {} playlists",
            overall_rate * 100.0,
            grouped_leaves.len()
        ),
        link: None,
    });
    if edb_only_playlist_count > 0 {
        checks.push(DiagCheck {
            label: "DB-only playlists".to_string(),
            status: DiagStatus::Fail,
            detail: format!("{edb_only_playlist_count} playlist(s) exist in eDB but not in PDB"),
            link: None,
        });
    }
    if edb_playlist_tracks.is_some() {
        let pdb_rate = if cross_source_pdb_total > 0 {
            cross_source_matches_total as f64 / cross_source_pdb_total as f64
        } else {
            1.0
        };
        let edb_rate = if cross_source_edb_total > 0 {
            cross_source_matches_total as f64 / cross_source_edb_total as f64
        } else {
            1.0
        };
        let status = DiagStatus::Pass;
        let detail = format!(
            "matched {} track keys; PDB {:.1}% ({}/{}), DB {:.1}% ({}/{})",
            cross_source_matches_total,
            pdb_rate * 100.0,
            cross_source_matches_total,
            cross_source_pdb_total,
            edb_rate * 100.0,
            cross_source_matches_total,
            cross_source_edb_total
        );
        checks.push(DiagCheck {
            label: "PDB vs eDB key overlap (informational)".to_string(),
            status,
            detail,
            link: None,
        });
    }
    checks.push(DiagCheck {
        label: "Operational interpretation".to_string(),
        status: DiagStatus::Pass,
        detail: "pass/warn here means operationally usable; strict parity may still fail on the same USB".to_string(),
        link: None,
    });

    // List playlists with issues
    for pd in &playlist_details {
        if pd.resolution_rate < 1.0 {
            checks.push(DiagCheck {
                label: pd.name.clone(),
                status: pd.status.clone(),
                detail: format!(
                    "{}/{} entries ({:.1}%)",
                    pd.resolved_entries,
                    pd.total_entries,
                    pd.resolution_rate * 100.0
                ),
                link: None,
            });
        }
    }

    let status = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    (
        DiagSection {
            title: "Playlist Resolution".to_string(),
            status,
            checks,
            counts: None,
        },
        playlist_details,
    )
}

pub(crate) fn build_usb_parity_comparison(
    parsed: &crate::pdb_reader::ParsedPdb,
    edb_playlists: &HashMap<String, ExportDbPlaylist>,
    reference_only_edb_fields: &ReferenceOnlyEdbFieldUsage,
    strict_raw_coverage: Option<StrictRawCoverageParity>,
) -> (
    Vec<DiagCheck>,
    Vec<DiagSummaryRow>,
    Vec<UsbParityPlaylistDetail>,
    DiagStatus,
) {
    #[derive(Clone)]
    struct PdbPlaylistTrackDetail {
        title: String,
        artist: String,
        album: Option<String>,
        key_name: Option<String>,
        track_number: Option<u32>,
        tempo_x100: Option<u32>,
        duration_seconds: Option<u32>,
        analysis_path: String,
        media_path: String,
        artwork_path: Option<String>,
        artist_id: u32,
        album_id: u32,
        key_id: u32,
        artwork_id: u32,
    }

    #[derive(Clone)]
    struct PdbPlaylistInfo {
        playlist_id: u32,
        sort_order: u32,
        track_keys: Vec<String>,
        track_details_by_key: HashMap<String, PdbPlaylistTrackDetail>,
        /// Maps identity_key → meta_key for secondary cross-DB matching.
        meta_by_track_key: HashMap<String, String>,
        /// Maps identity_key → normalized analysis-path key.
        analysis_by_track_key: HashMap<String, String>,
        duplicate_entries: usize,
    }

    #[derive(Clone)]
    struct EdbPlaylistInfo {
        playlist_id: Option<u32>,
        sort_order: Option<u32>,
        track_keys: Vec<String>,
        track_details_by_key: HashMap<String, UsbTrack>,
        /// Maps identity_key → meta_key for secondary cross-DB matching.
        meta_by_track_key: HashMap<String, String>,
        /// Maps identity_key → normalized analysis-path key.
        analysis_by_track_key: HashMap<String, String>,
    }

    fn normalized_optional_str(value: Option<&str>) -> String {
        value
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or_default()
            .to_string()
    }

    fn normalized_required_str(value: &str) -> String {
        value.trim().to_string()
    }

    fn normalized_artist_for_gate(value: &str) -> String {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("unknown artist") {
            "Unknown artist".to_string()
        } else {
            trimmed.to_string()
        }
    }

    fn sample_track_label(shared_key: &str, pdb_detail: &PdbPlaylistTrackDetail) -> String {
        let title = pdb_detail.title.trim();
        if !title.is_empty() {
            title.to_string()
        } else {
            shared_key.to_string()
        }
    }

    let track_key_by_id = parsed
        .tracks
        .iter()
        .map(|t| {
            let artist = parsed
                .artists
                .get(&t.artist_id)
                .map(String::as_str)
                .unwrap_or("");
            let key = track_identity_key(
                &t.track_file_path,
                &t.title,
                artist,
                Some(&t.id.to_string()),
            );
            (t.id, key)
        })
        .collect::<HashMap<_, _>>();

    let pdb_track_detail_by_id = parsed
        .tracks
        .iter()
        .map(|t| {
            let artist = parsed
                .artists
                .get(&t.artist_id)
                .cloned()
                .unwrap_or_default();
            let detail = PdbPlaylistTrackDetail {
                title: t.title.clone(),
                artist,
                album: parsed.albums.get(&t.album_id).cloned(),
                key_name: parsed.keys.get(&t.key_id).cloned(),
                track_number: (t.track_number > 0).then_some(t.track_number),
                tempo_x100: (t.tempo_x100 > 0).then_some(t.tempo_x100),
                duration_seconds: t.duration_seconds,
                analysis_path: t.anlz_path.clone(),
                media_path: t.track_file_path.clone(),
                artwork_path: parsed.artworks.get(&t.artwork_id).cloned(),
                artist_id: t.artist_id,
                album_id: t.album_id,
                key_id: t.key_id,
                artwork_id: t.artwork_id,
            };
            (t.id, detail)
        })
        .collect::<HashMap<_, _>>();

    let mut pdb_info_by_playlist = HashMap::<String, PdbPlaylistInfo>::new();
    let mut leaves = parsed
        .playlist_tree
        .iter()
        .filter(|n| !n.row_is_folder)
        .cloned()
        .collect::<Vec<_>>();
    leaves.sort_by_key(|n| n.sort_order);
    for leaf in &leaves {
        let mut rows = parsed
            .playlist_entries
            .iter()
            .filter(|e| e.playlist_id == leaf.id)
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by_key(|e| e.entry_index);

        let mut track_keys = Vec::<String>::new();
        let mut track_details_by_key = HashMap::<String, PdbPlaylistTrackDetail>::new();
        let mut meta_by_track_key = HashMap::<String, String>::new();
        let mut analysis_by_track_key = HashMap::<String, String>::new();
        for row in &rows {
            let key = track_key_by_id
                .get(&row.track_id)
                .cloned()
                .unwrap_or_else(|| format!("id:{}", row.track_id));
            track_keys.push(key.clone());
            if let Some(detail) = pdb_track_detail_by_id.get(&row.track_id).cloned() {
                let meta = build_meta_key(&detail.title, &detail.artist);
                if meta != "meta:|" {
                    meta_by_track_key.insert(key.clone(), meta);
                }
                let analysis = normalize_analysis_path_for_identity(&detail.analysis_path);
                if !analysis.is_empty() {
                    analysis_by_track_key.insert(key.clone(), analysis);
                }
                track_details_by_key.entry(key).or_insert(detail);
            }
        }

        let duplicate_entries = track_keys
            .len()
            .saturating_sub(unique_preserve_order(&track_keys).len());
        pdb_info_by_playlist.insert(
            canonicalize_playlist_name(&leaf.name),
            PdbPlaylistInfo {
                playlist_id: leaf.id,
                sort_order: leaf.sort_order,
                track_keys,
                track_details_by_key,
                meta_by_track_key,
                analysis_by_track_key,
                duplicate_entries,
            },
        );
    }

    let mut edb_info_by_playlist = HashMap::<String, EdbPlaylistInfo>::new();
    for (name, playlist) in edb_playlists {
        let mut track_keys = Vec::<String>::new();
        let mut track_details_by_key = HashMap::<String, UsbTrack>::new();
        let mut meta_by_track_key = HashMap::<String, String>::new();
        let mut analysis_by_track_key = HashMap::<String, String>::new();
        for track in &playlist.tracks {
            let key = track_identity_key(
                track.identity_path(),
                &track.title,
                &track.artist,
                Some(&track.id),
            );
            let meta = build_meta_key(&track.title, &track.artist);
            if meta != "meta:|" {
                meta_by_track_key.insert(key.clone(), meta);
            }
            let analysis = normalize_analysis_path_for_identity(
                track
                    .usb_analysis_path_raw
                    .as_deref()
                    .or(track.usb_analysis_path.as_deref())
                    .unwrap_or_default(),
            );
            if !analysis.is_empty() {
                analysis_by_track_key.insert(key.clone(), analysis);
            }
            track_keys.push(key.clone());
            track_details_by_key
                .entry(key)
                .or_insert_with(|| track.clone());
        }

        edb_info_by_playlist.insert(
            canonicalize_playlist_name(name),
            EdbPlaylistInfo {
                playlist_id: u32::try_from(playlist.playlist_id).ok(),
                sort_order: u32::try_from(playlist.sort_order).ok(),
                track_keys,
                track_details_by_key,
                meta_by_track_key,
                analysis_by_track_key,
            },
        );
    }

    let mut all_keys = HashSet::<String>::new();
    for key in pdb_info_by_playlist.keys() {
        all_keys.insert(key.clone());
    }
    for key in edb_info_by_playlist.keys() {
        all_keys.insert(key.clone());
    }

    let mut details = Vec::<UsbParityPlaylistDetail>::new();
    for key in all_keys {
        let pdb_name = leaves
            .iter()
            .find(|n| canonicalize_playlist_name(&n.name) == key)
            .map(|n| n.name.clone());
        let edb_name = edb_playlists
            .keys()
            .find(|n| canonicalize_playlist_name(n) == key)
            .cloned();

        let display = pdb_name
            .clone()
            .or(edb_name.clone())
            .unwrap_or_else(|| key.clone());

        let pdb_info = pdb_info_by_playlist.get(&key).cloned();
        let edb_info = edb_info_by_playlist.get(&key).cloned();

        let pdb_list = pdb_info
            .as_ref()
            .map(|info| info.track_keys.clone())
            .unwrap_or_default();
        let edb_list = edb_info
            .as_ref()
            .map(|info| info.track_keys.clone())
            .unwrap_or_default();

        let pdb_unique_list = unique_preserve_order(&pdb_list);
        let edb_unique_list = unique_preserve_order(&edb_list);

        let pdb_set = pdb_unique_list.iter().cloned().collect::<HashSet<_>>();
        let edb_set = edb_unique_list.iter().cloned().collect::<HashSet<_>>();

        // Secondary meta-key cross-matching: resolve path: vs meta: mismatches.
        // When PDB has no path (meta: key) and eDB has a path (path: key) for the
        // same title+artist, count them as matched.
        let pdb_meta_map = pdb_info.as_ref().map(|i| &i.meta_by_track_key);
        let edb_meta_map = edb_info.as_ref().map(|i| &i.meta_by_track_key);
        let pdb_analysis_map = pdb_info.as_ref().map(|i| &i.analysis_by_track_key);
        let edb_analysis_map = edb_info.as_ref().map(|i| &i.analysis_by_track_key);

        // Secondary meta-key cross-matching: when one side has a path: key and the
        // other has a meta: key (empty path), match them if title+artist are the same.
        let mut meta_matched_pdb = HashSet::<String>::new();
        let mut meta_matched_edb = HashSet::<String>::new();
        let mut consumed_meta = HashSet::<String>::new();
        let mut meta_match_pairs = Vec::<(String, String)>::new();
        let mut analysis_matched_pdb = HashSet::<String>::new();
        let mut analysis_matched_edb = HashSet::<String>::new();
        let mut consumed_analysis = HashSet::<String>::new();
        let mut analysis_match_pairs = Vec::<(String, String)>::new();

        // Primary secondary cross-match: analysis path.
        for pdb_key in pdb_set.difference(&edb_set) {
            let Some(pdb_analysis) = pdb_analysis_map.and_then(|m| m.get(pdb_key)).cloned() else {
                continue;
            };
            if consumed_analysis.contains(&pdb_analysis) {
                continue;
            }
            let edb_match = edb_set.difference(&pdb_set).find(|edb_k| {
                if analysis_matched_edb.contains(*edb_k) {
                    return false;
                }
                let edb_analysis = edb_analysis_map.and_then(|m| m.get(*edb_k)).cloned();
                edb_analysis.as_ref() == Some(&pdb_analysis)
            });
            if let Some(edb_key) = edb_match {
                analysis_matched_pdb.insert(pdb_key.clone());
                analysis_matched_edb.insert(edb_key.clone());
                analysis_match_pairs.push((pdb_key.clone(), edb_key.clone()));
                consumed_analysis.insert(pdb_analysis);
            }
        }

        // For each unmatched PDB key, find its meta key and look for an eDB track
        // with the same meta that is also unmatched.
        for pdb_key in pdb_set.difference(&edb_set) {
            if analysis_matched_pdb.contains(pdb_key) {
                continue;
            }
            let pdb_meta = if pdb_key.starts_with("meta:") {
                Some(pdb_key.clone())
            } else {
                pdb_meta_map.and_then(|m| m.get(pdb_key)).cloned()
            };
            if let Some(meta) = pdb_meta {
                if consumed_meta.contains(&meta) {
                    continue;
                }
                // Find an unmatched eDB key with the same meta
                let edb_match = edb_set.difference(&pdb_set).find(|edb_k| {
                    if meta_matched_edb.contains(*edb_k) || analysis_matched_edb.contains(*edb_k) {
                        return false;
                    }
                    let edb_meta = if edb_k.starts_with("meta:") {
                        Some((*edb_k).clone())
                    } else {
                        edb_meta_map.and_then(|m| m.get(*edb_k)).cloned()
                    };
                    edb_meta.as_ref() == Some(&meta)
                });
                if let Some(edb_key) = edb_match {
                    meta_matched_pdb.insert(pdb_key.clone());
                    meta_matched_edb.insert(edb_key.clone());
                    meta_match_pairs.push((pdb_key.clone(), edb_key.clone()));
                    consumed_meta.insert(meta);
                }
            }
        }

        let analysis_extra = analysis_matched_pdb.len();
        let meta_extra = meta_matched_pdb.len();
        let meta_matched: HashSet<String> = meta_matched_pdb
            .iter()
            .chain(meta_matched_edb.iter())
            .cloned()
            .collect();
        let analysis_matched: HashSet<String> = analysis_matched_pdb
            .iter()
            .chain(analysis_matched_edb.iter())
            .cloned()
            .collect();
        let matched_tracks = pdb_set.intersection(&edb_set).count() + analysis_extra + meta_extra;
        let only_in_pdb = pdb_set
            .difference(&edb_set)
            .count()
            .saturating_sub(analysis_extra)
            .saturating_sub(meta_extra);
        let only_in_edb = edb_set
            .difference(&pdb_set)
            .count()
            .saturating_sub(analysis_extra)
            .saturating_sub(meta_extra);
        // Build a canonical-key mapping for meta-matched pairs so order
        // comparison uses the same string on both sides.
        let mut pdb_key_canonical = HashMap::<&String, String>::new();
        let mut edb_key_canonical = HashMap::<&String, String>::new();
        for pdb_key in &analysis_matched_pdb {
            let analysis = pdb_analysis_map
                .and_then(|m| m.get(pdb_key))
                .cloned()
                .unwrap_or_else(|| pdb_key.clone());
            pdb_key_canonical.insert(pdb_key, analysis);
        }
        for edb_key in &analysis_matched_edb {
            let analysis = edb_analysis_map
                .and_then(|m| m.get(edb_key))
                .cloned()
                .unwrap_or_else(|| edb_key.clone());
            edb_key_canonical.insert(edb_key, analysis);
        }
        for pdb_key in &meta_matched_pdb {
            let meta = if pdb_key.starts_with("meta:") {
                pdb_key.clone()
            } else {
                pdb_meta_map
                    .and_then(|m| m.get(pdb_key))
                    .cloned()
                    .unwrap_or_else(|| pdb_key.clone())
            };
            pdb_key_canonical.insert(pdb_key, meta);
        }
        for edb_key in &meta_matched_edb {
            let meta = if edb_key.starts_with("meta:") {
                edb_key.clone()
            } else {
                edb_meta_map
                    .and_then(|m| m.get(edb_key))
                    .cloned()
                    .unwrap_or_else(|| edb_key.clone())
            };
            edb_key_canonical.insert(edb_key, meta);
        }
        let pdb_common_order: Vec<String> = pdb_unique_list
            .iter()
            .filter(|k| {
                edb_set.contains(*k) || meta_matched.contains(*k) || analysis_matched.contains(*k)
            })
            .map(|k| {
                pdb_key_canonical
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| k.clone())
            })
            .collect();
        let edb_common_order: Vec<String> = edb_unique_list
            .iter()
            .filter(|k| {
                pdb_set.contains(*k) || meta_matched.contains(*k) || analysis_matched.contains(*k)
            })
            .map(|k| {
                edb_key_canonical
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| k.clone())
            })
            .collect();
        let order_mismatch = pdb_common_order != edb_common_order;

        let sample_only_in_pdb = pdb_unique_list
            .iter()
            .filter(|item| {
                !edb_set.contains(*item)
                    && !meta_matched.contains(*item)
                    && !analysis_matched.contains(*item)
            })
            .take(3)
            .cloned()
            .collect::<Vec<_>>();
        let sample_only_in_edb = edb_unique_list
            .iter()
            .filter(|item| {
                !pdb_set.contains(*item)
                    && !meta_matched.contains(*item)
                    && !analysis_matched.contains(*item)
            })
            .take(3)
            .cloned()
            .collect::<Vec<_>>();

        let mut sample_metadata_mismatches = Vec::<String>::new();
        let mut pdb_missing_core_metadata = 0usize;
        let mut edb_missing_core_metadata = 0usize;
        let mut artwork_mismatch_tracks = 0usize;
        let mut path_mismatch_tracks = 0usize;
        let mut dictionary_id_issue_tracks = 0usize;

        let shared_pairs = pdb_set
            .intersection(&edb_set)
            .map(|key| (key.clone(), key.clone()))
            .chain(analysis_match_pairs.iter().cloned())
            .chain(meta_match_pairs.iter().cloned())
            .collect::<Vec<_>>();

        for (pdb_key, edb_key) in shared_pairs {
            let pdb_detail = pdb_info
                .as_ref()
                .and_then(|info| info.track_details_by_key.get(&pdb_key));
            let edb_detail = edb_info
                .as_ref()
                .and_then(|info| info.track_details_by_key.get(&edb_key));

            let Some(pdb_detail) = pdb_detail else {
                continue;
            };
            let Some(edb_detail) = edb_detail else {
                continue;
            };

            let pdb_title = normalized_required_str(&pdb_detail.title);
            let edb_title = normalized_required_str(&edb_detail.title);
            let edb_artist_raw = normalized_required_str(&edb_detail.artist);
            let pdb_artist = normalized_artist_for_gate(&pdb_detail.artist);
            let edb_artist = normalized_artist_for_gate(&edb_detail.artist);
            let pdb_album = normalized_optional_str(pdb_detail.album.as_deref());
            let edb_album = normalized_optional_str(edb_detail.album.as_deref());
            let pdb_key = normalized_optional_str(pdb_detail.key_name.as_deref());
            let edb_key = normalized_optional_str(edb_detail.key.as_deref());
            let pdb_analysis = normalized_required_str(&pdb_detail.analysis_path);
            let edb_analysis = normalized_optional_str(
                edb_detail
                    .usb_analysis_path_raw
                    .as_deref()
                    .or(edb_detail.usb_analysis_path.as_deref()),
            );
            let pdb_media = normalized_required_str(&pdb_detail.media_path);
            let edb_media = normalized_required_str(
                edb_detail
                    .usb_media_path
                    .as_deref()
                    .unwrap_or(&edb_detail.file_path),
            );
            let pdb_artwork = normalized_optional_str(pdb_detail.artwork_path.as_deref());
            let edb_artwork = normalized_optional_str(edb_detail.artwork_path.as_deref());

            let edb_bpm_x100 = edb_detail
                .bpm
                .filter(|v| *v > 0.0)
                .map(|v| (v * 100.0).round() as u32);
            let edb_duration_seconds = edb_detail.duration_ms.map(|v| (v / 1000) as u32);

            let pdb_missing_required = pdb_title.is_empty()
                || (pdb_detail.tempo_x100.is_none() && edb_bpm_x100.is_some())
                || (pdb_detail.duration_seconds.is_none() && edb_duration_seconds.is_some())
                || pdb_analysis.is_empty()
                || pdb_media.is_empty()
                || (!edb_album.is_empty() && pdb_album.is_empty())
                || (!edb_key.is_empty() && pdb_key.is_empty());

            if pdb_missing_required {
                pdb_missing_core_metadata += 1;
            }

            let edb_missing_required = edb_title.is_empty()
                || (edb_bpm_x100.is_none() && pdb_detail.tempo_x100.is_some())
                || (edb_duration_seconds.is_none() && pdb_detail.duration_seconds.is_some())
                || edb_analysis.is_empty()
                || edb_media.is_empty()
                || (!pdb_album.is_empty() && edb_album.is_empty())
                || (!pdb_key.is_empty() && edb_key.is_empty());

            if edb_missing_required {
                edb_missing_core_metadata += 1;
            }

            let edb_artist_requires_dict = !edb_artist_raw.is_empty()
                && !edb_artist_raw.eq_ignore_ascii_case("unknown artist");
            let artist_id_unresolved = edb_artist_requires_dict
                && (pdb_detail.artist_id == 0
                    || !parsed.artists.contains_key(&pdb_detail.artist_id));
            let album_id_unresolved = !edb_album.is_empty()
                && (pdb_detail.album_id == 0 || !parsed.albums.contains_key(&pdb_detail.album_id));
            let key_id_unresolved = !edb_key.is_empty()
                && (pdb_detail.key_id == 0 || !parsed.keys.contains_key(&pdb_detail.key_id));
            let artwork_id_unresolved = !edb_artwork.is_empty()
                && (pdb_detail.artwork_id == 0
                    || !parsed.artworks.contains_key(&pdb_detail.artwork_id));
            let has_dictionary_id_issue = artist_id_unresolved
                || album_id_unresolved
                || key_id_unresolved
                || artwork_id_unresolved;
            if has_dictionary_id_issue {
                dictionary_id_issue_tracks += 1;
            }

            let media_path_mismatch = normalize_track_path_for_identity(&pdb_detail.media_path)
                != normalize_track_path_for_identity(
                    edb_detail
                        .usb_media_path
                        .as_deref()
                        .unwrap_or(&edb_detail.file_path),
                );
            let analysis_path_mismatch = normalize_usb_path_for_parity(&pdb_detail.analysis_path)
                != normalize_usb_path_for_parity(
                    edb_detail
                        .usb_analysis_path_raw
                        .as_deref()
                        .or(edb_detail.usb_analysis_path.as_deref())
                        .unwrap_or(""),
                );
            if media_path_mismatch || analysis_path_mismatch {
                path_mismatch_tracks += 1;
            }

            let pdb_has_artwork = !pdb_artwork.is_empty();
            let edb_has_artwork = !edb_artwork.is_empty();
            let artwork_mismatch = pdb_has_artwork != edb_has_artwork;
            if artwork_mismatch {
                artwork_mismatch_tracks += 1;
            }

            let mut mismatches = Vec::<String>::new();
            if pdb_title != edb_title {
                mismatches.push("title".to_string());
            }
            if pdb_artist != edb_artist {
                mismatches.push("artist".to_string());
            }
            if pdb_album != edb_album {
                mismatches.push("album".to_string());
            }
            if pdb_key != edb_key {
                mismatches.push("key".to_string());
            }
            if pdb_detail.track_number != edb_detail.track_number {
                mismatches.push("trackNumber".to_string());
            }
            if pdb_detail.tempo_x100 != edb_bpm_x100 {
                mismatches.push("tempoX100".to_string());
            }
            if pdb_detail.duration_seconds != edb_duration_seconds {
                mismatches.push("durationSeconds".to_string());
            }
            if analysis_path_mismatch {
                mismatches.push("analysisPath".to_string());
            }
            if media_path_mismatch {
                mismatches.push("mediaPath".to_string());
            }
            if artwork_mismatch {
                if pdb_has_artwork && !edb_has_artwork {
                    mismatches.push("artworkMissingEdb".to_string());
                } else {
                    mismatches.push("artworkMissingPdb".to_string());
                }
            }
            if artist_id_unresolved {
                mismatches.push("artistDictId".to_string());
            }
            if album_id_unresolved {
                mismatches.push("albumDictId".to_string());
            }
            if key_id_unresolved {
                mismatches.push("keyDictId".to_string());
            }
            if artwork_id_unresolved {
                mismatches.push("artworkDictId".to_string());
            }
            if pdb_missing_required {
                let mut pdb_gaps = Vec::<&str>::new();
                if pdb_title.is_empty() {
                    pdb_gaps.push("title");
                }
                if pdb_detail.tempo_x100.is_none() && edb_bpm_x100.is_some() {
                    pdb_gaps.push("bpm");
                }
                if pdb_detail.duration_seconds.is_none() && edb_duration_seconds.is_some() {
                    pdb_gaps.push("duration");
                }
                if pdb_analysis.is_empty() {
                    pdb_gaps.push("analysisPath");
                }
                if pdb_media.is_empty() {
                    pdb_gaps.push("mediaPath");
                }
                if !edb_album.is_empty() && pdb_album.is_empty() {
                    pdb_gaps.push("album");
                }
                if !edb_key.is_empty() && pdb_key.is_empty() {
                    pdb_gaps.push("key");
                }
                mismatches.push(format!("pdbMissing({})", pdb_gaps.join("+")));
            }
            if edb_missing_required {
                let mut edb_gaps = Vec::<&str>::new();
                if edb_title.is_empty() {
                    edb_gaps.push("title");
                }
                if edb_bpm_x100.is_none() && pdb_detail.tempo_x100.is_some() {
                    edb_gaps.push("bpm");
                }
                if edb_duration_seconds.is_none() && pdb_detail.duration_seconds.is_some() {
                    edb_gaps.push("duration");
                }
                if edb_analysis.is_empty() {
                    edb_gaps.push("analysisPath");
                }
                if edb_media.is_empty() {
                    edb_gaps.push("mediaPath");
                }
                if !pdb_album.is_empty() && edb_album.is_empty() {
                    edb_gaps.push("album");
                }
                if !pdb_key.is_empty() && edb_key.is_empty() {
                    edb_gaps.push("key");
                }
                mismatches.push(format!("edbMissing({})", edb_gaps.join("+")));
            }

            if !mismatches.is_empty() {
                let entry = format!(
                    "{} [{}]",
                    sample_track_label(&pdb_key, pdb_detail),
                    mismatches.join(", ")
                );
                // Prioritize gap/dict entries over pure artwork mismatches
                let is_gap = pdb_missing_required
                    || edb_missing_required
                    || artist_id_unresolved
                    || album_id_unresolved
                    || key_id_unresolved
                    || artwork_id_unresolved;
                if is_gap {
                    sample_metadata_mismatches.insert(0, entry);
                } else if sample_metadata_mismatches.len() < 5 {
                    sample_metadata_mismatches.push(entry);
                }
                if sample_metadata_mismatches.len() > 10 {
                    sample_metadata_mismatches.truncate(10);
                }
            }
        }

        let pdb_duplicate_entries = pdb_info
            .as_ref()
            .map(|info| info.duplicate_entries)
            .unwrap_or(0);
        let playlist_id_match = match (pdb_info.as_ref(), edb_info.as_ref()) {
            (Some(pdb), Some(edb)) => edb
                .playlist_id
                .map(|playlist_id| playlist_id == pdb.playlist_id)
                .unwrap_or(false),
            _ => false,
        };

        let structural_pass = only_in_pdb == 0
            && only_in_edb == 0
            && !order_mismatch
            && pdb_duplicate_entries == 0
            && path_mismatch_tracks == 0
            && dictionary_id_issue_tracks == 0
            && playlist_id_match;

        let status = if !structural_pass {
            DiagStatus::Fail
        } else if pdb_missing_core_metadata > 0 || edb_missing_core_metadata > 0 {
            DiagStatus::Warn
        } else {
            DiagStatus::Pass
        };

        details.push(UsbParityPlaylistDetail {
            name: display,
            pdb_tracks: pdb_unique_list.len(),
            edb_tracks: edb_unique_list.len(),
            matched_tracks,
            only_in_pdb,
            only_in_edb,
            order_mismatch,
            parent_match: None,
            pdb_playlist_id: pdb_info.as_ref().map(|info| info.playlist_id),
            edb_playlist_id: edb_info.as_ref().and_then(|info| info.playlist_id),
            pdb_sort_order: pdb_info.as_ref().map(|info| info.sort_order),
            edb_sort_order: edb_info.as_ref().and_then(|info| info.sort_order),
            pdb_duplicate_entries,
            edb_missing_core_metadata,
            pdb_missing_core_metadata,
            artwork_mismatch_tracks,
            path_mismatch_tracks,
            dictionary_id_issue_tracks,
            playlist_id_match,
            sort_order_match: true,
            sample_only_in_pdb,
            sample_only_in_edb,
            sample_metadata_mismatches,
            status,
        });
    }
    // Keep strict parity playlist details in playlist order, not alphabetical:
    // 1) prefer PDB sort order when present (device-facing ordering source)
    // 2) fallback to eDB sort order for eDB-only playlists
    // 3) stable tie-breakers
    details.sort_by(|a, b| {
        let a_bucket = if a.pdb_sort_order.is_some() {
            0u8
        } else if a.edb_sort_order.is_some() {
            1u8
        } else {
            2u8
        };
        let b_bucket = if b.pdb_sort_order.is_some() {
            0u8
        } else if b.edb_sort_order.is_some() {
            1u8
        } else {
            2u8
        };
        a_bucket
            .cmp(&b_bucket)
            .then_with(|| {
                a.pdb_sort_order
                    .unwrap_or(u32::MAX)
                    .cmp(&b.pdb_sort_order.unwrap_or(u32::MAX))
            })
            .then_with(|| {
                a.edb_sort_order
                    .unwrap_or(u32::MAX)
                    .cmp(&b.edb_sort_order.unwrap_or(u32::MAX))
            })
            .then_with(|| {
                a.pdb_playlist_id
                    .unwrap_or(u32::MAX)
                    .cmp(&b.pdb_playlist_id.unwrap_or(u32::MAX))
            })
            .then_with(|| {
                a.edb_playlist_id
                    .unwrap_or(u32::MAX)
                    .cmp(&b.edb_playlist_id.unwrap_or(u32::MAX))
            })
            .then_with(|| a.name.cmp(&b.name))
    });

    let fail_playlists = details
        .iter()
        .filter(|d| matches!(d.status, DiagStatus::Fail))
        .count();
    let total_only_in_pdb: usize = details.iter().map(|d| d.only_in_pdb).sum();
    let total_only_in_edb: usize = details.iter().map(|d| d.only_in_edb).sum();
    let order_mismatches = details.iter().filter(|d| d.order_mismatch).count();
    let total_duplicate_entries: usize = details.iter().map(|d| d.pdb_duplicate_entries).sum();
    let total_pdb_missing_core_metadata: usize =
        details.iter().map(|d| d.pdb_missing_core_metadata).sum();
    let total_edb_missing_core_metadata: usize =
        details.iter().map(|d| d.edb_missing_core_metadata).sum();
    let total_artwork_mismatches: usize = details.iter().map(|d| d.artwork_mismatch_tracks).sum();
    let total_path_mismatches: usize = details.iter().map(|d| d.path_mismatch_tracks).sum();
    let total_dictionary_id_issues: usize =
        details.iter().map(|d| d.dictionary_id_issue_tracks).sum();
    let playlist_id_mismatches = details.iter().filter(|d| !d.playlist_id_match).count();
    let reference_only_field_tracks = reference_only_edb_fields.playlist_linked_tracks;
    let reference_only_field_list = if reference_only_edb_fields.populated_fields.is_empty() {
        "none".to_string()
    } else {
        reference_only_edb_fields.populated_fields.join(", ")
    };

    let warn_playlists = details
        .iter()
        .filter(|d| matches!(d.status, DiagStatus::Warn))
        .count();
    let mut overall_status = if fail_playlists > 0 {
        DiagStatus::Fail
    } else if warn_playlists > 0 {
        DiagStatus::Warn
    } else {
        DiagStatus::Pass
    };
    if let Some(raw_coverage) = strict_raw_coverage.as_ref() {
        overall_status = DiagStatus::worst(&overall_status, &raw_coverage.status);
    }

    let mut checks = Vec::<DiagCheck>::new();
    checks.push(DiagCheck {
        label: "Overall player parity status".to_string(),
        status: overall_status.clone(),
        detail: format!(
            "playlists checked: {}, failing playlists: {}, playlist id mismatches: {}",
            details.len(),
            fail_playlists,
            playlist_id_mismatches,
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Parity-report section (required)".to_string(),
        status: overall_status.clone(),
        detail: "See parity summary rows for category counts.".to_string(),
        link: None,
    });
    let mut summary_rows = vec![
        DiagSummaryRow {
            label: "Failing playlists".to_string(),
            status: if fail_playlists == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: fail_playlists,
        },
        DiagSummaryRow {
            label: "Membership only-in-PDB".to_string(),
            status: if total_only_in_pdb == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_only_in_pdb,
        },
        DiagSummaryRow {
            label: "Membership only-in-eDB".to_string(),
            status: if total_only_in_edb == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_only_in_edb,
        },
        DiagSummaryRow {
            label: "Order mismatches".to_string(),
            status: if order_mismatches == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: order_mismatches,
        },
        DiagSummaryRow {
            label: "Duplicate PDB entries".to_string(),
            status: if total_duplicate_entries == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_duplicate_entries,
        },
        DiagSummaryRow {
            label: "PDB metadata gaps".to_string(),
            status: if total_pdb_missing_core_metadata == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            count: total_pdb_missing_core_metadata,
        },
        DiagSummaryRow {
            label: "eDB source gaps".to_string(),
            status: if total_edb_missing_core_metadata == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            count: total_edb_missing_core_metadata,
        },
        DiagSummaryRow {
            label: "Path mismatches".to_string(),
            status: if total_path_mismatches == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_path_mismatches,
        },
        DiagSummaryRow {
            label: "Artwork presence mismatches".to_string(),
            status: if total_artwork_mismatches == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            count: total_artwork_mismatches,
        },
        DiagSummaryRow {
            label: "Unresolved PDB dictionary ids".to_string(),
            status: if total_dictionary_id_issues == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Fail
            },
            count: total_dictionary_id_issues,
        },
        DiagSummaryRow {
            label: "Reference-only eDB fields".to_string(),
            status: if reference_only_field_tracks == 0 {
                DiagStatus::Pass
            } else {
                DiagStatus::Warn
            },
            count: reference_only_field_tracks,
        },
    ];
    if let Some(raw_coverage) = strict_raw_coverage.as_ref() {
        summary_rows.push(DiagSummaryRow {
            label: "Indexed audio file presence".to_string(),
            status: raw_coverage.status.clone(),
            count: raw_coverage.missing_count + raw_coverage.extra_count,
        });
    }
    checks.push(DiagCheck {
        label: "Playlist identity parity".to_string(),
        status: if playlist_id_mismatches == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "{} playlist(s) have mismatched or unresolved playlist ids between PDB and eDB",
            playlist_id_mismatches
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Playlist membership parity".to_string(),
        status: if total_only_in_pdb == 0 && total_only_in_edb == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "membership deltas: only-in-PDB={}, only-in-eDB={}",
            total_only_in_pdb, total_only_in_edb
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Playlist ordering parity".to_string(),
        status: if order_mismatches == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!("entry-order mismatches={}", order_mismatches),
        link: None,
    });
    if let Some(raw_coverage) = strict_raw_coverage.as_ref() {
        checks.push(DiagCheck {
            label: "Indexed audio file presence".to_string(),
            status: raw_coverage.status.clone(),
            detail: raw_coverage.detail.clone(),
            link: None,
        });
    }
    checks.push(DiagCheck {
        label: "Duplicate PDB entries".to_string(),
        status: if total_duplicate_entries == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "{} duplicate PDB playlist entry/entries detected",
            total_duplicate_entries
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "PDB metadata completeness".to_string(),
        status: if total_pdb_missing_core_metadata == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        detail: format!(
            "{} playlist-linked PDB track(s) are missing required player metadata",
            total_pdb_missing_core_metadata
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Media and analysis path parity".to_string(),
        status: if total_path_mismatches == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "{} playlist-linked track(s) have media or analysis path mismatches",
            total_path_mismatches
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Artwork presence parity".to_string(),
        status: if total_artwork_mismatches == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        detail: format!(
            "{} playlist-linked track(s) have artwork in one DB but not the other",
            total_artwork_mismatches
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "PDB dictionary id resolution".to_string(),
        status: if total_dictionary_id_issues == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Fail
        },
        detail: format!(
            "{} playlist-linked track(s) have unresolved required PDB dictionary ids",
            total_dictionary_id_issues
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "eDB source completeness".to_string(),
        status: if total_edb_missing_core_metadata == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        detail: format!(
            "{} playlist-linked eDB track(s) are missing metadata used by strict parity comparison",
            total_edb_missing_core_metadata
        ),
        link: None,
    });
    checks.push(DiagCheck {
        label: "Reference-documented field coverage".to_string(),
        status: if reference_only_field_tracks == 0 {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        detail: if reference_only_field_tracks == 0 {
            "No playlist-linked eDB rows use documented reference fields outside current PDB/parity coverage.".to_string()
        } else {
            format!(
                "{} playlist-linked eDB track(s) use documented reference fields not yet verified in PDB/parity: {}",
                reference_only_field_tracks, reference_only_field_list
            )
        },
        link: None,
    });

    let overall = DiagStatus::worst_of(&checks.iter().map(|c| &c.status).collect::<Vec<_>>());
    (checks, summary_rows, details, overall)
}

pub(crate) fn track_identity_key(
    file_path: &str,
    title: &str,
    artist: &str,
    id_fallback: Option<&str>,
) -> String {
    let path = normalize_track_path_for_identity(file_path);
    if !path.is_empty() {
        return format!("path:{path}");
    }

    let meta = build_meta_key(title, artist);
    if meta != "meta:|" {
        return meta;
    }

    if let Some(id) = id_fallback.map(str::trim).filter(|id| !id.is_empty()) {
        return format!("id:{id}");
    }

    "unknown".to_string()
}

/// Build a meta-key string from title and artist (same format as track_identity_key fallback).
pub(crate) fn build_meta_key(title: &str, artist: &str) -> String {
    format!(
        "meta:{}|{}",
        canonicalize_playlist_name(title),
        canonicalize_playlist_name(artist)
    )
}

fn normalize_contents_path(value: &str, lowercase_output: bool) -> String {
    let normalized = repair_utf8_mojibake(value.trim()).replace('\\', "/");
    if normalized.is_empty() {
        return String::new();
    }
    let lowered = normalized.to_ascii_lowercase();

    let candidate = if let Some(idx) = lowered.rfind("/contents/") {
        normalized[idx..].to_string()
    } else if lowered.starts_with("contents/") {
        format!("/{normalized}")
    } else {
        normalized
    };

    if lowercase_output {
        candidate.to_ascii_lowercase()
    } else {
        candidate
    }
}

pub(crate) fn normalize_track_path_for_identity(value: &str) -> String {
    normalize_contents_path(value, true)
}

pub(crate) fn normalize_analysis_path_for_identity(value: &str) -> String {
    let normalized = repair_utf8_mojibake(value.trim()).replace('\\', "/");
    if normalized.is_empty() {
        return String::new();
    }
    canonicalize_playlist_name(&normalized)
}

fn unique_preserve_order(input: &[String]) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::<String>::new();
    for item in input {
        if seen.insert(item.clone()) {
            out.push(item.clone());
        }
    }
    out
}

pub(crate) fn normalize_pdb_path_for_edb_lookup(value: &str) -> String {
    normalize_contents_path(value, false)
}

pub(crate) fn normalize_path_for_contents_match(value: &str) -> String {
    let normalized = normalize_pdb_path_for_edb_lookup(value);
    if normalized.is_empty() {
        return normalized;
    }
    let path = std::path::Path::new(&normalized);
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let alias_stripped = if let Some((stem, ext)) = file.rsplit_once('.') {
        // Trim trailing whitespace: some DJ software appends a space before the
        // extension as a naming-conflict alias (e.g. "Track .mp3"). Both the
        // alias form and the original may exist on disk, so normalise both to
        // the same shape for comparison.
        let stem = stem.trim_end();
        let next_stem = stem
            .rsplit_once('-')
            .and_then(|(prefix, suffix)| {
                if !prefix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
                    Some(prefix.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| stem.to_string());
        format!("{next_stem}.{ext}")
    } else {
        file.to_string()
    };
    if parent.is_empty() {
        alias_stripped
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), alias_stripped)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;

    use super::{
        ExportDbPlaylist, ReferenceOnlyEdbFieldUsage, build_usb_parity_comparison,
        diagnose_contents_integrity, diagnose_playlist_resolution_with_db,
        diagnose_playlist_resolution_with_edb_internal, evaluate_strict_raw_coverage_parity,
        normalize_pdb_path_for_edb_lookup, normalize_track_path_for_identity, track_identity_key,
    };
    use crate::models::{DiagStatus, UsbTrack};
    use crate::pdb_reader::{ParsedPdb, PdbPlaylistEntryRow, PdbPlaylistTreeRow, PdbTrackRow};
    use tempfile::tempdir;

    #[test]
    fn track_identity_prefers_contents_path_for_absolute_usb_path() {
        let key = track_identity_key(
            "/tmp/workspace/USB/Contents/Artist/Track.mp3",
            "Track",
            "Artist",
            Some("2002"),
        );
        let expected = format!(
            "path:{}",
            normalize_track_path_for_identity("/Contents/Artist/Track.mp3")
        );
        assert_eq!(key, expected);
    }

    #[test]
    fn track_identity_path_matches_relative_contents_path() {
        let abs = normalize_track_path_for_identity("/tmp/workspace/USB/Contents/Artist/Track.mp3");
        let rel = normalize_track_path_for_identity("/Contents/Artist/Track.mp3");
        assert_eq!(abs, rel);
    }

    #[test]
    fn normalize_pdb_path_extracts_contents_segment() {
        let value = "/mnt/Somewhere/USB/Contents/Artist/Album/Track.mp3";
        assert_eq!(
            normalize_pdb_path_for_edb_lookup(value),
            "/Contents/Artist/Album/Track.mp3"
        );
    }

    #[test]
    fn contents_integrity_treats_unindexed_audio_as_warning() {
        let section = diagnose_contents_integrity(12, 0);
        let count_match = section
            .checks
            .iter()
            .find(|check| check.label == "Count match")
            .expect("count match check");
        assert!(matches!(count_match.status, DiagStatus::Warn));
        let counts = section.counts.expect("contents counts");
        assert_eq!(counts.contents_count, 12);
        assert_eq!(counts.indexed_count, 0);
        assert_eq!(counts.mismatch_count, 12);
    }

    fn make_usb_track(id: &str, title: &str, artist: &str, file_path: &str) -> UsbTrack {
        UsbTrack {
            id: id.to_string(),
            local_track_id: None,
            title: title.to_string(),
            artist: artist.to_string(),
            album: None,
            track_number: None,
            bpm: None,
            key: None,
            file_path: file_path.to_string(),
            usb_media_path: None,
            artwork_path: None,
            artwork_data_url: None,
            waveform_peaks_path: None,
            usb_analysis_path: None,
            usb_analysis_path_raw: None,
            waveform_preview: None,
            duration_ms: None,
        }
    }

    fn make_pdb_track(id: u32, title: &str, artist_id: u32, file_path: &str) -> PdbTrackRow {
        PdbTrackRow {
            content_link: None,
            sample_rate_hz: None,
            file_size_bytes: None,
            master_content_id: None,
            master_db_id: None,
            id,
            artist_id,
            album_id: 0,
            artwork_id: 0,
            key_id: 0,
            genre_id: 0,
            bitrate_kbps: None,
            track_number: id,
            tempo_x100: 12000,
            release_year: None,
            bit_depth: None,
            duration_seconds: Some(180),
            file_type: None,
            isrc: None,
            date_added: None,
            release_date: None,
            dj_comment: None,
            file_name: None,
            publish_track_info: None,
            autoload_hotcues: None,
            title: title.to_string(),
            anlz_path: format!("/PIONEER/USBANLZ/track-{id}.DAT"),
            track_file_path: file_path.to_string(),
        }
    }

    fn make_single_playlist_parsed(name: &str, playlist_id: u32, sort_order: u32) -> ParsedPdb {
        let mut parsed = ParsedPdb::default();
        parsed.artists.insert(1, "Artist".to_string());
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: playlist_id,
            parent_id: 0,
            sort_order,
            row_is_folder: false,
            name: name.to_string(),
        });
        parsed
            .tracks
            .push(make_pdb_track(1, "Track", 1, "/Contents/Artist/Track.mp3"));
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: 0,
            track_id: 1,
            playlist_id,
        });
        parsed
    }

    fn edb_playlists_from_tracks(
        playlists: HashMap<String, Vec<UsbTrack>>,
    ) -> HashMap<String, ExportDbPlaylist> {
        playlists
            .into_iter()
            .enumerate()
            .map(|(idx, (name, tracks))| {
                (
                    name,
                    ExportDbPlaylist {
                        playlist_id: i64::try_from(idx + 1).expect("playlist id"),
                        sort_order: i64::try_from(idx + 1).expect("sort order"),
                        tracks,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn scan_reference_only_edb_field_usage_handles_documented_and_ignored_fields() {
        for (column, value, expected_tracks, expected_fields) in [
            ("rating", "5", 1, vec!["rating".to_string()]),
            ("djPlayCount", "12", 0, Vec::new()),
        ] {
            let root = tempdir().expect("tempdir");
            let export_db_dir = root.path().join("PIONEER").join("rekordbox");
            fs::create_dir_all(&export_db_dir).expect("create export db dir");
            let edb_path = export_db_dir.join("exportLibrary.db");
            let conn = rusqlite::Connection::open(&edb_path).expect("open export db");
            conn.execute_batch(&format!(
                r#"
                CREATE TABLE content (
                  content_id INTEGER PRIMARY KEY,
                  title TEXT,
                  {column} INTEGER
                );
                CREATE TABLE playlist_content (
                  playlist_id INTEGER,
                  content_id INTEGER,
                  sequenceNo INTEGER
                );
                INSERT INTO content (content_id, title, {column}) VALUES (1, 'A', {value});
                INSERT INTO playlist_content (playlist_id, content_id, sequenceNo) VALUES (1, 1, 0);
                "#
            ))
            .expect("seed export db");
            drop(conn);

            let mut warnings = Vec::new();
            let usage = super::scan_reference_only_edb_field_usage(root.path(), &mut warnings);
            assert_eq!(
                usage.playlist_linked_tracks, expected_tracks,
                "column {column}"
            );
            assert_eq!(usage.populated_fields, expected_fields, "column {column}");
        }
    }

    #[test]
    fn parity_comparison_warns_when_reference_only_edb_fields_are_present() {
        let mut parsed = make_single_playlist_parsed("Test", 1, 1);
        // Clear tempo/duration/anlz so those fields don't produce additional parity failures,
        // leaving only the reference-only check as the sole non-Pass result.
        for track in &mut parsed.tracks {
            track.tempo_x100 = 0;
            track.duration_seconds = None;
            track.anlz_path = String::new();
        }
        let edb_tracks = HashMap::from([(
            "Test".to_string(),
            vec![make_usb_track(
                "usb-track-1",
                "Track",
                "Artist",
                "/Contents/Artist/Track.mp3",
            )],
        )]);
        let reference_only = ReferenceOnlyEdbFieldUsage {
            playlist_linked_tracks: 1,
            populated_fields: vec!["titleForSearch".to_string()],
        };

        let (checks, summary_rows, _details, overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &reference_only,
            None,
        );

        let coverage = checks
            .iter()
            .find(|check| check.label == "Reference-documented field coverage")
            .expect("reference-documented field coverage check");
        assert!(matches!(coverage.status, DiagStatus::Warn));
        assert!(coverage.detail.contains("titleForSearch"));

        let summary = summary_rows
            .iter()
            .find(|row| row.label == "Reference-only eDB fields")
            .expect("reference-only summary row");
        assert!(matches!(summary.status, DiagStatus::Warn));
        assert_eq!(summary.count, 1);
        assert!(matches!(overall, DiagStatus::Warn));
    }

    #[test]
    fn parity_counts_use_unique_tracks_when_pdb_has_duplicate_entries() {
        let mut parsed = ParsedPdb::default();
        parsed.tracks = vec![
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(1, "A", 1, "/Contents/A.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(2, "B", 1, "/Contents/B.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(3, "C", 1, "/Contents/C.mp3")
            },
        ];
        parsed.artists.insert(1, "Artist".to_string());
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 10,
            parent_id: 0,
            sort_order: 1,
            row_is_folder: false,
            name: "Test".to_string(),
        });
        // Duplicate entries inflate raw count to 12, but unique set is 3.
        for i in 0..4u32 {
            parsed.playlist_entries.push(PdbPlaylistEntryRow {
                entry_index: i * 3,
                track_id: 1,
                playlist_id: 10,
            });
            parsed.playlist_entries.push(PdbPlaylistEntryRow {
                entry_index: i * 3 + 1,
                track_id: 2,
                playlist_id: 10,
            });
            parsed.playlist_entries.push(PdbPlaylistEntryRow {
                entry_index: i * 3 + 2,
                track_id: 3,
                playlist_id: 10,
            });
        }

        let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
        edb_tracks.insert(
            "Test".to_string(),
            vec![
                make_usb_track("1", "A", "Artist", "/Contents/A.mp3"),
                make_usb_track("2", "B", "Artist", "/Contents/B.mp3"),
                make_usb_track("3", "C", "Artist", "/Contents/C.mp3"),
            ],
        );

        let (_, _, details, _) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &ReferenceOnlyEdbFieldUsage::default(),
            None,
        );
        let test = details
            .iter()
            .find(|d| d.name == "Test")
            .expect("test detail");
        assert_eq!(test.pdb_tracks, 3);
        assert_eq!(test.edb_tracks, 3);
        assert_eq!(test.matched_tracks, 3);
        assert_eq!(test.only_in_pdb, 0);
        assert_eq!(test.only_in_edb, 0);
    }

    #[test]
    fn strict_raw_coverage_thresholds_transition_pass_warn_fail() {
        let pass = evaluate_strict_raw_coverage_parity(0, 0, 100);
        assert!(matches!(pass.status, DiagStatus::Pass));
        assert_eq!(pass.missing_count, 0);
        assert_eq!(pass.extra_count, 0);
        assert!(pass.detail.contains("all 100 indexed"));

        let warn_missing = evaluate_strict_raw_coverage_parity(3, 0, 100);
        assert!(matches!(warn_missing.status, DiagStatus::Warn));
        assert_eq!(warn_missing.missing_count, 3);

        let warn_extra = evaluate_strict_raw_coverage_parity(0, 3, 100);
        assert!(matches!(warn_extra.status, DiagStatus::Warn));
        assert_eq!(warn_extra.extra_count, 3);

        let fail_missing = evaluate_strict_raw_coverage_parity(20, 0, 100);
        assert!(matches!(fail_missing.status, DiagStatus::Fail));
        assert_eq!(fail_missing.missing_count, 20);
    }

    #[test]
    fn parity_comparison_reports_strict_raw_coverage_as_separate_check_and_row() {
        let parsed = make_single_playlist_parsed("RawCoverage", 1, 1);
        let edb_tracks = HashMap::from([(
            "RawCoverage".to_string(),
            vec![make_usb_track(
                "1",
                "Track",
                "Artist",
                "/Contents/Artist/Track.mp3",
            )],
        )]);
        let raw_coverage = evaluate_strict_raw_coverage_parity(9, 0, 100);

        let (checks, summary_rows, _details, overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &ReferenceOnlyEdbFieldUsage::default(),
            Some(raw_coverage),
        );

        let raw_check = checks
            .iter()
            .find(|check| check.label == "Indexed audio file presence")
            .expect("indexed audio file presence check");
        assert!(matches!(raw_check.status, DiagStatus::Fail));
        assert!(raw_check.detail.contains("missing from USB"));

        let raw_summary = summary_rows
            .iter()
            .find(|row| row.label == "Indexed audio file presence")
            .expect("indexed audio file presence summary row");
        assert!(matches!(raw_summary.status, DiagStatus::Fail));
        assert_eq!(raw_summary.count, 9);
        assert!(matches!(overall, DiagStatus::Fail));
    }

    #[test]
    fn parity_report_fails_when_playlist_is_missing_from_one_database() {
        for missing_from_pdb in [true, false] {
            let name = if missing_from_pdb {
                "Missing From PDB"
            } else {
                "Missing From eDB"
            };
            let (parsed, edb_tracks) = if missing_from_pdb {
                let edb_tracks = HashMap::from([(
                    name.to_string(),
                    vec![make_usb_track(
                        "1",
                        "Track",
                        "Artist",
                        "/Contents/Artist/Track.mp3",
                    )],
                )]);
                (ParsedPdb::default(), edb_tracks)
            } else {
                (
                    make_single_playlist_parsed(name, 10, 1),
                    HashMap::<String, Vec<UsbTrack>>::new(),
                )
            };

            let (checks, _, details, overall) = build_usb_parity_comparison(
                &parsed,
                &edb_playlists_from_tracks(edb_tracks),
                &ReferenceOnlyEdbFieldUsage::default(),
                None,
            );
            let playlist = details
                .iter()
                .find(|d| d.name == name)
                .expect("playlist detail");
            assert!(matches!(overall, DiagStatus::Fail), "{name}");
            assert_eq!(
                playlist.pdb_tracks,
                if missing_from_pdb { 0 } else { 1 },
                "{name}"
            );
            assert_eq!(
                playlist.edb_tracks,
                if missing_from_pdb { 1 } else { 0 },
                "{name}"
            );
            assert_eq!(
                playlist.only_in_pdb,
                if missing_from_pdb { 0 } else { 1 },
                "{name}"
            );
            assert_eq!(
                playlist.only_in_edb,
                if missing_from_pdb { 1 } else { 0 },
                "{name}"
            );
            assert!(matches!(playlist.status, DiagStatus::Fail), "{name}");
            let membership = checks
                .iter()
                .find(|c| c.label == "Playlist membership parity")
                .expect("membership parity check");
            assert!(matches!(membership.status, DiagStatus::Fail), "{name}");
            let expected_detail = if missing_from_pdb {
                "only-in-eDB=1"
            } else {
                "only-in-PDB=1"
            };
            assert!(
                membership.detail.contains(expected_detail),
                "{name}: {:?}",
                membership
            );
        }
    }

    #[test]
    fn parity_report_fails_when_playlist_order_differs() {
        let mut parsed = ParsedPdb::default();
        parsed.artists.insert(1, "Artist".to_string());
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 10,
            parent_id: 0,
            sort_order: 1,
            row_is_folder: false,
            name: "Order".to_string(),
        });
        parsed.tracks = vec![
            make_pdb_track(1, "A", 1, "/Contents/Artist/A.mp3"),
            make_pdb_track(2, "B", 1, "/Contents/Artist/B.mp3"),
        ];
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: 0,
            track_id: 1,
            playlist_id: 10,
        });
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: 1,
            track_id: 2,
            playlist_id: 10,
        });

        let mut first = make_usb_track("1", "A", "Artist", "/Contents/Artist/A.mp3");
        first.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
        first.bpm = Some(120.0);
        first.duration_ms = Some(180_000);
        first.track_number = Some(1);
        let mut second = make_usb_track("2", "B", "Artist", "/Contents/Artist/B.mp3");
        second.usb_analysis_path = Some("/PIONEER/USBANLZ/track-2.DAT".to_string());
        second.bpm = Some(120.0);
        second.duration_ms = Some(180_000);
        second.track_number = Some(2);

        let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
        edb_tracks.insert("Order".to_string(), vec![second, first]);

        let (checks, _, details, overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &ReferenceOnlyEdbFieldUsage::default(),
            None,
        );
        let playlist = details
            .iter()
            .find(|d| d.name == "Order")
            .expect("order detail");
        assert!(matches!(overall, DiagStatus::Fail));
        assert!(playlist.order_mismatch);
        assert!(matches!(playlist.status, DiagStatus::Fail));
        let ordering = checks
            .iter()
            .find(|c| c.label == "Playlist ordering parity")
            .expect("ordering check");
        assert!(matches!(ordering.status, DiagStatus::Fail));
        assert!(ordering.detail.contains("entry-order mismatches=1"));
    }

    #[test]
    fn parity_playlist_details_follow_playlist_sort_order_not_name() {
        let mut parsed = ParsedPdb::default();
        parsed.artists.insert(1, "Artist".to_string());
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 10,
            parent_id: 0,
            sort_order: 1,
            row_is_folder: false,
            name: "Zeta".to_string(),
        });
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 20,
            parent_id: 0,
            sort_order: 2,
            row_is_folder: false,
            name: "Alpha".to_string(),
        });
        parsed.tracks = vec![
            make_pdb_track(1, "Track Z", 1, "/Contents/Artist/Z.mp3"),
            make_pdb_track(2, "Track A", 1, "/Contents/Artist/A.mp3"),
        ];
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: 0,
            track_id: 1,
            playlist_id: 10,
        });
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: 0,
            track_id: 2,
            playlist_id: 20,
        });

        let mut edb_playlists = HashMap::<String, ExportDbPlaylist>::new();
        edb_playlists.insert(
            "Zeta".to_string(),
            ExportDbPlaylist {
                playlist_id: 10,
                sort_order: 1,
                tracks: vec![make_usb_track(
                    "1",
                    "Track Z",
                    "Artist",
                    "/Contents/Artist/Z.mp3",
                )],
            },
        );
        edb_playlists.insert(
            "Alpha".to_string(),
            ExportDbPlaylist {
                playlist_id: 20,
                sort_order: 2,
                tracks: vec![make_usb_track(
                    "2",
                    "Track A",
                    "Artist",
                    "/Contents/Artist/A.mp3",
                )],
            },
        );

        let (_checks, _rows, details, _overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists,
            &ReferenceOnlyEdbFieldUsage::default(),
            None,
        );

        assert_eq!(details.len(), 2);
        assert_eq!(details[0].name, "Zeta");
        assert_eq!(details[1].name, "Alpha");
    }

    #[test]
    fn parity_report_fails_when_media_or_analysis_path_differs_between_edb_and_pdb() {
        for analysis_path_case in [false, true] {
            let name = if analysis_path_case {
                "Analysis Paths"
            } else {
                "Paths"
            };
            let mut parsed = make_single_playlist_parsed(name, 10, 1);
            let mut track = if analysis_path_case {
                make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3")
            } else {
                parsed.tracks[0].track_file_path = "/Contents/Artist/DIFFERENT.mp3".to_string();
                make_usb_track("1", "Track", "Artist", "/contents/artist/track.mp3")
            };
            track.usb_analysis_path = Some(if analysis_path_case {
                "/PIONEER/USBANLZ/track-1-DIFFERENT.DAT".to_string()
            } else {
                "/PIONEER/USBANLZ/track-1.DAT".to_string()
            });
            track.bpm = Some(120.0);
            track.duration_ms = Some(180_000);
            track.track_number = Some(1);

            let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
            edb_tracks.insert(name.to_string(), vec![track]);

            let (checks, _, details, overall) = build_usb_parity_comparison(
                &parsed,
                &edb_playlists_from_tracks(edb_tracks),
                &ReferenceOnlyEdbFieldUsage::default(),
                None,
            );
            let playlist = details
                .iter()
                .find(|d| d.name == name)
                .expect("paths detail");
            assert!(matches!(overall, DiagStatus::Fail), "{name}");
            assert_eq!(playlist.path_mismatch_tracks, 1, "{name}");
            assert!(matches!(playlist.status, DiagStatus::Fail), "{name}");
            if analysis_path_case {
                assert!(
                    playlist
                        .sample_metadata_mismatches
                        .iter()
                        .any(|m| m.contains("analysisPath")),
                    "{playlist:?}"
                );
            }
            let paths = checks
                .iter()
                .find(|c| c.label == "Media and analysis path parity")
                .expect("path parity check");
            assert!(matches!(paths.status, DiagStatus::Fail), "{name}");
        }
    }

    #[test]
    fn parity_report_passes_when_both_have_artwork_with_different_paths() {
        let mut parsed = make_single_playlist_parsed("Artwork", 1, 1);
        parsed.tracks[0].artwork_id = 7;
        parsed
            .artworks
            .insert(7, "/PIONEER/Artwork/cover.jpg".to_string());

        let mut track = make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3");
        track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
        track.artwork_path = Some("/PIONEER/Artwork/other.jpg".to_string());
        track.bpm = Some(120.0);
        track.duration_ms = Some(180_000);
        track.track_number = Some(1);

        let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
        edb_tracks.insert("Artwork".to_string(), vec![track]);

        let (checks, _, details, overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &ReferenceOnlyEdbFieldUsage::default(),
            None,
        );
        let playlist = details
            .iter()
            .find(|d| d.name == "Artwork")
            .expect("artwork detail");
        assert!(matches!(overall, DiagStatus::Pass));
        // Both have artwork (different paths) — no presence mismatch
        assert_eq!(playlist.artwork_mismatch_tracks, 0);
        assert!(matches!(playlist.status, DiagStatus::Pass));
        let artwork = checks
            .iter()
            .find(|c| c.label == "Artwork presence parity")
            .expect("artwork parity check");
        assert!(matches!(artwork.status, DiagStatus::Pass));
    }

    #[test]
    fn parity_report_warns_when_artwork_present_in_one_db_only() {
        // PDB has artwork, eDB does not
        let mut parsed = make_single_playlist_parsed("ArtworkGap", 1, 1);
        parsed.tracks[0].artwork_id = 7;
        parsed
            .artworks
            .insert(7, "/PIONEER/Artwork/cover.jpg".to_string());

        let mut track = make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3");
        track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
        track.artwork_path = None;
        track.bpm = Some(120.0);
        track.duration_ms = Some(180_000);
        track.track_number = Some(1);

        let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
        edb_tracks.insert("ArtworkGap".to_string(), vec![track]);

        let (checks, _, details, _overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &ReferenceOnlyEdbFieldUsage::default(),
            None,
        );
        let playlist = details
            .iter()
            .find(|d| d.name == "ArtworkGap")
            .expect("artwork detail");
        assert_eq!(playlist.artwork_mismatch_tracks, 1);
        let artwork = checks
            .iter()
            .find(|c| c.label == "Artwork presence parity")
            .expect("artwork parity check");
        assert!(matches!(artwork.status, DiagStatus::Warn));
    }

    #[test]
    fn parity_report_fails_when_required_pdb_dictionary_ids_do_not_resolve() {
        let mut parsed = make_single_playlist_parsed("Dictionary IDs", 10, 1);
        parsed.tracks[0].artist_id = 99;

        let mut track = make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3");
        track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
        track.bpm = Some(120.0);
        track.duration_ms = Some(180_000);
        track.track_number = Some(1);

        let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
        edb_tracks.insert("Dictionary IDs".to_string(), vec![track]);

        let (checks, _, details, overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &ReferenceOnlyEdbFieldUsage::default(),
            None,
        );
        let playlist = details
            .iter()
            .find(|d| d.name == "Dictionary IDs")
            .expect("dictionary detail");
        assert!(matches!(overall, DiagStatus::Fail));
        assert_eq!(playlist.dictionary_id_issue_tracks, 1);
        assert!(
            playlist
                .sample_metadata_mismatches
                .iter()
                .any(|m| m.contains("artistDictId"))
        );
        let dictionaries = checks
            .iter()
            .find(|c| c.label == "PDB dictionary id resolution")
            .expect("dictionary id resolution check");
        assert!(matches!(dictionaries.status, DiagStatus::Fail));
    }

    #[test]
    fn parity_report_warns_when_edb_source_metadata_is_incomplete() {
        let mut parsed = make_single_playlist_parsed("Thin eDB", 10, 1);
        parsed.tracks[0].album_id = 7;
        parsed.tracks[0].key_id = 8;
        parsed.albums.insert(7, "Album".to_string());
        parsed.keys.insert(8, "8A".to_string());

        let mut track = make_usb_track("1", "Track", "Artist", "/Contents/Artist/Track.mp3");
        track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
        track.bpm = Some(120.0);
        track.duration_ms = Some(180_000);
        track.track_number = Some(1);

        let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
        edb_tracks.insert("Thin eDB".to_string(), vec![track]);

        let (checks, _, details, overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &ReferenceOnlyEdbFieldUsage::default(),
            None,
        );
        let playlist = details
            .iter()
            .find(|d| d.name == "Thin eDB")
            .expect("thin edb detail");
        let edb_source = checks
            .iter()
            .find(|c| c.label == "eDB source completeness")
            .expect("eDB source completeness check");
        assert!(matches!(edb_source.status, DiagStatus::Warn));
        assert!(matches!(overall, DiagStatus::Fail));
        assert_eq!(playlist.edb_missing_core_metadata, 1);
        assert!(matches!(playlist.status, DiagStatus::Fail));
    }

    #[test]
    fn parity_report_treats_empty_artist_as_unknown_artist_for_gate() {
        let mut parsed = ParsedPdb::default();
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 1,
            parent_id: 0,
            sort_order: 1,
            row_is_folder: false,
            name: "Unknown Artist Gate".to_string(),
        });
        parsed
            .tracks
            .push(make_pdb_track(1, "Track", 0, "/Contents/Unknown/Track.mp3"));
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: 0,
            track_id: 1,
            playlist_id: 1,
        });

        let mut track = make_usb_track("1", "Track", "", "/Contents/Unknown/Track.mp3");
        track.usb_analysis_path = Some("/PIONEER/USBANLZ/track-1.DAT".to_string());
        track.bpm = Some(120.0);
        track.duration_ms = Some(180_000);
        track.track_number = Some(1);

        let mut edb_tracks = HashMap::<String, Vec<UsbTrack>>::new();
        edb_tracks.insert("Unknown Artist Gate".to_string(), vec![track]);

        let (_, _, details, overall) = build_usb_parity_comparison(
            &parsed,
            &edb_playlists_from_tracks(edb_tracks),
            &ReferenceOnlyEdbFieldUsage::default(),
            None,
        );
        let playlist = details
            .iter()
            .find(|d| d.name == "Unknown Artist Gate")
            .expect("unknown artist gate detail");

        assert!(matches!(overall, DiagStatus::Pass));
        assert_eq!(playlist.pdb_missing_core_metadata, 0);
        assert_eq!(playlist.edb_missing_core_metadata, 0);
        assert!(matches!(playlist.status, DiagStatus::Pass));
    }

    #[test]
    fn playlist_resolution_uses_unique_entries_when_duplicates_exist() {
        let mut parsed = ParsedPdb::default();
        parsed.tracks = vec![
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(1, "A", 1, "/Contents/A.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(2, "B", 1, "/Contents/B.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(3, "C", 1, "/Contents/C.mp3")
            },
        ];
        parsed.artists.insert(1, "Artist".to_string());
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 10,
            parent_id: 0,
            sort_order: 1,
            row_is_folder: false,
            name: "Test".to_string(),
        });
        for i in 0..4u32 {
            parsed.playlist_entries.push(PdbPlaylistEntryRow {
                entry_index: i * 3,
                track_id: 1,
                playlist_id: 10,
            });
            parsed.playlist_entries.push(PdbPlaylistEntryRow {
                entry_index: i * 3 + 1,
                track_id: 2,
                playlist_id: 10,
            });
            parsed.playlist_entries.push(PdbPlaylistEntryRow {
                entry_index: i * 3 + 2,
                track_id: 3,
                playlist_id: 10,
            });
        }

        let (_section, details) = diagnose_playlist_resolution_with_db(Some(&parsed), None);
        let test = details
            .iter()
            .find(|d| d.name == "Test")
            .expect("test detail");
        assert_eq!(test.total_entries, 3);
        assert_eq!(test.resolved_entries, 3);
        assert!((test.resolution_rate - 1.0).abs() < 0.0001);
    }

    #[test]
    fn playlist_resolution_collapses_duplicate_leaf_names() {
        let mut parsed = ParsedPdb::default();
        parsed.tracks = vec![
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(1, "A", 1, "/Contents/A.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(2, "B", 1, "/Contents/B.mp3")
            },
            PdbTrackRow {
                tempo_x100: 0,
                duration_seconds: None,
                anlz_path: String::new(),
                ..make_pdb_track(3, "C", 1, "/Contents/C.mp3")
            },
        ];
        parsed.artists.insert(1, "Artist".to_string());
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 10,
            parent_id: 0,
            sort_order: 1,
            row_is_folder: false,
            name: "Dup".to_string(),
        });
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 11,
            parent_id: 0,
            sort_order: 2,
            row_is_folder: false,
            name: "Dup".to_string(),
        });
        parsed.playlist_tree.push(PdbPlaylistTreeRow {
            id: 12,
            parent_id: 0,
            sort_order: 3,
            row_is_folder: false,
            name: "Dup".to_string(),
        });
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: 0,
            track_id: 1,
            playlist_id: 10,
        });
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: 1,
            track_id: 2,
            playlist_id: 11,
        });
        parsed.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: 2,
            track_id: 3,
            playlist_id: 12,
        });

        let (_section, details) = diagnose_playlist_resolution_with_db(Some(&parsed), None);
        let dup_entries = details.iter().filter(|d| d.name == "Dup").count();
        assert_eq!(dup_entries, 1, "duplicate playlist names must collapse");
        let dup = details
            .iter()
            .find(|d| d.name == "Dup")
            .expect("dup detail");
        assert_eq!(dup.total_entries, 3);
        assert_eq!(dup.resolved_entries, 3);
    }

    #[test]
    fn playlist_resolution_passes_for_fully_resolved_playlist_without_edb_match() {
        let parsed = make_single_playlist_parsed("PDB Only", 10, 1);

        let (section, details) =
            diagnose_playlist_resolution_with_db(Some(&parsed), Some(&HashMap::new()));
        let playlist = details
            .iter()
            .find(|d| d.name == "PDB Only")
            .expect("pdb only detail");
        assert!(matches!(section.status, DiagStatus::Pass));
        assert_eq!(playlist.total_entries, 1);
        assert_eq!(playlist.resolved_entries, 1);
        assert_eq!(playlist.pdb_entries, 1);
        assert_eq!(playlist.edb_entries, 0);
        assert_eq!(playlist.matched_entries, 0);
        assert!(matches!(playlist.status, DiagStatus::Pass));
        let overlap = section
            .checks
            .iter()
            .find(|c| c.label == "PDB vs eDB key overlap (informational)")
            .expect("overlap check");
        assert!(overlap.detail.contains("PDB 0.0% (0/1)"));
    }

    #[test]
    fn playlist_resolution_summary_remains_operational_not_player_validation_wording() {
        let parsed = make_single_playlist_parsed("Operational", 10, 1);

        let (section, _details) =
            diagnose_playlist_resolution_with_db(Some(&parsed), Some(&HashMap::new()));
        let overall = section
            .checks
            .iter()
            .find(|c| c.label == "Overall resolution")
            .expect("overall resolution check");
        assert!(overall.detail.contains("entries resolve"));
        assert!(
            !overall
                .detail
                .to_ascii_lowercase()
                .contains("player parity")
        );
        assert!(!overall.detail.to_ascii_lowercase().contains("validation"));
        let interpretation = section
            .checks
            .iter()
            .find(|c| c.label == "Operational interpretation")
            .expect("operational interpretation check");
        assert!(matches!(interpretation.status, DiagStatus::Pass));
        assert!(
            interpretation
                .detail
                .contains("strict parity may still fail")
        );
        assert!(interpretation.detail.contains("operationally usable"));
    }

    #[test]
    fn playlist_resolution_emits_stage_and_per_playlist_progress_messages() {
        let parsed = make_single_playlist_parsed("Progress", 10, 1);
        let mut progress = Vec::<String>::new();

        let (_section, _details) = diagnose_playlist_resolution_with_edb_internal(
            Some(&parsed),
            Some(&HashMap::new()),
            |done, total, message| {
                progress.push(format!("{done}/{total} {message}"));
            },
        );

        assert_eq!(progress.len(), 1);
        assert!(progress[0].contains("1/1"));
        assert!(progress[0].contains("Resolving playlist 1/1: Progress"));
    }

    // ── Identity-key normalization tests ─────────────────────────────

    #[test]
    fn normalize_track_path_for_identity_strips_backslash_and_lowercases() {
        let result = super::normalize_track_path_for_identity(
            "\\PIONEER\\Contents\\Artist\\Album\\track.mp3",
        );
        // canonicalize_playlist_name strips non-alphanumeric, lowercases
        assert!(
            result.contains("contents"),
            "expected 'contents' segment, got: {result}"
        );
        assert!(
            result.contains("artist"),
            "expected 'artist' segment, got: {result}"
        );
        assert!(!result.contains('\\'), "backslashes should be stripped");
    }

    #[test]
    fn normalize_track_path_for_identity_extracts_contents_segment() {
        let result = super::normalize_track_path_for_identity(
            "/mnt/usb/PIONEER/Contents/Artist/Album/track.mp3",
        );
        assert!(
            result.starts_with("/contents/"),
            "should start with '/contents/', got: {result}"
        );
        assert!(
            result.contains("/artist/album/track.mp3"),
            "should contain path segments, got: {result}"
        );
    }

    #[test]
    fn normalize_track_path_for_identity_handles_empty() {
        assert_eq!(super::normalize_track_path_for_identity(""), "");
        assert_eq!(super::normalize_track_path_for_identity("  "), "");
    }

    #[test]
    fn normalize_pdb_path_for_edb_lookup_preserves_case() {
        let result = super::normalize_pdb_path_for_edb_lookup(
            "/mnt/usb/PIONEER/Contents/Artist/Album/Track.MP3",
        );
        assert!(
            result.starts_with("/Contents/"),
            "expected /Contents/ prefix, got: {result}"
        );
        assert!(
            result.contains("Track.MP3"),
            "should preserve original case"
        );
    }

    #[test]
    fn normalize_pdb_path_for_edb_lookup_strips_backslash() {
        let result =
            super::normalize_pdb_path_for_edb_lookup("\\PIONEER\\Contents\\Artist\\track.mp3");
        assert!(!result.contains('\\'), "backslashes should be stripped");
        // The rfind looks for /contents/ (lowercased), then slices the original
        assert!(
            result.contains("Contents"),
            "expected Contents segment, got: {result}"
        );
    }

    #[test]
    fn normalize_pdb_path_for_edb_lookup_handles_bare_contents() {
        let result = super::normalize_pdb_path_for_edb_lookup("Contents/Artist/track.mp3");
        assert_eq!(result, "/Contents/Artist/track.mp3");
    }

    #[test]
    fn track_identity_key_matches_for_equivalent_pdb_and_edb_paths() {
        let pdb_key =
            super::track_identity_key("/Contents/Artist/Album/track.mp3", "Title", "Artist", None);
        let edb_key =
            super::track_identity_key("/Contents/Artist/Album/track.mp3", "Title", "Artist", None);
        assert_eq!(
            pdb_key, edb_key,
            "same path should produce same identity key"
        );
    }

    #[test]
    fn track_identity_key_matches_with_backslash_prefix() {
        let pdb_key = super::track_identity_key(
            "\\PIONEER\\Contents\\Artist\\track.mp3",
            "Title",
            "Artist",
            None,
        );
        let edb_key =
            super::track_identity_key("/Contents/Artist/track.mp3", "Title", "Artist", None);
        assert_eq!(
            pdb_key, edb_key,
            "backslash PDB path should match forward-slash eDB path"
        );
    }

    #[test]
    fn track_identity_key_falls_back_to_metadata() {
        let key = super::track_identity_key("", "My Title", "My Artist", None);
        assert!(
            key.starts_with("meta:"),
            "empty path should fall back to meta key, got: {key}"
        );
    }

    #[test]
    fn track_identity_key_preserves_unicode_path_segments() {
        let pdb_key = super::track_identity_key(
            "/Contents/劇団レコード/Album/track.flac",
            "Get Higher",
            "劇団レコード",
            None,
        );
        let edb_key = super::track_identity_key(
            "/Contents/劇団レコード/Album/track.flac",
            "Get Higher",
            "劇団レコード",
            None,
        );
        assert_eq!(pdb_key, edb_key);
        assert!(pdb_key.contains("劇団レコード"));
    }

    #[test]
    fn track_identity_key_falls_back_to_id() {
        let key = super::track_identity_key("", "", "", Some("track-123"));
        assert_eq!(key, "id:track-123");
    }

    #[test]
    fn track_identity_key_returns_unknown_when_all_empty() {
        let key = super::track_identity_key("", "", "", None);
        assert_eq!(key, "unknown");
    }
}

fn normalize_usb_path_for_parity(value: &str) -> String {
    repair_utf8_mojibake(value.trim())
        .replace('\\', "/")
        .to_ascii_lowercase()
}
