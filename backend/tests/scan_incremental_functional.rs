use std::fs;

use backend::commands::BackendCommands;
use backend::models::{
    AddTracksToPlaylistRequest, CreatePlaylistRequest, DedupeMode, GetPlaylistTracksRequest,
    RelocateSourceRootRequest, ScanLibraryRequest, SearchTracksRequest,
};
use tempfile::tempdir;

#[test]
fn scan_incremental_covers_unchanged_new_and_deleted_files() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");

    let track_a = media.join("Artist - A.mp3");
    let track_b = media.join("Artist - B.mp3");
    let track_c = media.join("Artist - C.mp3");
    fs::write(&track_a, b"audio-a").expect("write track a");
    fs::write(&track_b, b"audio-b").expect("write track b");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");

    let first = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(first.ok, "first scan failed: {first:?}");
    let first_data = first.data.expect("first scan data");
    assert_eq!(first_data.indexed, 2);
    assert_eq!(first_data.updated, 0);
    assert_eq!(first_data.removed, 0);

    let unchanged = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(unchanged.ok, "unchanged scan failed: {unchanged:?}");
    let unchanged_data = unchanged.data.expect("unchanged scan data");
    assert_eq!(unchanged_data.indexed, 0);
    assert_eq!(unchanged_data.updated, 0);
    assert_eq!(unchanged_data.removed, 0);

    fs::write(&track_c, b"audio-c").expect("write track c");
    let with_new = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(with_new.ok, "scan with new track failed: {with_new:?}");
    let with_new_data = with_new.data.expect("scan with new data");
    assert_eq!(with_new_data.indexed, 1);
    assert_eq!(with_new_data.updated, 0);
    assert_eq!(with_new_data.removed, 0);

    fs::remove_file(&track_b).expect("remove track b");
    let with_delete = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(with_delete.ok, "scan with delete failed: {with_delete:?}");
    let with_delete_data = with_delete.data.expect("scan with delete data");
    assert_eq!(with_delete_data.indexed, 0);
    assert_eq!(with_delete_data.updated, 0);
    assert_eq!(with_delete_data.removed, 1);

    let final_tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("final search data");
    assert_eq!(final_tracks.total, 2, "expected A and C after delete");
}

#[test]
fn scan_missing_source_root_reports_not_found_without_pruning_tracks() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    fs::write(media.join("Artist - A.mp3"), b"audio-a").expect("write track a");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let source_root = media.to_string_lossy().to_string();

    let first = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![source_root.clone()],
        incremental: true,
    });
    assert!(first.ok, "first scan failed: {first:?}");
    assert_eq!(first.data.expect("first scan data").indexed, 1);

    fs::remove_dir_all(&media).expect("remove media dir");

    let missing = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![source_root.clone()],
        incremental: true,
    });
    assert!(missing.ok, "missing-root scan failed: {missing:?}");
    let missing_data = missing.data.expect("missing scan data");
    assert_eq!(missing_data.indexed, 0);
    assert_eq!(missing_data.updated, 0);
    assert_eq!(missing_data.removed, 0);
    assert_eq!(missing_data.not_found, vec![source_root]);
    assert!(!missing_data.warnings.is_empty());

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("search data");
    assert_eq!(tracks.total, 1, "missing root must not prune tracks");
}

#[test]
fn relocate_source_root_rewrites_paths_and_preserves_playlist_membership() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    let nested = media.join("Artist");
    fs::create_dir_all(&nested).expect("create media dir");
    fs::write(nested.join("Artist - A.mp3"), b"audio-a").expect("write track a");

    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    let old_root = media.to_string_lossy().to_string();

    let scan = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![old_root.clone()],
        incremental: true,
    });
    assert!(scan.ok, "scan failed: {scan:?}");

    let tracks = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("search data");
    assert_eq!(tracks.total, 1);
    let track_id = tracks.items[0].id.clone();

    let playlist = backend
        .create_playlist(CreatePlaylistRequest {
            name: "Crate".to_string(),
        })
        .data
        .expect("playlist data");
    let added = backend.add_tracks_to_playlist(AddTracksToPlaylistRequest {
        playlist_id: playlist.playlist_id.clone(),
        track_ids: vec![track_id.clone()],
        dedupe: DedupeMode::Allow,
    });
    assert!(added.ok, "add tracks failed: {added:?}");

    let new_media = root.path().join("relocated");
    fs::rename(&media, &new_media).expect("move media dir");
    let new_root = new_media.to_string_lossy().to_string();

    let relocated = backend.relocate_source_root(RelocateSourceRootRequest {
        old_root: old_root.clone(),
        new_root: new_root.clone(),
    });
    assert!(relocated.ok, "relocation failed: {relocated:?}");
    let relocated_data = relocated.data.expect("relocation data");
    assert_eq!(relocated_data.matched, 1);
    assert_eq!(relocated_data.updated, 1);
    assert_eq!(relocated_data.missing_at_new_root, 0);
    assert_eq!(relocated_data.conflicts, 0);

    let tracks_after = backend
        .search_tracks(SearchTracksRequest {
            query: String::new(),
            limit: 50,
            cursor: None,
        })
        .data
        .expect("search data after relocation");
    assert_eq!(tracks_after.total, 1);
    assert_eq!(tracks_after.items[0].id, track_id);
    assert!(tracks_after.items[0].file_path.starts_with(&new_root));

    let playlist_tracks = backend
        .get_playlist_tracks(GetPlaylistTracksRequest {
            playlist_id: playlist.playlist_id,
            ..Default::default()
        })
        .data
        .expect("playlist tracks data");
    assert_eq!(playlist_tracks.items.len(), 1);
    assert_eq!(playlist_tracks.items[0].id, track_id);
    assert!(playlist_tracks.items[0].file_path.starts_with(&new_root));
}

