//! USB diagnostics and parity report. `run` holds the entry points; the other
//! submodules hold the checks and the display layout they feed.

use std::collections::{HashMap, HashSet};

use crate::edb::{
    ExportDbPlaylist, open_edb_from_usb_root,
    try_read_playlists_with_metadata_from_edb_db_only_with_conn,
    try_read_playlists_with_metadata_from_edb_with_conn,
};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    DiagCheck, DiagCountsSummary, DiagOverviewItem, DiagSection, DiagSectionGroup, DiagStatus,
    DiagSummaryRow, PlayerCounterSnapshot, PlayerPageSignal, PlayerTableSignal, PlaylistDiagEntry,
    RunUsbDiagnosticsData, RunUsbDiagnosticsRequest, RunUsbParityReportData,
    RunUsbParityReportRequest, UsbParityPlaylistDetail, UsbTrack, WarningEntry,
};
use crate::pdb_reader::parse_pdb;
use crate::pdb_reader::parse_pdb_with_diagnostics;
use crate::utils::{
    collect_chain_lenient as collect_chain, page_offset, read_u8_at, read_u16_le_at,
    read_u32_le_at, table_ptr_fields,
};

use super::BackendService;
use super::export::{compute_playlist_usb_export_status, usb_playlist_names_for_export_compare};
use super::export_helpers::{load_table_columns, table_exists};
use super::export_prune_stale_setting;
use super::usb_helpers::is_named_history_playlist;
use super::usb_utils::{
    canonicalize_playlist_name, collect_contents_audio_files, repair_utf8_mojibake,
    resolve_usb_root,
};
use super::usb_vendor_compat::vendor_edb_path;

mod analysis_ownership;
mod audio_presence;
mod content_checks;
mod edb_checks;
mod overview;
mod parity;
mod paths;
mod pdb_integrity;
mod player_counters;
mod run;
#[cfg(test)]
mod tests;

pub(crate) use analysis_ownership::*;
pub(crate) use audio_presence::*;
pub(crate) use content_checks::*;
pub(crate) use edb_checks::*;
pub(crate) use overview::counted;
use overview::*;
pub(crate) use parity::*;
pub(crate) use paths::*;
pub(crate) use pdb_integrity::*;
use player_counters::*;
