//! What the analysis coordinator's events look like to whoever shows them: the
//! same `job:event` payloads as every other job. No Tauri here, so the mapping
//! is tested with the rest of the backend.

use chrono::Utc;

use super::AnalyzeTrackProgress;
use super::coordinator::AnalysisEvent;
use crate::models::JobEventPayload;

const JOB_TYPE: &str = "analysis";
const STAGE: &str = "analyze_new_tracks";

fn job_payload(event: &str, job_id: &str, percent: usize, message: String) -> JobEventPayload {
    JobEventPayload {
        event: event.to_string(),
        job_id: job_id.to_string(),
        job_type: JOB_TYPE.to_string(),
        stage: STAGE.to_string(),
        current: 0,
        total: 1,
        percent,
        message,
        track_id: None,
        track_title: None,
        file_path: None,
        bpm: None,
        bpm_source_text: None,
        key: None,
        key_display: None,
        key_color: None,
        artwork_path: None,
        waveform_peaks_path: None,
        waveform_preview: None,
        duration_ms: None,
        track_ready: None,
        track_started: None,
        reanalysis: None,
        failed: None,
        analysis_ready: None,
        error_message: None,
        library_total_duration_ms: None,
        library_duration_unknown_count: None,
        track: None,
        waiting_track_ids: None,
        waiting_tooltip: None,
        warning_count: None,
        timestamp: Utc::now().to_rfc3339(),
    }
}

fn track_payload(job_id: &str, progress: &AnalyzeTrackProgress) -> JobEventPayload {
    let total = progress.total.max(1);
    let file_name = std::path::Path::new(&progress.file_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&progress.file_path);
    JobEventPayload {
        current: progress.current,
        total,
        track_id: Some(progress.track_id.clone()),
        track_title: Some(progress.track_title.clone()),
        file_path: Some(progress.file_path.clone()),
        bpm: progress.bpm,
        bpm_source_text: crate::service::settings::bpm_source_text(
            progress.bpm_analyzer.as_deref(),
        ),
        key: progress.key.clone(),
        key_display: progress.key_display.clone(),
        key_color: progress.key_color,
        artwork_path: progress.artwork_path.clone(),
        waveform_peaks_path: progress.waveform_peaks_path.clone(),
        waveform_preview: progress.waveform_preview.clone(),
        duration_ms: progress.duration_ms,
        track_ready: Some(progress.track_ready),
        track_started: Some(progress.track_started),
        reanalysis: Some(progress.reanalysis),
        failed: Some(progress.failed),
        analysis_ready: Some(progress.analysis_ready),
        error_message: progress.error_message.clone(),
        library_total_duration_ms: progress.library_total_duration_ms,
        library_duration_unknown_count: progress.library_duration_unknown_count,
        track: progress.track.clone().map(Box::new),
        ..job_payload(
            "job.progress",
            job_id,
            (progress.current * 100 / total).min(100),
            crate::i18n::tr::analysis_progress(progress.current, progress.total, file_name)
                .render(),
        )
    }
}

