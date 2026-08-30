# TODO

- (nothing outstanding)

## Done

- Unified all four track-list views (library, app playlist, USB playlist, USB
  history) onto the shared `vanilla-ui/components/shared/track_list_controller.mjs`.
  The backend list contract is unified: `browse_source_files`,
  `get_playlist_tracks`, `fetch_usb_playlist_tracks`, `fetch_usb_history_tracks`
  all take `query`/`sortBy`/`sortDir`/`cursor`/`limit` and return
  `items`/`total`/`nextCursor`/`hasMore` (+ duration aggregates).
  - **USB playlist / USB history:** server-hydrated pages, backend search+sort;
    ~600 lines of parallel USB machinery + the separate `inspect_usb_tracks`
    hydration step deleted.
  - **App playlist:** server-paginated/searched/sorted like the rest. A column
    sort stays a reversible view op until committed on navigate-away/export via
    `reorder_playlist_tracks`'s `sortBy` mode; drag-reorder is a single-move
    (`moveTrackId`/`beforeTrackId`). `unanalyzedCount` comes back on
    `get_playlist_tracks`. The controller lost its client sort/filter mode
    (nothing used it any more) and `sortTracks` / `filterTracksByQuery` /
    `applySortToTracks` are deleted.
  - Track-row action resolution is one shared `resolveRowActionTrack` helper
    across all four views.
  - **Library:** backend `browse_source_files` for fetch/paginate/search/sort —
    column sort now spans the whole library, not just the loaded page. `ctl.items`
    is backed by `state.tracks` (getItems/setItems) so playback resolution,
    analysis patching and selection stay consistent. `state.filteredTracks` +
    the client search overlay + `loadTracks`/`resetAndLoadLibraryTracks`/
    `loadMoreLibraryTracks`/`applySearchLocalFilter`/`ensureLibraryContainerFilled`
    /`readLibraryPagination` are gone; `renderLibraryChrome` keeps just the
    empty-state / onboarding / is-analyzing bits the controller doesn't own.
    Background waveform-preview hydration + source-root chips run from the
    controller's `onPage` / `onResponse` hooks.
