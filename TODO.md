# TODO

- Finish unifying the four track-list views onto the shared
  `vanilla-ui/components/shared/track_list_controller.mjs`.

  **Done:** the controller exists; the backend list contract is unified
  (`browse_source_files`, `get_playlist_tracks`, and the new
  `fetch_usb_playlist_tracks` / `fetch_usb_history_tracks` all take
  `query`/`sortBy`/`sortDir`/`cursor`/`limit` and return
  `items`/`total`/`nextCursor`/`hasMore`). **USB playlist** and **USB history**
  are fully migrated (server-hydrated pages, backend search+sort, ~600 lines of
  parallel USB machinery + the separate `inspect_usb_tracks` hydration step
  deleted). The **app playlist** is migrated too, in the controller's
  `sortMode: "client"` — its list is the whole (unpaginated) playlist backed by
  `getCurrentPlaylist().tracks`, and its column sort + search stay client-side
  view ops (`ctl.view`) because the sort only becomes real order on
  navigate-away/export via `commitActivePlaylistSort`, and search must not
  shrink the list drag-reorder / commit operate on. Drag-reorder and the
  sort-commit stay outside the controller (the editable-list carve-out).

  **Remaining:** migrate the **library** view (`browse_source_files`) — the
  payoff is that its column sort would then span the whole library instead of
  only the loaded page, and the `state.filteredTracks` client-search overlay
  could go; the cost is coordinating the controller with `state.tracks`
  (read by playback resolution, analysis patching, selection), the
  select-all-across-pages flow, and the source-root chips / background
  waveform-preview hydration hooks.
