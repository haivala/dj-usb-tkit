//! Every command that writes the PDB, run one after another on a stick this
//! app made and on one whose pages are shaped the way rekordbox leaves them
//! (`0x34` page flags, a multi-row last transaction). After each write the
//! PDB must hold nothing `pdb_shape_problems` finds: the check every write's
//! commit passes, here held to the stricter "nothing at all", since both
//! sticks start clean.

mod common;
mod library_sources;

use common::{copy_audio_fixture, open_edb, pdb_path, seed_tracks_as_analyzed};

use std::fs;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::models::{
    AddLibrarySelectionToPlaylistRequest, BrowseSourceFilesRequest, CreatePlaylistRequest,
    ExportToUsbOptions, ExportToUsbRequest, FetchUsbPlaylistsRequest, InitializeUsbRequest,
    MoveUsbPlaylistRequest, MoveUsbPlaylistTrackRequest, RemoveTracksFromPlaylistRequest,
    RemoveUsbPlaylistRequest, RepairUsbDiagnosticsRequest, SaveUsbTrackAnalysisEditsRequest,
    SaveUsbTrackMetadataEditsRequest, ScanLibraryRequest, SetExportSyncModeRequest,
    SetUsbBackgroundColorRequest, SortUsbPlaylistTracksRequest, UpdateUsbPlayerMenuConfigRequest,
    UsbMetadataEditTrack, UsbOrderEditRequest, UsbPlayerMenuEdit, UsbPlaylist,
};
use backend::pdb_page;
use backend::service::pdb_shape_problems;
use tempfile::{TempDir, tempdir};

/// Tracks per playlist. Three playlists of this many grow the track table
/// over several pages.
const PER_PLAYLIST: usize = 12;

struct Stick {
    _root: TempDir,
    backend: BackendCommands,
    usb: PathBuf,
    /// Library playlist ids, A, B and C.
    playlists: Vec<String>,
    /// Library track ids per playlist.
    tracks: Vec<Vec<String>>,
}

impl Stick {
    fn root(&self) -> Option<String> {
        Some(self.usb.to_string_lossy().to_string())
    }

    fn export(&self, playlist: usize, prune_stale: bool) {
        let exported = self.backend.export_to_usb(ExportToUsbRequest {
            usb_root: self.root(),
            playlist_id: self.playlists[playlist].clone(),
            options: Some(ExportToUsbOptions {
                include_artwork: false,
                include_analysis: true,
                prune_stale,
                ..Default::default()
            }),
            sort: None,
        });
        assert!(exported.ok, "export {playlist}: {exported:?}");
    }

    fn usb_playlists(&self) -> Vec<UsbPlaylist> {
        self.backend
            .fetch_usb_playlists(FetchUsbPlaylistsRequest {
                usb_root: self.root(),
            })
            .data
            .expect("fetch usb playlists")
            .data
            .items
    }

    fn usb_playlist(&self, name: &str) -> UsbPlaylist {
        self.usb_playlists()
            .into_iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no USB playlist {name}"))
    }
}

