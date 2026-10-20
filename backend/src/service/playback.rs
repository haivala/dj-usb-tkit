//! Native playback and resolving which file a track plays from.

use super::*;

/// The status-line label: only *where* playback is coming from, with no
/// qualifier about how it was resolved or what was skipped -- any fallback
/// detail belongs in the Event Log, not here. Every caller either resolved
/// the library copy or is playing a USB copy (a request with neither errors
/// out before it gets here), so those are the only two outcomes.
pub(super) fn playback_source_label(library_resolved: bool) -> String {
    if library_resolved { "Library" } else { "USB" }.to_string()
}

pub(super) fn is_recoverable_playback_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "busy", "already", "in use", "device", "stream", "sink", "playing",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) fn play_native_with_recovery(
    playback: &PlaybackController,
    path: &str,
    start_offset_ms: Option<u64>,
    start_ratio: Option<f64>,
    start_paused: bool,
) -> BackendResult<PlaybackStatusData> {
    match playback.play_path(path, start_offset_ms, start_ratio, start_paused) {
        Ok(status) => Ok(status),
        Err(err) => {
            if !is_recoverable_playback_error(&err.to_string()) {
                return Err(err);
            }
            let _ = playback.stop();
            playback.play_path(path, start_offset_ms, start_ratio, start_paused)
        }
    }
}

// Plain data assembly for the three playback-resolution outcomes below
// (library, USB, USB-after-library-failure).
pub(super) fn play_resolved_track_data(
    status: PlaybackStatusData,
    requested_path: &str,
    track_id: Option<String>,
    matched_by: TrackMatch,
    source: PlaybackSource,
    source_label: String,
    has_usb_context: bool,
) -> PlayResolvedTrackData {
    let library_resolved = source == PlaybackSource::Library;
    PlayResolvedTrackData {
        path: status.path.unwrap_or_else(|| requested_path.to_string()),
        playing: status.playing,
        paused: status.paused,
        position_ms: status.position_ms,
        duration_ms: status.duration_ms,
        track_id,
        matched_by,
        source,
        source_label,
        library_resolved,
        has_usb_context,
    }
}

impl BackendService {
    pub fn play_resolved_track(
        &self,
        playback: &PlaybackController,
        req: PlayResolvedTrackRequest,
    ) -> BackendResult<PlayResolvedTrackData> {
        let resolved = self.resolve_playback_source(ResolvePlaybackSourceRequest {
            title: req.title.clone(),
            artist: req.artist.clone(),
            album: req.album.clone(),
            bpm: req.bpm,
            file_path: req.file_path.clone(),
            file_size_bytes: req.file_size_bytes,
            track_id: req.track_id.clone(),
        })?;
        let matched_by = resolved.matched_by;
        let library_path = resolved
            .resolved_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .filter(|_| matched_by.is_match())
            .map(|path| path.trim().to_string());
        let has_usb_context = req.usb_root_valid
            && req
                .usb_root
                .as_deref()
                .map(|root| !root.trim().is_empty())
                .unwrap_or(false);
        let usb_path = if has_usb_context {
            let track_path = req.file_path.as_deref().unwrap_or("").trim();
            let root = req.usb_root.as_deref().unwrap_or("").trim();
            if !track_path.is_empty() && browse_path_matches_root(track_path, root) {
                Some(track_path.to_string())
            } else {
                None
            }
        } else {
            None
        };

        if library_path.is_none() && usb_path.is_none() {
            return Err(BackendError::NotFound(
                "track not found in Library or selected USB".to_string(),
            ));
        }

        if let Some(path) = library_path.as_deref() {
            match play_native_with_recovery(
                playback,
                path,
                req.start_offset_ms,
                req.start_ratio,
                req.start_paused,
            ) {
                Ok(status) => {
                    return Ok(play_resolved_track_data(
                        status,
                        path,
                        resolved.track_id.or(req.track_id),
                        matched_by,
                        PlaybackSource::Library,
                        playback_source_label(true),
                        has_usb_context,
                    ));
                }
                Err(library_err) => {
                    if let Some(usb_fallback) = usb_path
                        .as_deref()
                        .filter(|usb_path| browse_path_key(usb_path) != browse_path_key(path))
                    {
                        crate::backend_log!(
                            Warn,
                            "playback",
                            "library copy failed to play ({matched_by}), fell back to USB: {library_err} [library path: {path}]"
                        );
                        let status = play_native_with_recovery(
                            playback,
                            usb_fallback,
                            req.start_offset_ms,
                            req.start_ratio,
                            req.start_paused,
                        )?;
                        return Ok(play_resolved_track_data(
                            status,
                            usb_fallback,
                            req.track_id,
                            matched_by,
                            PlaybackSource::Usb,
                            playback_source_label(false),
                            has_usb_context,
                        ));
                    }
                    return Err(library_err);
                }
            }
        }

        if let Some(path) = usb_path.as_deref() {
            let status = play_native_with_recovery(
                playback,
                path,
                req.start_offset_ms,
                req.start_ratio,
                req.start_paused,
            )?;
            return Ok(play_resolved_track_data(
                status,
                path,
                req.track_id,
                matched_by,
                PlaybackSource::Usb,
                playback_source_label(false),
                has_usb_context,
            ));
        }

        Err(BackendError::NotFound(
            "track not found in Library or selected USB".to_string(),
        ))
    }