/// The `job:event` an analysis event becomes; its `event` field names it.
pub fn analysis_event_payload(event: &AnalysisEvent) -> JobEventPayload {
    match event {
        AnalysisEvent::JobStarted { job_id, total } => JobEventPayload {
            total: (*total).max(1),
            ..job_payload(
                "job.started",
                job_id,
                0,
                crate::i18n::tr::analysis_started().render(),
            )
        },
        AnalysisEvent::TracksWaiting {
            job_id,
            track_ids,
            tooltip,
        } => JobEventPayload {
            waiting_track_ids: Some(track_ids.clone()),
            waiting_tooltip: Some(tooltip.clone()),
            ..job_payload("job.tracks_waiting", job_id, 0, String::new())
        },
        AnalysisEvent::TracksNotWaiting { job_id, track_ids } => JobEventPayload {
            waiting_track_ids: Some(track_ids.clone()),
            ..job_payload("job.tracks_not_waiting", job_id, 0, String::new())
        },
        AnalysisEvent::Track { job_id, progress } => track_payload(job_id, progress),
        AnalysisEvent::JobFinished(summary) => JobEventPayload {
            current: 1,
            warning_count: Some(summary.warning_count),
            ..job_payload(
                "job.completed",
                &summary.job_id,
                100,
                summary.message.clone(),
            )
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::service::analysis::coordinator::AnalysisJobSummary;

    const JOB_MANAGER: &str = include_str!("../../../../vanilla-ui/job_manager.mjs");

    fn sent(event: &AnalysisEvent) -> Value {
        serde_json::to_value(analysis_event_payload(event)).expect("serializes")
    }

    // What the UI reads has to be what is sent: e2e mocks only repeat the names.
    fn assert_ui_reads(names: &[&str]) {
        for name in names {
            assert!(JOB_MANAGER.contains(name), "job_manager.mjs reads {name}");
        }
    }

    fn progress() -> AnalyzeTrackProgress {
        AnalyzeTrackProgress {
            current: 1,
            total: 4,
            track_id: "t1".to_string(),
            track_title: "Title".to_string(),
            file_path: "/music/Song.mp3".to_string(),
            bpm: Some(128.0),
            bpm_analyzer: None,
            key: None,
            key_display: None,
            key_color: None,
            artwork_path: None,
            waveform_peaks_path: None,
            waveform_preview: None,
            duration_ms: None,
            track_ready: true,
            track_started: false,
            reanalysis: false,
            failed: false,
            analysis_ready: true,
            error_message: None,
            library_total_duration_ms: None,
            library_duration_unknown_count: None,
            track: None,
        }
    }

    #[test]
    fn job_start_and_track_events_reach_the_ui_as_it_reads_them() {
        let started = sent(&AnalysisEvent::JobStarted {
            job_id: "j".to_string(),
            total: 3,
        });
        assert_eq!(started["event"], "job.started");
        assert_eq!(started["jobId"], "j");
        assert_eq!(started["jobType"], "analysis");
        assert_eq!(started["stage"], "analyze_new_tracks");
        assert_eq!(started["total"], 3);
        assert_ui_reads(&["job.started", "\"analysis\"", "analyze_new_tracks"]);

        let track = sent(&AnalysisEvent::Track {
            job_id: "j".to_string(),
            progress: Box::new(progress()),
        });
        assert_eq!(track["event"], "job.progress");
        assert_eq!(track["trackId"], "t1");
        assert_eq!(track["trackReady"], true);
        assert_eq!(track["percent"], 25);
        assert_eq!(track["message"], "Analyzing 1/4: Song.mp3");
        assert_ui_reads(&["job.progress", "payload.trackId", "payload.trackReady"]);
    }

    #[test]
    fn waiting_events_reach_the_ui_as_it_reads_them() {
        let waiting = sent(&AnalysisEvent::TracksWaiting {
            job_id: "j".to_string(),
            track_ids: vec!["t1".to_string()],
            tooltip: "Waiting".to_string(),
        });
        assert_eq!(waiting["event"], "job.tracks_waiting");
        assert_eq!(waiting["waitingTrackIds"], json!(["t1"]));
        assert_eq!(waiting["waitingTooltip"], "Waiting");

        let not_waiting = sent(&AnalysisEvent::TracksNotWaiting {
            job_id: "j".to_string(),
            track_ids: vec!["t1".to_string()],
        });
        assert_eq!(not_waiting["event"], "job.tracks_not_waiting");
        assert_eq!(not_waiting["waitingTrackIds"], json!(["t1"]));
        assert_ui_reads(&[
            "job.tracks_waiting",
            "job.tracks_not_waiting",
            "payload.waitingTrackIds",
            "payload.waitingTooltip",
        ]);
    }

    #[test]
    fn the_job_end_reaches_the_ui_as_it_reads_it() {
        let finished = sent(&AnalysisEvent::JobFinished(AnalysisJobSummary {
            job_id: "j".to_string(),
            warnings: Vec::new(),
            warning_count: 2,
            message: "Analysis finished".to_string(),
        }));
        assert_eq!(finished["event"], "job.completed");
        assert_eq!(finished["percent"], 100);
        assert_eq!(finished["message"], "Analysis finished");
        assert_eq!(finished["warningCount"], 2);
        assert_ui_reads(&["job.completed", "payload.warningCount"]);
    }
}