/// Three library playlists of distinct tracks, the USB initialized.
fn stick() -> Stick {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let usb = root.path().join("usb");
    fs::create_dir_all(&media).expect("media dir");
    fs::create_dir_all(&usb).expect("usb dir");
    let formats = [
        "formats/track_format_wav.wav",
        "formats/track_format_flac.flac",
        "formats/track_format_aif.aif",
    ];
    for i in 0..PER_PLAYLIST * 3 {
        let fixture = formats[i % formats.len()];
        let ext = fixture.rsplit('.').next().unwrap();
        copy_audio_fixture(
            &media,
            fixture,
            &format!("Gate Artist {i:02} - Gate Track {i:02}.{ext}"),
        );
    }
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("backend");
    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scan.ok, "scan: {scan:?}");
    library_sources::show_only(
        &backend,
        vec![media.to_string_lossy().to_string()],
        false,
        false,
    );
    let mut scanned: Vec<(String, String)> = backend
        .browse_source_files(BrowseSourceFilesRequest {
            query: "gate".to_string(),
            limit: 100,
            cursor: None,
            ..Default::default()
        })
        .data
        .expect("browse")
        .items
        .into_iter()
        .map(|t| (t.file_path, t.id))
        .collect();
    scanned.sort();
    assert_eq!(scanned.len(), PER_PLAYLIST * 3);
    let ids: Vec<String> = scanned.into_iter().map(|(_, id)| id).collect();
    seed_tracks_as_analyzed(&data_dir, &ids);

    let mut playlists = Vec::new();
    let mut tracks = Vec::new();
    for (n, chunk) in ids.chunks(PER_PLAYLIST).enumerate() {
        let created = backend.create_playlist(CreatePlaylistRequest {
            name: format!("Gate {}", ["A", "B", "C"][n]),
        });
        let id = created.data.expect("playlist").data.playlist_id;
        let added =
            backend.add_library_selection_to_playlist(AddLibrarySelectionToPlaylistRequest {
                playlist_id: id.clone(),
                track_ids: chunk.to_vec(),
                ..Default::default()
            });
        assert!(added.ok, "add: {added:?}");
        playlists.push(id);
        tracks.push(chunk.to_vec());
    }
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "initialize: {initialized:?}");
    Stick {
        _root: root,
        backend,
        usb,
        playlists,
        tracks,
    }
}

fn assert_clean(usb: &Path, step: &str) {
    let bytes = fs::read(pdb_path(usb)).expect("read PDB");
    let problems = pdb_shape_problems(&bytes);
    assert!(problems.is_empty(), "after {step}: {problems:#?}");
}

/// Reshapes the data pages the way rekordbox leaves them: on a page with two
/// rows or more, a last transaction holding its last two rows instead of one
/// (or all), and flags `0x34` on every page of a table whose index page lists
/// pages (rekordbox's index lists its `0x34` pages). Content stays the same.
fn give_pages_rekordbox_shape(usb: &Path) {
    let path = pdb_path(usb);
    let mut bytes = fs::read(&path).expect("read PDB");
    let page_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let indexed: Vec<u32> = bytes
        .chunks_exact(page_size)
        .skip(1)
        .filter(|page| page[0x1b] == 0x64 && u16::from_le_bytes([page[0x38], page[0x39]]) > 0)
        .map(|page| u32::from_le_bytes(page[8..12].try_into().unwrap()))
        .collect();
    assert!(!indexed.is_empty(), "no index page lists pages");
    let mut reshaped = 0;
    for page in bytes.chunks_exact_mut(page_size).skip(1) {
        let stored = u32::from_le_bytes(page[4..8].try_into().unwrap());
        if stored == 0 || page[0x1b] & 0x40 != 0 {
            continue; // blank or index page
        }
        let Some((slots, _)) = pdb_page::read_slot_counts(page) else {
            continue;
        };
        if slots == 0 || !pdb_page::footer_clear_of_heap(page, slots) {
            continue;
        }
        if indexed.contains(&u32::from_le_bytes(page[8..12].try_into().unwrap())) {
            page[0x1b] = 0x34;
        }
        if slots >= 2 {
            pdb_page::write_transaction(page, slots, &[slots - 2, slots - 1]);
        }
        reshaped += 1;
    }
    assert!(reshaped > 0, "no data pages to reshape");
    fs::write(&path, bytes).expect("write PDB");
}

