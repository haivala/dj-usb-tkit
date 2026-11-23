//! What a running job blocks. One table for the whole app, documented in `docs/COMMANDS.md`
//! ("What a running job blocks"):
//!
//! - every command is classified by name (`command_block`): either it belongs to an
//!   [`ActionGroup`] or no job ever blocks it;
//! - each job type blocks some groups (`blocked_by`);
//! - the desktop app refuses a blocked command before it runs, and sends the blocked groups
//!   with the reason (`blocked_actions`) whenever a job starts or ends, so the UI disables
//!   every control marked with a blocked group.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::i18n::{Text, tr};

/// A set of user actions a running job can block. The UI marks each control with its
/// group (`data-action-group`, the kebab-case name).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionGroup {
    /// Picking another USB (or forgetting one).
    SwitchUsb,
    /// Exporting playlists to the USB.
    Export,
    /// Every other change to the USB, and the checks of it (diagnostics, parity, repair).
    Usb,
    /// Changing what the library holds: source folders, scans, rekordbox / Mixxx imports.
    Library,
    /// Converting tracks (the transcode files) and their folder.
    Convert,
    /// Starting an analysis, also Analyze Library's scan of the folders first.
    Analyze,
    /// Saving cue / beat-grid / metadata edits of library tracks.
    TrackEdit,
    /// Installing or removing the essentia analysis engine.
    Essentia,
    /// Installing an update or changing the language (both restart the window).
    App,
}

use ActionGroup::*;

impl ActionGroup {
    pub const ALL: [ActionGroup; 9] = [
        SwitchUsb, Export, Usb, Library, Convert, Analyze, TrackEdit, Essentia, App,
    ];
}

/// The groups a running job of `job_type` blocks.
pub fn blocked_by(job_type: &str) -> &'static [ActionGroup] {
    match job_type {
        "export" | "usb_write" => &[SwitchUsb, Export, Usb, Library, Convert, App],
        "diagnostics" => &[SwitchUsb, Export, Usb, App],
        // Short reads (a USB's playlists, its menu): the stick's databases are read from
        // the staged copies, so reads never wait for writes or checks, nor these for reads.
        "usb_read" => &[SwitchUsb, App],
        "scan" | "transcode" => &[Export, Library, Convert, App],
        "analysis" => &[Essentia, App],
        "migration" => &[Analyze, TrackEdit, App],
        "update" => &ActionGroup::ALL,
        _ => &[App],
    }
}

/// What a blocked control says: which job to wait for.
fn reason(job_type: &str) -> Text {
    match job_type {
        "export" => tr::job_blocks_export(),
        "usb_write" => tr::job_blocks_usb_write(),
        "usb_read" => tr::job_blocks_usb_read(),
        "diagnostics" => tr::job_blocks_diagnostics(),
        "scan" => tr::job_blocks_scan(),
        "transcode" => tr::job_blocks_transcode(),
        "analysis" => tr::job_blocks_analysis(),
        "migration" => tr::job_blocks_migration(),
        "update" => tr::job_blocks_update(),
        _ => tr::job_blocks_other(),
    }
}

/// The blocked groups while jobs of these types run, each with the reason of the first job
/// that blocks it.
pub fn blocked_actions<'a>(
    running_job_types: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<ActionGroup, String> {
    let mut out = BTreeMap::new();
    for job_type in running_job_types {
        for group in blocked_by(job_type) {
            out.entry(*group)
                .or_insert_with(|| reason(job_type).render());
        }
    }
    out
}

/// How a command is classified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandBlock {
    /// No job blocks it: reads, app playlists, settings, playback, cancels.
    Never,
    /// Refused while its group is blocked.
    By(ActionGroup),
}

