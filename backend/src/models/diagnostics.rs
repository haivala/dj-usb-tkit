//! USB diagnostics, the parity report, and repairs.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsbDiagnosticsRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagStatus {
    Pass,
    Warn,
    Fail,
}

impl DiagStatus {
    pub fn worst(a: &DiagStatus, b: &DiagStatus) -> DiagStatus {
        match (a, b) {
            (DiagStatus::Fail, _) | (_, DiagStatus::Fail) => DiagStatus::Fail,
            (DiagStatus::Warn, _) | (_, DiagStatus::Warn) => DiagStatus::Warn,
            _ => DiagStatus::Pass,
        }
    }

    pub fn worst_of(statuses: &[&DiagStatus]) -> DiagStatus {
        let mut result = DiagStatus::Pass;
        for s in statuses {
            result = DiagStatus::worst(&result, s);
        }
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagCountsSummary {
    pub contents_count: usize,
    pub indexed_count: usize,
    pub mismatch_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagCheck {
    /// The catalog key of `label`: what the report's layout and overview match
    /// a check on, so they never compare translated text. Empty for a check
    /// named by data (a playlist's row).
    #[serde(skip)]
    pub key: &'static str,
    pub label: String,
    pub status: DiagStatus,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl DiagCheck {
    pub fn new(
        label: crate::i18n::Text,
        status: DiagStatus,
        detail: impl crate::i18n::IntoArg,
    ) -> Self {
        Self {
            key: label.key(),
            label: label.render(),
            status,
            detail: crate::i18n::shown(detail),
            link: None,
        }
    }

    /// A check named by data, such as a playlist's name.
    pub fn named(label: String, status: DiagStatus, detail: impl crate::i18n::IntoArg) -> Self {
        Self {
            key: "",
            label,
            status,
            detail: crate::i18n::shown(detail),
            link: None,
        }
    }

    pub fn with_link(mut self, link: impl Into<String>) -> Self {
        self.link = Some(link.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagSummaryRow {
    pub label: String,
    pub status: DiagStatus,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagSection {
    /// The catalog key of `title` for a report's display section: what the overview finds
    /// a section by, so it never compares translated text. Empty for the sections the
    /// checks build, before they're laid out.
    #[serde(skip)]
    pub key: &'static str,
    pub title: String,
    pub status: DiagStatus,
    pub checks: Vec<DiagCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<DiagCountsSummary>,
}

/// One plain-language row of the diagnostics overview. `status: None` means
/// not applicable (e.g. Engine DJ library not set up).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagOverviewItem {
    pub title: String,
    pub status: Option<DiagStatus>,
    pub detail: String,
}

/// Diagnostics sections shown side by side under one heading.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagSectionGroup {
    pub title: String,
    pub sections: Vec<DiagSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDiagEntry {
    pub name: String,
    pub total_entries: usize,
    pub resolved_entries: usize,
    pub resolution_rate: f64,
    pub status: DiagStatus,
    #[serde(default)]
    pub pdb_entries: usize,
    #[serde(default)]
    pub edb_entries: usize,
    #[serde(default)]
    pub matched_entries: usize,
    #[serde(default)]
    pub pdb_match_rate: f64,
    #[serde(default)]
    pub edb_match_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsbDiagnosticsData {
    pub overall_status: DiagStatus,
    /// Plain-language summary the desktop UI shows first
    /// (`service::diagnostics::diagnostics_overview`).
    pub overview: Vec<DiagOverviewItem>,
    /// The technical report: the stage sections below regrouped into display
    /// order (`service::diagnostics::regroup_sections`).
    pub section_groups: Vec<DiagSectionGroup>,
    // Per-stage sections, as computed. Backend-only (CLI tool, tests); the UI
    // gets the same checks through `sections`.
    #[serde(skip_serializing)]
    pub pdb_integrity: DiagSection,
    #[serde(skip_serializing)]
    pub edb_access: DiagSection,
    #[serde(skip_serializing)]
    pub contents_integrity: DiagSection,
    #[serde(skip_serializing)]
    pub analysis_integrity: DiagSection,
    #[serde(skip_serializing)]
    pub playlist_resolution: DiagSection,
    pub playlist_details: Vec<PlaylistDiagEntry>,
    /// Raw player-counter table signals -- consumed by the `run_usb_diagnostics`
    /// CLI debug tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdj_counter_snapshot: Option<PlayerCounterSnapshot>,
    /// The player-counter snapshot rendered as a `DiagSection`.
    #[serde(default, skip_serializing)]
    pub cdj_counter_section: Option<DiagSection>,
    /// Present when the USB has an Engine DJ library (`Engine Library/Database2`).
    #[serde(default, skip_serializing)]
    pub engine_library_section: Option<DiagSection>,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
    /// The status line after the run.
    #[serde(default)]
    pub status_text: String,
    /// "Completed in 370ms", in the report's header.
    #[serde(default)]
    pub duration_text: String,
    /// "PASS", on the report's badge.
    #[serde(default)]
    pub status_label: String,
    /// "USB health: good", the health dots' tooltip.
    #[serde(default)]
    pub health_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCounterSnapshot {
    pub playlist_count_candidate: usize,
    pub song_count_candidate: usize,
    pub confidence: String,
    pub shape_mode: String,
    pub baseline_init_like: bool,
    /// The PDB has tracks but t19 does not count them (still the init
    /// template, or an app-written ring at another count), so older players
    /// show 0 or 1 songs (repaired by `repair_pdb_t19_template_state`).
    #[serde(default)]
    pub t19_template_state: bool,
    pub t00_tracks: usize,
    pub t08_entries: usize,
    pub t11: PlayerTableSignal,
    pub t12: PlayerTableSignal,
    pub t17: PlayerTableSignal,
    pub t18: PlayerTableSignal,
    pub t19: PlayerTableSignal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerTableSignal {
    pub table_type: u32,
    pub ec: u32,
    pub first: u32,
    pub last: u32,
    pub chain_len: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_page: Option<PlayerPageSignal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPageSignal {
    pub page: u32,
    pub seq: u32,
    pub nrs: u8,
    pub u3: u8,
    pub num_rl: u16,
    pub rowpf0: u16,
    pub tranrf0: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsbParityReportRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbParityPlaylistDetail {
    pub name: String,
    pub pdb_tracks: usize,
    pub edb_tracks: usize,
    pub matched_tracks: usize,
    pub only_in_pdb: usize,
    pub only_in_edb: usize,
    pub order_mismatch: bool,
    #[serde(default)]
    pub path_mismatch_tracks: usize,
    #[serde(default)]
    pub dictionary_id_issue_tracks: usize,
    /// Both databases have the playlist (matched by name). Its id may
    /// differ: rekordbox numbers each database's playlists separately.
    #[serde(default)]
    pub playlist_in_both: bool,
    /// The playlist sits in the same place in both databases' playlist list
    /// (its position among the playlists both have, in its folder).
    #[serde(default)]
    pub sort_order_match: bool,
    #[serde(default)]
    pub parent_match: Option<bool>,
    #[serde(default)]
    pub pdb_playlist_id: Option<u32>,
    #[serde(default)]
    pub edb_playlist_id: Option<u32>,
    #[serde(default)]
    pub pdb_sort_order: Option<u32>,
    #[serde(default)]
    pub edb_sort_order: Option<u32>,
    #[serde(default)]
    pub pdb_duplicate_entries: usize,
    #[serde(default)]
    pub edb_missing_core_metadata: usize,
    #[serde(default)]
    pub pdb_missing_core_metadata: usize,
    #[serde(default)]
    pub artwork_mismatch_tracks: usize,
    /// Tracks whose star rating or colour differs between the PDB and the eDB.
    #[serde(default)]
    pub rating_colour_mismatch_tracks: usize,
    /// Tracks whose other synced fields (comment, ISRC, genre, label, …)
    /// differ between the PDB and the eDB. A note: never fails parity.
    #[serde(default)]
    pub field_note_tracks: usize,
    /// Other playlists of the same folder and name (ignoring case and
    /// punctuation), in either database: the strict parity repair merges
    /// them into the first.
    #[serde(default)]
    pub same_name_copies: usize,
    #[serde(default)]
    pub sample_only_in_pdb: Vec<String>,
    #[serde(default)]
    pub sample_only_in_edb: Vec<String>,
    #[serde(default)]
    pub sample_metadata_mismatches: Vec<String>,
    pub status: DiagStatus,
    /// Short human-readable badges for the parity table's "Issues" column
    /// (e.g. `"+PDB 3"`, `"order mismatch"`). Built backend-side from the
    /// counters above so the frontend renders them verbatim.
    #[serde(default)]
    pub issue_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunUsbParityReportData {
    pub overall_status: DiagStatus,
    /// Plain-language summary the desktop UI shows first
    /// (`service::diagnostics::parity_overview`).
    pub overview: Vec<DiagOverviewItem>,
    /// The technical report: `checks` regrouped into columns
    /// (`service::diagnostics::parity_section_groups`).
    pub section_groups: Vec<DiagSectionGroup>,
    // Flat check list and per-category counts. Backend-only (CLI tools,
    // tests); the UI gets the same checks through `section_groups`.
    #[serde(skip_serializing)]
    pub checks: Vec<DiagCheck>,
    #[serde(default, skip_serializing)]
    pub summary_rows: Vec<DiagSummaryRow>,
    pub playlist_details: Vec<UsbParityPlaylistDetail>,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
    /// The status line after the run.
    #[serde(default)]
    pub status_text: String,
    /// "Completed in 370ms", in the report's header.
    #[serde(default)]
    pub duration_text: String,
    /// "Parity PASS", on its badge.
    #[serde(default)]
    pub status_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairUsbDiagnosticsRequest {
    #[serde(default)]
    pub usb_root: Option<String>,
    #[serde(default)]
    pub apply: bool,
    #[serde(default)]
    pub selected_fix_ids: Vec<String>,
    /// A name typed into a row's `name_input`, by fix id. A fix whose name
    /// is missing or blank uses the row's default.
    #[serde(default)]
    pub fix_names: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairUnsupportedItem {
    pub issue: String,
    pub reason: String,
    /// `"event-log"` when the Event Log already lists the affected items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairFixProposal {
    pub id: String,
    pub title: String,
    pub description: String,
    pub supported: bool,
    pub destructive: bool,
    pub estimated_writes: usize,
    pub estimated_deletes: usize,
    /// Structural-prerequisite fixes that must run before a strict-parity
    /// upgrade -- the editor locks their checkbox checked. Set backend-side
    /// (see `REPAIR_FIX_DISPLAY_ORDER` / the apply order in `service::repair`).
    #[serde(default)]
    pub always_applied: bool,
    /// rekordbox's own transaction bookkeeping (real rekordbox exports carry
    /// it and play fine): not counted as an issue or listed in the editor,
    /// but applied with every repair that runs. Set backend-side.
    #[serde(default)]
    pub bookkeeping: bool,
    /// `"event-log"` when the Event Log already lists the affected items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Shown under the row as a text field; see `RepairRow::name_input`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_input: Option<RepairNameInput>,
}

/// One row of the repair preview's fix list, ready to show: a fix that can be
/// chosen, or something only described. `service::repair::repair_rows` builds
/// them from `proposed_fixes` and `unsupported_items`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairRow {
    pub id: String,
    pub title: String,
    pub description: String,
    /// "✓ supported · safe · 3 writes · always applied".
    pub meta_text: String,
    /// The row has a checkbox.
    pub selectable: bool,
    /// The checkbox starts ticked.
    pub checked: bool,
    /// The checkbox can't be unticked.
    pub locked: bool,
    /// Why, when `locked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_tooltip: Option<String>,
    /// `"event-log"` when the Event Log already lists the affected items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// A name the fix asks for (e.g. the playlist it creates). Apply sends
    /// what was typed back in `RepairUsbDiagnosticsRequest::fix_names`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_input: Option<RepairNameInput>,
}

/// A text field under a repair row, filled with the fix's default name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepairNameInput {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairUsbDiagnosticsData {
    /// One-sentence plain-language headline of the preview.
    #[serde(default)]
    pub summary: String,
    /// Plain-language rows of what Apply Fixes changes, by area
    /// (`service::repair::repair_overview`); the fix list is the detail.
    #[serde(default)]
    pub overview: Vec<DiagOverviewItem>,
    /// The preview's fix list, ready to show.
    #[serde(default)]
    pub rows: Vec<RepairRow>,
    /// There's nothing to fix, not even by hand.
    #[serde(default)]
    pub nothing_to_fix: bool,
    // Backend only (apply logic, CLI tools, tests): the UI gets `rows` and the text.
    #[serde(default, skip_serializing)]
    pub detected_issues: Vec<String>,
    #[serde(default, skip_serializing)]
    pub proposed_fixes: Vec<RepairFixProposal>,
    #[serde(default, skip_serializing)]
    pub unsupported_items: Vec<RepairUnsupportedItem>,
    #[serde(default, skip_serializing)]
    pub applied_fixes: Vec<String>,
    #[serde(default, skip_serializing)]
    pub skipped_fixes: Vec<String>,
    #[serde(default, skip_serializing)]
    pub failed_fixes: Vec<String>,
    #[serde(default, skip_serializing)]
    pub estimated_file_writes: usize,
    #[serde(default, skip_serializing)]
    pub estimated_file_deletes: usize,
    pub warnings: Vec<WarningEntry>,
    pub duration_ms: u64,
    /// The status line after a preview or an apply.
    #[serde(default)]
    pub status_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Box<RunUsbDiagnosticsData>>,
    /// Whether an apply may have changed what the USB lists and player menu
    /// show, so the UI must stop showing what it has loaded.
    #[serde(default)]
    pub usb_content_changed: bool,
    /// The "Write Engine DJ library" setting after this call; the
    /// `keep_engine_library_up_to_date` fix turns it on.
    #[serde(default)]
    pub engine_library_enabled: bool,
}