    pub fn stop_playback_native(
        &self,
        playback: &PlaybackController,
    ) -> BackendResult<StopPlaybackData> {
        let status = playback.stop()?;
        Ok(StopPlaybackData {
            stopped: true,
            previous_path: status.path,
        })
    }

    pub fn pause_playback_native(
        &self,
        playback: &PlaybackController,
    ) -> BackendResult<PlaybackStatusData> {
        playback.pause()
    }

    pub fn resume_playback_native(
        &self,
        playback: &PlaybackController,
    ) -> BackendResult<PlaybackStatusData> {
        playback.resume()
    }

    pub fn get_playback_status_native(
        &self,
        playback: &PlaybackController,
    ) -> BackendResult<PlaybackStatusData> {
        playback.status()
    }

    pub fn set_playback_metronome(
        &self,
        playback: &PlaybackController,
        req: SetPlaybackMetronomeRequest,
    ) -> BackendResult<PlaybackMetronomeData> {
        let enabled = playback.set_metronome(
            req.enabled,
            req.first_beat_ms.unwrap_or(0.0),
            req.bpm.unwrap_or(0.0),
            req.mix.unwrap_or(crate::metronome::DEFAULT_MIX),
        );
        Ok(PlaybackMetronomeData { enabled })
    }

    pub fn resolve_playback_source(
        &self,
        req: ResolvePlaybackSourceRequest,
    ) -> BackendResult<ResolvePlaybackSourceData> {
        let conn = self.db.connect()?;
        let usb_paths = UsbPathCheck::load(&conn)?;
        let is_usb_rooted = |path: &str| usb_paths.is_usb_path(path);

        // Fast path: any origin (library, playlist, USB, history) may carry
        // the id of the row it was dispatched from. If that row is a
        // genuine local track whose file is still on disk, resolve to it
        // directly with a single indexed lookup -- no fingerprint scan
        // needed. If it turns out to be USB-rooted (e.g. a playlist entry
        // created before this fix, still pointing at a stale placeholder),
        // or its file has since moved/been deleted, fall through to the
        // fingerprint/title search below so it can self-heal onto a live
        // library copy. The stale row is kept as a last-resort fallback in
        // case the search finds nothing better.
        let mut weak_self: Option<ResolvePlaybackSourceData> = None;
        if let Some(id) = req.track_id.as_deref().filter(|id| !id.trim().is_empty()) {
            let mut stmt =
                conn.prepare(&format!("SELECT {TRACK_COLS} FROM tracks WHERE id = ?1"))?;
            let track = stmt
                .query_row(params![id], |row| row_to_track(row, false))
                .optional()?;
            if let Some(track) = track
                && !is_usb_rooted(&track.file_path)
            {
                let data = ResolvePlaybackSourceData {
                    resolved_path: Some(track.file_path.clone()),
                    matched_by: TrackMatch::SelfId,
                    track_id: Some(track.id),
                };
                if stored_file_path(&track.file_path).is_file() {
                    return Ok(data);
                }
                weak_self = Some(data);
            }
        }

        // The request's own file plays when it's a local file on disk: a
        // title/artist match could be another copy, or a stale one on an
        // exported stick.
        if let Some(path) = req
            .file_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty() && !is_usb_rooted(path))
            .filter(|path| stored_file_path(path).is_file())
        {
            let track_id = conn
                .query_row(
                    "SELECT id FROM tracks WHERE file_path = ?1",
                    params![path],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            return Ok(ResolvePlaybackSourceData {
                resolved_path: Some(path.to_string()),
                matched_by: if track_id.is_some() {
                    TrackMatch::SelfId
                } else {
                    TrackMatch::OwnFile
                },
                track_id,
            });
        }

        let title = req.title.trim();
        let artist = req.artist.trim();
        if title.is_empty() || artist.is_empty() {
            return Ok(weak_self.unwrap_or(ResolvePlaybackSourceData {
                resolved_path: None,
                matched_by: TrackMatch::NoMatch,
                track_id: None,
            }));
        }

        let fingerprint = build_track_match_fingerprint(title, artist, req.album.as_deref());
        let mut stmt = conn.prepare(
            &format!("SELECT {TRACK_COLS} FROM tracks WHERE match_fingerprint = ?1 ORDER BY updated_at DESC LIMIT 64"),
        )?;
        let rows = stmt.query_map(params![fingerprint], |row| row_to_track(row, false))?;
        let candidates = rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|t| !is_usb_rooted(&t.file_path))
            .collect::<Vec<_>>();

