# TODO

- Unify the four track-list views (library, app playlist, USB playlist, USB
  history). Rendering already shares one path (`renderTrackTable`/
  `createTrackRow` in `vanilla-ui/track_table.mjs`); data-fetching/pagination
  doesn't — library has cursor pagination + a "load more near bottom" scroll
  handler, app playlist is a single unpaginated query, USB playlist/history
  need a separate hydration step (PDB parse + SQLCipher decrypt) that the
  others don't. Worth designing one generic paginated-fetch+hydrate
  component so future large-list fixes don't need reimplementing per view.

  Deferred (decided, not just idle): not worth doing proactively —
  library's cursor pagination and app-playlist's small,
  user-bounded size aren't showing any actual problem today, so unifying
  now would be speculative work against a hypothetical future bug rather
  than a fix for a demonstrated one, and a bigger/riskier change than any
  single round of the USB pagination work that prompted this TODO. Revisit
  if/when library or app-playlist actually shows similar pain (e.g. a slow
  /frozen library with a very large collection) — plan it against that
  concrete problem then, not ahead of need.
