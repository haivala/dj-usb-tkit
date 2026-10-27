//! Player counter snapshot: the table/page signals players read first.

use super::*;

pub(super) fn first_data_page_signal(
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

pub(super) fn table_signal(
    bytes: &[u8],
    page_size: usize,
    table_type: u32,
) -> Option<PlayerTableSignal> {
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

pub(super) fn compute_player_counter_snapshot(
    pdb_path: &std::path::Path,
    parsed: &crate::pdb_reader::ParsedPdb,
    property: Option<&crate::pdb_writer::T19Property>,
) -> Option<PlayerCounterSnapshot> {
    let bytes = std::fs::read(pdb_path).ok()?;
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

    let t19_template_state =
        crate::pdb_writer::t19_sync_target(&bytes, page_size, t00_tracks, property).is_some();

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
        t19_template_state,
        t00_tracks,
        t08_entries,
        t11,
        t12,
        t17,
        t18,
        t19,
    })
}

/// Label of the template-state t19 check. Listed under the PDB display
/// section, so it drives the "Older Pioneer players" overview line.
pub(super) const T19_TEMPLATE_STATE_LABEL: &str = "PDB history page";

/// Renders a `PlayerCounterSnapshot` as the "Player Counter Snapshot"
/// `DiagSection` (previously assembled in `vanilla-ui/components/usb/actions.mjs`
/// `renderDiagnosticsReport`). Every check is informational (`PASS`) except a
/// a t19 that doesn't match the USB, which is a `WARN` (players show a wrong track count but
/// still work); the section status is `PASS` only
/// at high confidence.
pub(super) fn player_counter_snapshot_section(cdj: &PlayerCounterSnapshot) -> DiagSection {
    let t19_line = match cdj.t19.data_page.as_ref() {
        Some(page) => format!(
            "t19 ec={} chain={} p{} nrs={} num_rl={} rowpf0=0x{:04x} tranrf0=0x{:04x}",
            cdj.t19.ec,
            cdj.t19.chain_len,
            page.page,
            page.nrs,
            page.num_rl,
            page.rowpf0,
            page.tranrf0,
        ),
        None => format!("t19 ec={} chain={}", cdj.t19.ec, cdj.t19.chain_len),
    };
    let history_pointers = format!(
        "t11 {}-{} ec={} | t12 {}-{} ec={} | t17 {}-{} ec={} | t18 {}-{} ec={}",
        cdj.t11.first,
        cdj.t11.last,
        cdj.t11.ec,
        cdj.t12.first,
        cdj.t12.last,
        cdj.t12.ec,
        cdj.t17.first,
        cdj.t17.last,
        cdj.t17.ec,
        cdj.t18.first,
        cdj.t18.last,
        cdj.t18.ec,
    );
    let info = |label: &str, detail: String| DiagCheck {
        label: label.to_string(),
        status: DiagStatus::Pass,
        detail,
        link: None,
    };
    let mut checks = Vec::new();
    if cdj.t19_template_state {
        checks.push(DiagCheck {
            label: T19_TEMPLATE_STATE_LABEL.to_string(),
            status: DiagStatus::Warn,
            detail: "t19 does not match the track count or the eDB's property row — older \
                     players may show 0 or 1 tracks. Repair fixes it."
                .to_string(),
            link: None,
        });
    }
    checks.extend([
        info(
            "Predicted player counter",
            format!(
                "playlists {}, songs {} (confidence: {})",
                cdj.playlist_count_candidate, cdj.song_count_candidate, cdj.confidence
            ),
        ),
        info(
            "Shape mode",
            format!(
                "{} (baseline-init-like: {})",
                cdj.shape_mode,
                if cdj.baseline_init_like { "yes" } else { "no" }
            ),
        ),
        info(
            "Cross-check",
            format!(
                "t00 tracks {}, t08 entries {}",
                cdj.t00_tracks, cdj.t08_entries
            ),
        ),
        info("History pointers", history_pointers),
        info("Primitive signal", t19_line),
    ]);
    DiagSection {
        title: "Player Counter Snapshot".to_string(),
        status: if cdj.confidence.eq_ignore_ascii_case("high") {
            DiagStatus::Pass
        } else {
            DiagStatus::Warn
        },
        checks,
        counts: None,
    }
}