fn run_every_write(rekordbox_shaped: bool) {
    let stick = stick();
    let usb = stick.usb.clone();
    let backend = &stick.backend;
    let root = stick.root();
    assert_clean(&usb, "initialize");

    stick.export(0, false);
    assert_clean(&usb, "first export");
    if rekordbox_shaped {
        give_pages_rekordbox_shape(&usb);
        assert_clean(&usb, "the rekordbox-shaped pages themselves");
    }

    // Exports onto the stick, growing the track table onto new pages.
    stick.export(1, false);
    assert_clean(&usb, "export onto the stick");
    stick.export(2, false);
    assert_clean(&usb, "second export onto the stick");

    // Mirror export: tracks dropped from a playlist leave the stick.
    let dropped = stick.tracks[0][..3].to_vec();
    let removed = backend.remove_tracks_from_playlist(RemoveTracksFromPlaylistRequest {
        playlist_id: stick.playlists[0].clone(),
        track_ids: dropped,
    });
    assert!(removed.ok, "remove tracks: {removed:?}");
    let mirror = backend.set_export_sync_mode(SetExportSyncModeRequest {
        prune_stale: true,
        playlist_id: Some(stick.playlists[0].clone()),
        sort: None,
    });
    assert!(mirror.ok, "mirror mode: {mirror:?}");
    stick.export(0, true);
    assert_eq!(stick.usb_playlist("Gate A").track_count, PER_PLAYLIST - 3);
    assert_clean(&usb, "mirror export with removals");

    // Track details edited on the stick.
    let b = stick.usb_playlist("Gate B");
    let edited: Vec<UsbMetadataEditTrack> = b.tracks[..2]
        .iter()
        .map(|t| UsbMetadataEditTrack {
            usb_media_path_raw: t.usb_media_path.clone().expect("media path"),
            local_track_id: t.local_track_id.clone(),
            title: Some(t.title.clone()),
            artist: Some(t.artist.clone()),
            album: t.album.clone(),
        })
        .collect();
    let saved = backend.save_usb_track_metadata_edits_with_progress(
        SaveUsbTrackMetadataEditsRequest {
            usb_root: usb.to_string_lossy().to_string(),
            tracks: edited,
            genre: Some("Gate Genre With A Much Longer Name".to_string()),
            rating: Some(4),
            release_date: Some("2001-02-03".to_string()),
        },
        |_, _, _| {},
    );
    assert!(saved.ok, "metadata edit: {saved:?}");
    assert_clean(&usb, "track metadata edit");

    // BPM edited on the stick (the track editor's save).
    let track = &stick.usb_playlist("Gate B").tracks[2];
    let saved = backend.save_usb_track_analysis_edits(SaveUsbTrackAnalysisEditsRequest {
        usb_root: usb.to_string_lossy().to_string(),
        usb_analysis_path_raw: track.usb_analysis_path_raw.clone().expect("analysis path"),
        usb_media_path_raw: track.usb_media_path.clone().expect("media path"),
        bpm: Some(track.bpm.unwrap_or(120.0) + 1.0),
        key: None,
        duration_ms: track.duration_ms,
        first_beat_ms: None,
        cues: None,
        local_track_id: track.local_track_id.clone(),
        title: Some(track.title.clone()),
        artist: Some(track.artist.clone()),
        album: track.album.clone(),
    });
    assert!(saved.ok, "analysis edit: {saved:?}");
    assert_clean(&usb, "BPM edit");

    // Player menu: hide a category, show it again, move it.
    const KEY_KIND: u32 = 139;
    for action in [
        UsbPlayerMenuEdit::Remove,
        UsbPlayerMenuEdit::Add,
        UsbPlayerMenuEdit::MoveUp,
    ] {
        let edit = backend.update_usb_player_menu_config(UpdateUsbPlayerMenuConfigRequest {
            usb_root: root.clone(),
            action,
            kind: KEY_KIND,
        });
        assert!(edit.ok, "menu {action:?}: {edit:?}");
        assert_clean(&usb, &format!("player menu {action:?}"));
    }
    let color = backend.set_usb_background_color(SetUsbBackgroundColorRequest {
        usb_root: root.clone(),
        color: 5,
    });
    assert!(color.ok, "background colour: {color:?}");
    assert_clean(&usb, "background colour");

    // Order edit: playlists and tracks moved, a playlist sorted.
    let order = || UsbOrderEditRequest {
        usb_root: root.clone(),
    };
    assert!(backend.begin_usb_order_edit(order()).ok, "begin order edit");
    let a = stick.usb_playlist("Gate A");
    let c = stick.usb_playlist("Gate C");
    let moved = backend.move_usb_playlist(MoveUsbPlaylistRequest {
        usb_root: root.clone(),
        playlist_id: c.id.clone(),
        before_playlist_id: Some(a.id.clone()),
    });
    assert!(moved.ok, "move playlist: {moved:?}");
    let moved = backend.move_usb_playlist_track(MoveUsbPlaylistTrackRequest {
        usb_root: root.clone(),
        playlist_id: a.id.clone(),
        move_track_id: a.tracks.last().expect("track").id.clone(),
        before_track_id: Some(a.tracks[0].id.clone()),
    });
    assert!(moved.ok, "move track: {moved:?}");
    let sorted = backend.sort_usb_playlist_tracks(SortUsbPlaylistTracksRequest {
        usb_root: root.clone(),
        playlist_id: c.id.clone(),
        sort_by: Some("title".to_string()),
        sort_dir: Some("desc".to_string()),
    });
    assert!(sorted.ok, "sort: {sorted:?}");
    let committed = backend.commit_usb_order_edit(order());
    assert!(committed.ok, "commit order edit: {committed:?}");
    assert_clean(&usb, "order edit");

    // A playlist removed from the stick, with its tracks.
    let removed = backend.remove_usb_playlist(RemoveUsbPlaylistRequest {
        usb_root: root.clone(),
        playlist_id: None,
        playlist_name: "Gate B".to_string(),
    });
    assert!(removed.ok, "remove playlist: {removed:?}");
    assert_clean(&usb, "remove playlist");

    // Re-exported after the removal: the freed rows and pages get reused.
    stick.export(1, false);
    assert_clean(&usb, "export after a removal");

    // Strict parity carrying eDB-only values into the PDB's rows.
    let conn = open_edb(&pdb_path(&usb).with_file_name("exportLibrary.db"));
    conn.execute("UPDATE content SET rating = 2, color_id = 3", [])
        .expect("eDB rating and colour");
    conn.execute(
        "DELETE FROM playlist_content WHERE playlist_id IN \
         (SELECT playlist_id FROM playlist WHERE name = 'Gate C')",
        [],
    )
    .expect("drop eDB membership");
    drop(conn);
    let before = fs::read(pdb_path(&usb)).expect("read PDB");
    apply_every_fix(&stick);
    assert_ne!(fs::read(pdb_path(&usb)).expect("read PDB"), before);
    assert_clean(&usb, "strict parity repair");

    // A missing PDB rebuilt from the eDB.
    fs::remove_file(pdb_path(&usb)).expect("remove PDB");
    apply_every_fix(&stick);
    assert_clean(&usb, "PDB rebuilt by repair");
}

/// Applies every fix the repair offers, except keeping the Engine library.
fn apply_every_fix(stick: &Stick) {
    let request = |apply, selected_fix_ids| RepairUsbDiagnosticsRequest {
        fix_names: Default::default(),
        usb_root: stick.root(),
        apply,
        selected_fix_ids,
    };
    let preview = stick
        .backend
        .repair_usb_diagnostics(request(false, Vec::new()))
        .data
        .expect("repair preview")
        .data;
    let fixes: Vec<String> = preview
        .proposed_fixes
        .iter()
        .map(|f| f.id.clone())
        .filter(|id| id != "keep_engine_library_up_to_date")
        .collect();
    assert!(!fixes.is_empty(), "nothing to repair: {preview:?}");
    let applied = stick
        .backend
        .repair_usb_diagnostics(request(true, fixes))
        .data
        .expect("repair apply")
        .data;
    assert!(
        applied.failed_fixes.is_empty(),
        "{:?}",
        applied.failed_fixes
    );
}

#[test]
fn every_write_leaves_an_app_made_pdb_readable() {
    run_every_write(false);
}

#[test]
fn every_write_leaves_a_rekordbox_shaped_pdb_readable() {
    run_every_write(true);
}
