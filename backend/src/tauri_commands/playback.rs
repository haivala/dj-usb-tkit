//! Playback commands and the playback events relayed to the UI.

use super::jobs::{panic_message, run_blocking_command};
use crate::commands::BackendCommands;
use crate::error::{ErrorCode, ErrorPayload};
use crate::models::{
    ApiResponse, PlayResolvedTrackData, PlayResolvedTrackRequest, PlaybackEventPayload,
    PlaybackMetronomeData, PlaybackStatusData, SetPlaybackMetronomeRequest, StopPlaybackData,
};
use chrono::Utc;
use serde::Serialize;
use std::panic::{self, AssertUnwindSafe};
use std::thread;
use tauri::{AppHandle, Emitter, Manager, State};

pub(super) const PLAYBACK_EVENT_CHANNEL: &str = "playback:event";

pub(super) fn playback_panic_response<T: serde::Serialize>(
    app: &AppHandle,
    action: &str,
    panic_text: String,
) -> ApiResponse<T> {
    let message = format!("native playback {action} crashed: {panic_text}");
    let _ = app.emit(
        "backend:log",
        serde_json::json!({
            "level": "error",
            "source": "playback",
            "code": "playback.crash",
            "message": message,
            "timestamp": Utc::now().to_rfc3339(),
        }),
    );
    ApiResponse::failure(ErrorPayload::new(ErrorCode::InternalError, message))
}

pub(super) async fn run_playback_blocking<T, F>(
    app: &AppHandle,
    action: &str,
    task: F,
) -> Result<ApiResponse<T>, ApiResponse<T>>
where
    T: Send + Serialize + 'static,
    F: FnOnce() -> ApiResponse<T> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(move || panic::catch_unwind(AssertUnwindSafe(task)))
        .await
    {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => {
            let panic_text = panic_message(e);
            emit_playback_event(
                app,
                PlaybackEvent::Error(Some(format!(
                    "Native playback {action} crashed: {panic_text}"
                ))),
            );
            Err(playback_panic_response(app, action, panic_text))
        }
        Err(e) => {
            let message = format!("native playback {action} task failed: {e}");
            emit_playback_event(app, PlaybackEvent::Error(Some(message.clone())));
            Err(ApiResponse::failure(ErrorPayload::new(
                ErrorCode::InternalError,
                message,
            )))
        }
    }
}

/// The three playback notifications the frontend listens for. Each carries only
/// the fields that are meaningful for it -- the flat `PlaybackEventPayload` wire
/// shape (with its inert `None`/`false`/`0` columns) is assembled here, once.
pub(super) enum PlaybackEvent {
    /// `playback.started`, or `playback.seeked` when `is_seek`.
    Started {
        is_seek: bool,
        path: String,
        playing: bool,
        position_ms: u64,
        duration_ms: Option<u64>,
        track_id: Option<String>,
    },
    Stopped {
        path: Option<String>,
        duration_ms: Option<u64>,
    },
    /// `playback.paused` / `playback.resumed`: the worker's status right after
    /// the pause/resume was applied.
    PauseChanged(PlaybackStatusData),
    Error(Option<String>),
}

pub(super) fn emit_playback_event<R: tauri::Runtime>(app: &AppHandle<R>, event: PlaybackEvent) {
    let payload = match event {
        PlaybackEvent::Started {
            is_seek,
            path,
            playing,
            position_ms,
            duration_ms,
            track_id,
        } => PlaybackEventPayload {
            event: if is_seek {
                "playback.seeked"
            } else {
                "playback.started"
            }
            .to_string(),
            path: Some(path),
            playing,
            paused: false,
            position_ms,
            duration_ms,
            message: None,
            track_id,
            timestamp: Utc::now().to_rfc3339(),
        },
        PlaybackEvent::Stopped { path, duration_ms } => PlaybackEventPayload {
            event: "playback.stopped".to_string(),
            path,
            playing: false,
            paused: false,
            position_ms: 0,
            duration_ms,
            message: None,
            track_id: None,
            timestamp: Utc::now().to_rfc3339(),
        },
        PlaybackEvent::PauseChanged(status) => PlaybackEventPayload {
            event: if status.paused {
                "playback.paused"
            } else {
                "playback.resumed"
            }
            .to_string(),
            path: status.path,
            playing: status.playing,
            paused: status.paused,
            position_ms: status.position_ms,
            duration_ms: status.duration_ms,
            message: None,
            track_id: None,
            timestamp: Utc::now().to_rfc3339(),
        },
        PlaybackEvent::Error(message) => PlaybackEventPayload {
            event: "playback.error".to_string(),
            path: None,
            playing: false,
            paused: false,
            position_ms: 0,
            duration_ms: None,
            message,
            track_id: None,
            timestamp: Utc::now().to_rfc3339(),
        },
    };

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(PLAYBACK_EVENT_CHANNEL, payload.clone());
    }
    let _ = app.emit(PLAYBACK_EVENT_CHANNEL, payload);
}

