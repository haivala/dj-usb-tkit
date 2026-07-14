# TODO

- **Detect missing library source folders on app open.** Source roots are
  persisted in the `ui_source_roots_v1` setting (`backend/src/service/mod.rs`)
  and used by `scan_library`/`scan_audio_files` to (re)index tracks. If a
  source root is moved/renamed/unmounted outside the app, tracks under it
  quietly stop resolving on the next scan, and any playlist that referenced
  them silently ends up with fewer (or zero) tracks — no warning is surfaced
  to the user before export.
  - On startup (or when opening the library view), check each persisted
    source root with `Path::exists()` and show a clear "this folder is
    missing" state in the UI, with an action to relocate or remove it —
    instead of letting affected tracks/playlists go quietly empty.
  - `remove_tracks_by_source_roots` (`backend/src/service/mod.rs:1280`)
    already exists for explicit removal; this is about surfacing the missing
    root *before* the user is confused by an empty playlist or a failed/empty
    export, not adding new removal logic.
  - Repro case that surfaced this: a source folder was moved after tracks
    from it were added to a playlist and exported once; a second export
    after the move wrote the playlist with zero tracks and no warning.