/// The classification of every command the desktop app registers, by name; `None` for a
/// name that isn't classified (a test keeps that list empty, so a new command can't be
/// forgotten here).
pub fn command_block(command: &str) -> Option<CommandBlock> {
    use CommandBlock::{By, Never};
    Some(match command {
        "select_usb" | "prune_usb_device" | "pick_usb_folder" => By(SwitchUsb),

        "export_to_usb" | "export_playlists_to_usb" => By(Export),

        "set_usb_device_name"
        | "set_usb_audio_formats"
        | "restore_usb_backup"
        | "delete_usb_backup"
        | "update_usb_player_menu_config"
        | "set_usb_background_color"
        | "remove_usb_playlist"
        | "begin_usb_order_edit"
        | "move_usb_playlist"
        | "move_usb_playlist_track"
        | "sort_usb_playlist_tracks"
        | "commit_usb_order_edit"
        | "save_usb_track_analysis_edits"
        | "save_usb_track_metadata_edits"
        | "initialize_usb"
        | "run_usb_diagnostics"
        | "run_usb_parity_report"
        | "repair_usb_diagnostics" => By(Usb),

        "scan_rekordbox_db"
        | "scan_mixxx_db"
        | "import_external_playlists"
        | "apply_external_track_data"
        | "add_source_folders"
        | "remove_source_folder"
        | "set_source_folder_enabled"
        | "set_external_library_enabled"
        | "relocate_source_root"
        | "pick_source_folders" => By(Library),

        "transcode_tracks"
        | "remove_track_transcodes"
        | "set_transcode_folder"
        | "pick_transcode_folder" => By(Convert),

        "analyze_new_tracks" | "scan_library" => By(Analyze),

        "save_track_analysis_edits" | "save_track_metadata_edits" => By(TrackEdit),

        "download_essentia" | "remove_essentia" => By(Essentia),

        "set_language" | "install_update" => By(App),

        // Reads.
        "list_mixxx_playlists"
        | "list_rekordbox_playlists"
        | "browse_source_files"
        | "get_library_sources"
        | "resolve_track_identity"
        | "get_track_detail"
        | "get_usb_track_detail"
        | "get_tracks_metadata_summary"
        | "get_usb_tracks_metadata_summary"
        | "list_playlists"
        | "get_playlist_tracks"
        | "get_playlist_export_space"
        | "get_playlists_export_space"
        | "get_batch_export_overview"
        | "list_matching_track_ids"
        | "get_selection_duration"
        | "get_playlist_membership"
        | "get_ui_texts"
        | "get_frontend_settings"
        | "get_track_columns"
        | "list_usb_devices"
        | "list_usb_backups"
        | "fetch_usb_playlists"
        | "fetch_usb_histories"
        | "get_usb_player_menu_config"
        | "get_usb_background_color"
        | "inspect_usb_track"
        | "fetch_usb_playlist_tracks"
        | "fetch_usb_history_tracks"
        | "get_usb_history_tracklist_options"
        | "build_usb_history_tracklist"
        | "get_transcode_formats"
        | "get_transcode_folder"
        | "detect_external_rekordbox_db"
        | "detect_external_mixxx_db"
        | "check_for_update"
        | "get_playback_status_native"
        | "get_backend_log_buffer" => Never,

        // App playlists: an export marks a playlist changed while it runs, so editing
        // them stays allowed.
        "create_playlist"
        | "rename_playlist"
        | "reorder_playlists"
        | "delete_playlist"
        | "add_track_candidates_to_playlist"
        | "add_library_selection_to_playlist"
        | "add_library_selection_to_playlists"
        | "add_track_candidates_to_playlists"
        | "remove_tracks_from_playlist"
        | "reorder_playlist_tracks" => Never,

        // Settings, cancels, playback and the window.
        "set_frontend_setting"
        | "set_track_columns"
        | "set_export_sync_mode"
        | "cancel_usb_order_edit"
        | "set_analysis_paused"
        | "cancel_analysis"
        | "cancel_transcode"
        | "cancel_essentia_download"
        | "run_data_migrations"
        | "play_resolved_track"
        | "stop_playback_native"
        | "pause_playback_native"
        | "resume_playback_native"
        | "set_playback_metronome"
        | "open_frontend_log"
        | "reveal_track_file"
        | "append_frontend_log"
        | "save_text_file"
        | "show_window"
        | "set_theme_background" => Never,

        _ => return None,
    })
}

/// The reason `command` is refused while jobs of these types run, or `None` when it may run.
pub fn refusal<'a>(
    command: &str,
    running_job_types: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let Some(CommandBlock::By(group)) = command_block(command) else {
        return None;
    };
    blocked_actions(running_job_types).remove(&group)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_blocked_while_no_job_runs() {
        assert!(blocked_actions([]).is_empty());
        assert_eq!(refusal("export_to_usb", []), None);
    }

    #[test]
    fn an_export_blocks_usb_and_library_changes_but_not_analysis_or_playlists() {
        assert!(refusal("export_playlists_to_usb", ["export"]).is_some());
        assert!(refusal("set_usb_device_name", ["export"]).is_some());
        assert!(refusal("remove_source_folder", ["export"]).is_some());
        assert!(refusal("transcode_tracks", ["export"]).is_some());
        assert_eq!(refusal("analyze_new_tracks", ["export"]), None);
        // Analyze Library: its scan of the folders first, then the analysis.
        assert_eq!(refusal("scan_library", ["export"]), None);
        assert_eq!(refusal("rename_playlist", ["export"]), None);
        assert_eq!(refusal("save_track_analysis_edits", ["export"]), None);
    }

    #[test]
    fn an_analysis_blocks_only_the_engine_and_the_app() {
        let blocked: Vec<_> = blocked_actions(["analysis"]).into_keys().collect();
        assert_eq!(blocked, vec![Essentia, App]);
        // Analyze Library scans first, and adds to the running analysis.
        assert_eq!(refusal("scan_library", ["analysis"]), None);
        assert_eq!(refusal("add_source_folders", ["analysis"]), None);
    }

    #[test]
    fn a_migration_blocks_analysis_and_track_edits() {
        assert!(refusal("analyze_new_tracks", ["migration"]).is_some());
        assert!(refusal("save_track_metadata_edits", ["migration"]).is_some());
        assert_eq!(refusal("select_usb", ["migration"]), None);
    }

    #[test]
    fn an_update_blocks_everything() {
        let blocked: Vec<_> = blocked_actions(["update"]).into_keys().collect();
        assert_eq!(blocked, ActionGroup::ALL.to_vec());
    }

    #[test]
    fn the_first_blocking_job_gives_the_reason() {
        let blocked = blocked_actions(["scan", "export"]);
        assert_eq!(blocked[&Export], tr::job_blocks_scan().render());
        assert_eq!(blocked[&Usb], tr::job_blocks_export().render());
    }

    #[test]
    fn every_group_is_blocked_by_some_job() {
        let blocked = blocked_actions([
            "export",
            "usb_write",
            "usb_read",
            "diagnostics",
            "scan",
            "transcode",
            "analysis",
            "migration",
        ]);
        let mut groups: Vec<_> = blocked.into_keys().collect();
        groups.sort();
        assert_eq!(groups, ActionGroup::ALL.to_vec());
    }
}