/// Relays natural end-of-track notifications from the playback worker thread to the
/// frontend. Purely reactive — blocks on `recv()`, no polling loop — the worker thread
/// itself (`player.rs`) is what detects a sink emptying on its own, using the same
/// serialized loop that processes explicit Play/Stop commands, so this relay can never
/// race an explicit stop the way a separate status-polling thread could.
pub fn start_playback_transition_relay(
    app: AppHandle,
    transitions: std::sync::mpsc::Receiver<crate::player::PlaybackTransition>,
) {
    thread::spawn(move || {
        while let Ok(transition) = transitions.recv() {
            emit_playback_event(
                &app,
                PlaybackEvent::Stopped {
                    path: transition.path,
                    duration_ms: transition.duration_ms,
                },
            );
        }
        // The channel only disconnects if the playback worker thread exited (panicked or
        // the process is shutting down) — natural end-of-track detection is silently gone
        // for the rest of the session, so this needs to be visible, not just dropped.
        let _ = app.emit(
            "backend:log",
            serde_json::json!({
                "level": "warn",
                "source": "playback",
                "code": "playback.transition_relay_stopped",
                "message": "playback transition relay stopped: worker channel disconnected; natural end-of-track detection is no longer active",
                "timestamp": Utc::now().to_rfc3339(),
            }),
        );
    });
}

#[tauri::command]
pub async fn play_resolved_track(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    request: PlayResolvedTrackRequest,
) -> Result<ApiResponse<PlayResolvedTrackData>, String> {
    let commands = state.inner().clone();
    let is_seek = request.start_offset_ms.unwrap_or(0) > 0
        || request
            .start_ratio
            .map(|ratio| ratio > 0.0)
            .unwrap_or(false);
    let request_for_call = request.clone();
    let response = match run_playback_blocking(&app, "start", move || {
        commands.play_resolved_track(request_for_call)
    })
    .await
    {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if let Some(data) = response.data.as_ref() {
        // A `start_paused` load/seek is a pause change, not a start: a
        // `playback.started` with `playing: false` would read as stopped.
        let event = if data.paused {
            PlaybackEvent::PauseChanged(PlaybackStatusData {
                path: Some(data.path.clone()),
                playing: false,
                paused: true,
                position_ms: data.position_ms,
                duration_ms: data.duration_ms,
            })
        } else {
            PlaybackEvent::Started {
                is_seek,
                path: data.path.clone(),
                playing: data.playing,
                position_ms: data.position_ms,
                duration_ms: data.duration_ms,
                track_id: data.track_id.clone(),
            }
        };
        emit_playback_event(&app, event);
    } else {
        emit_playback_event(
            &app,
            PlaybackEvent::Error(response.error.as_ref().map(|e| e.message.clone())),
        );
    }
    Ok(response)
}

#[tauri::command]
pub async fn stop_playback_native(
    app: AppHandle,
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<StopPlaybackData>, String> {
    let commands = state.inner().clone();
    let response =
        match run_playback_blocking(&app, "stop", move || commands.stop_playback_native()).await {
            Ok(r) => r,
            Err(r) => return Ok(r),
        };
    if let Some(data) = response.data.as_ref() {
        emit_playback_event(
            &app,
            PlaybackEvent::Stopped {
                path: data.previous_path.clone(),
                duration_ms: None,
            },
        );
    } else {
        emit_playback_event(
            &app,
            PlaybackEvent::Error(response.error.as_ref().map(|e| e.message.clone())),
        );
    }
    Ok(response)
}

/// Shared body of pause/resume: run on the playback thread, then broadcast the
/// resulting status so every listener sees the same backend-owned state.
pub(super) async fn run_pause_change(
    app: AppHandle,
    commands: BackendCommands,
    label: &'static str,
    op: fn(&BackendCommands) -> ApiResponse<PlaybackStatusData>,
) -> Result<ApiResponse<PlaybackStatusData>, String> {
    let response = match run_playback_blocking(&app, label, move || op(&commands)).await {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    match response.data.as_ref() {
        // Only a loaded track has a pause state worth announcing; pausing when
        // idle is a quiet no-op.
        Some(data) if data.path.is_some() && (data.playing || data.paused) => {
            emit_playback_event(&app, PlaybackEvent::PauseChanged(data.clone()));
        }
        Some(_) => {}
        None => emit_playback_event(
            &app,
            PlaybackEvent::Error(response.error.as_ref().map(|e| e.message.clone())),
        ),
    }
    Ok(response)
}

#[tauri::command]
pub async fn pause_playback_native(
    app: AppHandle,
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<PlaybackStatusData>, String> {
    let commands = state.inner().clone();
    run_pause_change(
        app,
        commands,
        "pause",
        BackendCommands::pause_playback_native,
    )
    .await
}

#[tauri::command]
pub async fn resume_playback_native(
    app: AppHandle,
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<PlaybackStatusData>, String> {
    let commands = state.inner().clone();
    run_pause_change(
        app,
        commands,
        "resume",
        BackendCommands::resume_playback_native,
    )
    .await
}

#[tauri::command]
pub async fn get_playback_status_native(
    state: State<'_, BackendCommands>,
) -> Result<ApiResponse<PlaybackStatusData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("native playback status", move || {
        commands.get_playback_status_native()
    })
    .await)
}

#[tauri::command]
pub async fn set_playback_metronome(
    state: State<'_, BackendCommands>,
    request: SetPlaybackMetronomeRequest,
) -> Result<ApiResponse<PlaybackMetronomeData>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("playback metronome", move || {
        commands.set_playback_metronome(request)
    })
    .await)
}