fn set_waveform_path(data_dir: &std::path::Path, file_name: &str, dat: &std::path::Path) {
    let conn = rusqlite::Connection::open(data_dir.join("backend.db")).expect("open backend db");
    let updated = conn
        .execute(
            "UPDATE tracks SET waveform_peaks_path = ?1 WHERE file_path LIKE ?2",
            rusqlite::params![dat.to_string_lossy(), format!("%{file_name}")],
        )
        .expect("set waveform path");
    assert_eq!(updated, 1, "expected one track row for {file_name}");
}

fn waveform_path(data_dir: &std::path::Path, file_name: &str) -> Option<String> {
    let conn = rusqlite::Connection::open(data_dir.join("backend.db")).expect("open backend db");
    conn.query_row(
        "SELECT waveform_peaks_path FROM tracks WHERE file_path LIKE ?1",
        rusqlite::params![format!("%{file_name}")],
        |row| row.get(0),
    )
    .expect("query waveform path")
}

fn write_bundle(dat: &std::path::Path, twoex: &[u8]) {
    fs::create_dir_all(dat.parent().unwrap()).expect("create bundle dir");
    fs::write(dat, b"PMAI-dat").expect("write DAT");
    fs::write(dat.with_extension("EXT"), b"PMAI-ext").expect("write EXT");
    fs::write(dat.with_extension("2EX"), twoex).expect("write 2EX");
}

fn scan(backend: &BackendCommands, media: &std::path::Path) {
    let res = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(res.ok, "scan failed: {res:?}");
}

/// Local cache bundles are named by a source-path hash, not the track id, so
/// the stale-PWV6 check must follow `waveform_peaks_path` -- and must never
/// touch a bundle outside the local cache (e.g. one on a USB stick).
#[test]
fn scan_clears_stale_local_anlz_cache_but_never_usb_bundles() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    for name in ["Stale.mp3", "Fresh.mp3", "Usb.mp3"] {
        fs::write(media.join(name), name.as_bytes()).expect("write track");
    }
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    scan(&backend, &media);

    let cache = data_dir.join("analysis").join("waveforms");
    let stale = cache.join("0A1B2C3D.DAT");
    let fresh = cache.join("4E5F6071.DAT");
    let usb = root
        .path()
        .join("usb/PIONEER/USBANLZ/P001/0000ABCD/ANLZ0000.DAT");
    write_bundle(&stale, b"PMAI-PPTH-PWV7");
    write_bundle(&fresh, b"PMAI-PPTH-PWV7-PWV6");
    write_bundle(&usb, b"PMAI-PPTH-PWV7");
    set_waveform_path(&data_dir, "Stale.mp3", &stale);
    set_waveform_path(&data_dir, "Fresh.mp3", &fresh);
    set_waveform_path(&data_dir, "Usb.mp3", &usb);

    scan(&backend, &media);

    assert_eq!(waveform_path(&data_dir, "Stale.mp3"), None);
    for ext in ["DAT", "EXT", "2EX"] {
        assert!(
            !stale.with_extension(ext).exists(),
            "stale .{ext} not deleted"
        );
        assert!(fresh.with_extension(ext).exists(), "fresh .{ext} deleted");
        assert!(usb.with_extension(ext).exists(), "USB .{ext} deleted");
    }
    assert_eq!(
        waveform_path(&data_dir, "Fresh.mp3").as_deref(),
        Some(fresh.to_string_lossy().as_ref())
    );
    assert_eq!(
        waveform_path(&data_dir, "Usb.mp3").as_deref(),
        Some(usb.to_string_lossy().as_ref())
    );
}

#[test]
fn scan_removing_a_track_deletes_its_hashed_cache_bundle_unless_shared() {
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    fs::create_dir_all(&media).expect("create media dir");
    for name in ["Gone.mp3", "GoneShared.mp3", "Kept.mp3"] {
        fs::write(media.join(name), name.as_bytes()).expect("write track");
    }
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    scan(&backend, &media);

    let cache = data_dir.join("analysis").join("waveforms");
    let own = cache.join("11111111.DAT");
    let shared = cache.join("22222222.DAT");
    write_bundle(&own, b"PMAI-PWV6");
    write_bundle(&shared, b"PMAI-PWV6");
    set_waveform_path(&data_dir, "Gone.mp3", &own);
    set_waveform_path(&data_dir, "GoneShared.mp3", &shared);
    set_waveform_path(&data_dir, "Kept.mp3", &shared);

    fs::remove_file(media.join("Gone.mp3")).expect("remove Gone");
    fs::remove_file(media.join("GoneShared.mp3")).expect("remove GoneShared");
    scan(&backend, &media);

    for ext in ["DAT", "EXT", "2EX"] {
        assert!(
            !own.with_extension(ext).exists(),
            "orphaned .{ext} not deleted"
        );
        assert!(shared.with_extension(ext).exists(), "shared .{ext} deleted");
    }
}
