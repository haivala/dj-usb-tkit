//! USB diagnostics and parity report. `run` holds the entry points; the other
//! submodules hold the checks and the display layout they feed.

use super::BackendService;
use super::export_helpers::{load_table_columns, table_exists};
use super::usb::vendor_compat::vendor_edb_path;
use crate::edb::{ExportDbPlaylist, open_edb_from_usb_root};
use crate::error::{BackendError, BackendResult};
use crate::logging::{self, Level};
use crate::models::{
    DiagCheck, DiagCountsSummary, DiagOverviewItem, DiagSection, DiagSectionGroup, DiagStatus,
    DiagSummaryRow, PlayerCounterSnapshot, PlayerPageSignal, PlayerTableSignal, PlaylistDiagEntry,
    RunUsbDiagnosticsData, RunUsbDiagnosticsRequest, RunUsbParityReportData,
    RunUsbParityReportRequest, UsbParityPlaylistDetail, UsbTrack, WarningEntry,
};
use crate::service::usb::history::is_named_history_playlist;
use crate::service::usb::root::resolve_usb_root;
use crate::service::usb::stick_files::collect_contents_files;
use crate::service::usb::text::{canonicalize_playlist_name, repair_utf8_mojibake};
use crate::utils::{
    collect_chain_lenient as collect_chain, counted, page_offset, read_u8_at, read_u16_le_at,
    read_u32_le_at, table_ptr_fields,
};
use std::collections::{HashMap, HashSet};

mod analysis_ownership;
mod audio_presence;
mod content_checks;
mod drive_layout;
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
use drive_layout::*;
pub(crate) use edb_checks::*;
use overview::*;
pub(crate) use parity::*;
pub(crate) use paths::*;
pub(crate) use pdb_integrity::*;
use player_counters::*;