        if let Some(track) = best_candidate(candidates, &req) {
            return Ok(ResolvePlaybackSourceData {
                resolved_path: Some(track.file_path.clone()),
                matched_by: TrackMatch::Hash,
                track_id: Some(track.id),
            });
        }

        let like = format!("%{}%", title);
        let mut stmt = conn.prepare(&format!(
            "SELECT {TRACK_COLS} FROM tracks WHERE title LIKE ?1 ORDER BY updated_at DESC LIMIT 200"
        ))?;
        let rows = stmt.query_map(params![like], |row| row_to_track(row, false))?;
        let candidates = rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|t| !is_usb_rooted(&t.file_path))
            .collect::<Vec<_>>();
        if let Some(track) = best_candidate(candidates, &req) {
            return Ok(ResolvePlaybackSourceData {
                resolved_path: Some(track.file_path.clone()),
                matched_by: TrackMatch::Metadata,
                track_id: Some(track.id),
            });
        }

        Ok(weak_self.unwrap_or(ResolvePlaybackSourceData {
            resolved_path: None,
            matched_by: TrackMatch::NoMatch,
            track_id: None,
        }))
    }
}

pub(super) fn best_candidate(
    candidates: Vec<Track>,
    req: &ResolvePlaybackSourceRequest,
) -> Option<Track> {
    // Track the best-scoring candidate whose file is actually on disk
    // separately from the best overall. A stale/duplicate library row
    // pointing at a moved-or-deleted file must never win over a live copy
    // of the same track, however well it scores -- but if nothing is on
    // disk we still return the best guess so the caller's USB fallback (and
    // the "file not found" surfacing) can take over.
    let mut best: Option<Track> = None;
    let mut best_score = -1i32;
    let mut best_on_disk: Option<Track> = None;
    let mut best_on_disk_score = -1i32;
    for candidate in candidates {
        if let Some(source_path) = req.file_path.as_deref()
            && !source_path.is_empty()
            && !same_format_extension(source_path, &candidate.file_path)
        {
            continue;
        }
        // A known file-size mismatch is stronger evidence of "different
        // file" than any positive score contribution elsewhere can
        // outweigh -- veto outright rather than merely downweighting.
        if let (Some(req_size), Some(candidate_size)) =
            (req.file_size_bytes, candidate.file_size_bytes)
            && req_size != candidate_size
        {
            continue;
        }
        let score = score_playback_candidate(&candidate, req);
        if score > best_on_disk_score && stored_file_path(&candidate.file_path).is_file() {
            best_on_disk_score = score;
            best_on_disk = Some(candidate.clone());
        }
        if score > best_score {
            best_score = score;
            best = Some(candidate);
        }
    }
    best_on_disk
        .filter(|_| best_on_disk_score >= 24)
        .or_else(|| best.filter(|_| best_score >= 24))
}

pub(super) fn score_playback_candidate(track: &Track, req: &ResolvePlaybackSourceRequest) -> i32 {
    let mut score = 0;
    if normalize_hash_part(&track.title) == normalize_hash_part(&req.title) {
        score += 12;
    }
    if normalize_hash_part(&track.artist) == normalize_hash_part(&req.artist) {
        score += 12;
    }
    if normalize_hash_part(track.album.as_deref().unwrap_or_default())
        == normalize_hash_part(req.album.as_deref().unwrap_or_default())
    {
        score += 8;
    }
    if let Some(src_path) = req.file_path.as_deref() {
        if matching_file_name(src_path) == matching_file_name(&track.file_path) {
            score += 16;
        }
        if matching_file_stem(src_path) == matching_file_stem(&track.file_path) {
            score += 8;
        }
    }
    if let (Some(a), Some(b)) = (req.bpm, track.bpm)
        && (a - b).abs() <= 0.15
    {
        score += 4;
    }
    score
}
