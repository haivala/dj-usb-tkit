//! The process-wide language: saving it switches every thread, cached USB reads answer in it,
//! and errors and warnings show it while the log gets English.
//!
//! In its own test binary (its own process) because the language is process-wide: switching it
//! beside the tests that assert English would break them. The tests here take `LANGUAGE_LOCK`,
//! so they don't run in parallel with each other either, and put English back when done.

mod common;
mod library_sources;
mod parity_fixture;

use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError, mpsc};
use std::time::Duration;

use backend::commands::BackendCommands;
use backend::i18n::{self, ENGLISH, TEST_LANGUAGE, tr};
use backend::models::{
    AnalyzeNewTracksRequest, CreatePlaylistRequest, DeletePlaylistRequest,
    FetchUsbHistoriesRequest, InitializeUsbRequest, RepairUsbDiagnosticsRequest,
    RunUsbDiagnosticsData, RunUsbDiagnosticsRequest, RunUsbParityReportData,
    RunUsbParityReportRequest, ScanLibraryRequest, SetFrontendSettingRequest,
    ValidateUsbRootRequest,
};
use backend::service::analysis_event_payload;
use tempfile::tempdir;

const LANGUAGE_SETTING: &str = "ui_language_v1";

static LANGUAGE_LOCK: Mutex<()> = Mutex::new(());

/// Holds the lock, and puts English back when the test ends (also on a failed assert).
struct LanguageTest {
    _lock: MutexGuard<'static, ()>,
}

impl LanguageTest {
    fn start() -> Self {
        let lock = LANGUAGE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        i18n::set_language(ENGLISH).expect("English");
        Self { _lock: lock }
    }
}

impl Drop for LanguageTest {
    fn drop(&mut self) {
        let _ = i18n::set_language(ENGLISH);
    }
}

fn save_language(
    backend: &BackendCommands,
    language: &str,
) -> backend::models::ApiResponse<backend::models::SetFrontendSettingData> {
    backend.set_frontend_setting(SetFrontendSettingRequest {
        key: LANGUAGE_SETTING.to_string(),
        value: Some(language.to_string()),
    })
}

/// Every line logged in this test binary, as (Event Log text, log file text). The sink is
/// process-wide and installed once, so the tests share it and look for their own lines.
fn logged_lines() -> &'static Mutex<Vec<(String, String)>> {
    static LINES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    static SINK: Once = Once::new();
    SINK.call_once(|| {
        backend::logging::set_sink(Box::new(|line| {
            LINES
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((line.message.to_string(), line.english.to_string()));
        }));
    });
    &LINES
}

fn is_test_language(text: &str) -> bool {
    text.starts_with('[') && text.ends_with(']') && !text.contains(['\u{2068}', '\u{2069}'])
}

#[test]
fn saving_the_language_switches_it_for_every_thread() {
    let _test = LanguageTest::start();
    let root = tempdir().expect("temp root");
    let backend = BackendCommands::new(root.path().join("data")).expect("create backend");

    let saved = save_language(&backend, TEST_LANGUAGE);
    assert!(saved.ok, "{saved:?}");
    assert_eq!(i18n::current_language(), TEST_LANGUAGE);

    // Commands run on worker threads (Tauri's blocking pool, the analysis coordinator); the
    // language isn't per thread.
    let from_worker = std::thread::spawn(|| tr::status_warning_count(2usize).to_string())
        .join()
        .expect("worker thread");
    assert!(is_test_language(&from_worker), "{from_worker:?}");
    assert!(from_worker.contains('2'), "{from_worker:?}");
    assert_eq!(
        tr::status_warning_count(2usize).render_english(),
        "(2 warnings)"
    );

    // Clearing the setting is English again.
    let cleared = backend.set_frontend_setting(SetFrontendSettingRequest {
        key: LANGUAGE_SETTING.to_string(),
        value: None,
    });
    assert!(cleared.ok, "{cleared:?}");
    assert_eq!(i18n::current_language(), ENGLISH);
}

#[test]
fn the_saved_language_is_applied_at_startup() {
    let _test = LanguageTest::start();
    let root = tempdir().expect("temp root");
    let data_dir = root.path().join("data");
    let backend = BackendCommands::new(&data_dir).expect("create backend");
    assert!(save_language(&backend, TEST_LANGUAGE).ok);

    // A new process starts in English until the desktop app applies the saved language.
    i18n::set_language(ENGLISH).expect("English");
    let restarted = BackendCommands::new(&data_dir).expect("create backend again");
    assert_eq!(i18n::current_language(), ENGLISH);
    let applied = restarted.apply_saved_language();
    assert!(applied.ok, "{applied:?}");
    assert_eq!(i18n::current_language(), TEST_LANGUAGE);
}

#[test]
fn a_cached_usb_read_answers_in_the_current_language() {
    let _test = LanguageTest::start();
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    std::fs::create_dir_all(&usb).expect("create usb dir");
    let backend = BackendCommands::new(root.path().join("data")).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "{initialized:?}");
    let read = || {
        backend
            .fetch_usb_histories(FetchUsbHistoriesRequest {
                usb_root: Some(usb.to_string_lossy().to_string()),
            })
            .data
            .expect("history data")
    };

    let english = read();
    assert_eq!(english.summary_text, "0 sessions, 0 tracks");
    assert_eq!(english.status_text, "USB histories loaded: 0");

    // Saving the language clears the parse cache, whose warnings were logged in the old one;
    // the texts are made when answering either way.
    assert!(save_language(&backend, TEST_LANGUAGE).ok);
    let switched = read();
    assert!(
        is_test_language(&switched.summary_text),
        "{:?}",
        switched.summary_text
    );
    assert!(
        is_test_language(&switched.status_text),
        "{:?}",
        switched.status_text
    );

    assert!(save_language(&backend, ENGLISH).ok);
    assert_eq!(read().summary_text, "0 sessions, 0 tracks");
}

#[test]
fn an_error_shows_the_current_language_and_logs_english() {
    let _test = LanguageTest::start();
    let logged = logged_lines();
    let root = tempdir().expect("temp root");
    let backend = BackendCommands::new(root.path().join("data")).expect("create backend");
    assert!(save_language(&backend, TEST_LANGUAGE).ok);

    let refused = save_language(&backend, "xx");
    assert!(!refused.ok);
    let shown = refused.error.expect("error").message;
    assert!(is_test_language(&shown), "{shown:?}");
    assert_eq!(
        i18n::current_language(),
        TEST_LANGUAGE,
        "a refused language changes nothing"
    );

    // A command's own errors: the user's input, and something that isn't there.
    let empty_name = backend.create_playlist(CreatePlaylistRequest {
        name: "  ".to_string(),
    });
    let empty_name = empty_name.error.expect("empty name refused").message;
    assert!(is_test_language(&empty_name), "{empty_name:?}");
    let missing = backend.delete_playlist(DeletePlaylistRequest {
        playlist_id: "no-such-playlist".to_string(),
    });
    let missing = missing.error.expect("missing playlist").message;
    assert!(is_test_language(&missing), "{missing:?}");
    assert!(missing.contains("no-such-playlist"), "{missing:?}");

    let lines = logged.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(
        lines.contains(&(shown.clone(), "Unknown language: xx".to_string())),
        "{lines:?}"
    );
    assert!(
        lines.contains(&(empty_name, "playlist name must not be empty".to_string())),
        "{lines:?}"
    );
    assert!(
        lines.contains(&(missing, "playlist not found: no-such-playlist".to_string())),
        "{lines:?}"
    );
}

#[test]
fn a_warning_shows_the_current_language_and_logs_english() {
    let _test = LanguageTest::start();
    let logged = logged_lines();
    let root = tempdir().expect("temp root");
    let backend = BackendCommands::new(root.path().join("data")).expect("create backend");
    assert!(save_language(&backend, TEST_LANGUAGE).ok);

    let missing = root.path().join("no-such-usb");
    let validated = backend
        .validate_usb_root(ValidateUsbRootRequest {
            path: missing.to_string_lossy().to_string(),
        })
        .data
        .expect("validation data");
    assert!(!validated.valid);
    let warning = validated
        .warnings
        .iter()
        .find(|warning| warning.code == "usb.import.path-not-found")
        .expect("path warning");
    assert!(is_test_language(&warning.message), "{warning:?}");
    let shown_path = missing.display().to_string();
    assert!(warning.message.contains(&shown_path), "{warning:?}");

    let lines = logged.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(
        lines.contains(&(
            warning.message.clone(),
            format!("Path does not exist: {shown_path}")
        )),
        "{lines:?}"
    );
}

#[test]
fn diagnostics_report_follows_the_language() {
    let _test = LanguageTest::start();
    let root = tempdir().expect("temp root");
    let usb = root.path().join("usb");
    std::fs::create_dir_all(&usb).expect("create usb dir");
    let backend = BackendCommands::new(root.path().join("data")).expect("create backend");
    let initialized = backend.initialize_usb(InitializeUsbRequest {
        usb_root: usb.to_string_lossy().to_string(),
    });
    assert!(initialized.ok, "{initialized:?}");
    let run = || -> RunUsbDiagnosticsData {
        let response = backend.run_usb_diagnostics(RunUsbDiagnosticsRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
        });
        response.data.expect("diagnostics data")
    };
    // What the report's layout decides: which boxes and columns, and each overview row's
    // status. The overview finds its columns by their (translated) titles.
    let shape = |data: &RunUsbDiagnosticsData| {
        let columns: Vec<String> = data
            .section_groups
            .iter()
            .flat_map(|g| g.sections.iter().map(|s| format!("{:?}", s.status)))
            .collect();
        let rows: Vec<String> = data
            .overview
            .iter()
            .map(|item| format!("{:?}", item.status))
            .collect();
        (columns, rows)
    };

    let english = run();
    assert!(!english.overview.is_empty() && !english.section_groups.is_empty());
    assert_eq!(english.overview[0].title, "Older Pioneer players");

    assert!(save_language(&backend, TEST_LANGUAGE).ok);
    let switched = run();
    assert_eq!(shape(&switched), shape(&english));
    for group in &switched.section_groups {
        assert!(is_test_language(&group.title), "{:?}", group.title);
        for section in &group.sections {
            assert!(is_test_language(&section.title), "{:?}", section.title);
            // A fresh USB has no playlists, so no check is named after one.
            for check in &section.checks {
                assert!(is_test_language(&check.label), "{:?}", check.label);
                // Raw table signals ("t11 23-23 ec=24 | …"): no words to translate.
                let is_signal = [
                    tr::diagnostics_check_history_pointers(),
                    tr::diagnostics_check_primitive_signal(),
                ]
                .iter()
                .any(|label| label.render() == check.label);
                assert!(is_signal || is_test_language(&check.detail), "{check:?}");
            }
        }
    }
    for item in &switched.overview {
        assert!(is_test_language(&item.title), "{:?}", item.title);
        assert!(is_test_language(&item.detail), "{:?}", item.detail);
    }

    // An audio file in no playlist: the preview offers a fix for it.
    let contents = usb.join("Contents");
    std::fs::create_dir_all(&contents).expect("create Contents");
    std::fs::write(contents.join("stray.mp3"), [0u8; 64]).expect("write stray file");
    let repair = backend.repair_usb_diagnostics(RepairUsbDiagnosticsRequest {
        usb_root: Some(usb.to_string_lossy().to_string()),
        apply: false,
        selected_fix_ids: Vec::new(),
    });
    let repair = repair.data.expect("repair preview");
    assert!(is_test_language(&repair.summary), "{:?}", repair.summary);
    assert!(!repair.rows.is_empty(), "{repair:?}");
    for row in &repair.rows {
        for text in [&row.title, &row.description] {
            assert!(is_test_language(text), "{row:?}");
        }
        // The details line's parts, joined with " · ".
        for part in row.meta_text.split(" · ") {
            assert!(is_test_language(part), "{row:?}");
        }
    }
}

#[test]
fn parity_report_follows_the_language() {
    let _test = LanguageTest::start();
    let (_root, backend, usb, playlist_name) = parity_fixture::setup_clean_strict_parity_fixture();
    let run = || -> RunUsbParityReportData {
        let response = backend.run_usb_parity_report(RunUsbParityReportRequest {
            usb_root: Some(usb.to_string_lossy().to_string()),
        });
        response.data.expect("parity data")
    };
    // What the report's layout decides, as in the diagnostics test.
    let shape = |data: &RunUsbParityReportData| {
        let columns: Vec<String> = data
            .section_groups
            .iter()
            .flat_map(|g| g.sections.iter().map(|s| format!("{:?}", s.status)))
            .collect();
        let rows: Vec<String> = data
            .overview
            .iter()
            .map(|item| format!("{:?}", item.status))
            .collect();
        (format!("{:?}", data.overall_status), columns, rows)
    };

    let english = run();
    assert!(!english.overview.is_empty() && !english.section_groups.is_empty());
    assert_eq!(english.playlist_details.len(), 1);

    assert!(save_language(&backend, TEST_LANGUAGE).ok);
    let switched = run();
    assert_eq!(shape(&switched), shape(&english));
    for text in [
        &switched.status_text,
        &switched.duration_text,
        &switched.status_label,
    ] {
        assert!(is_test_language(text), "{text:?}");
    }
    for group in &switched.section_groups {
        assert!(is_test_language(&group.title), "{:?}", group.title);
        for section in &group.sections {
            assert!(is_test_language(&section.title), "{:?}", section.title);
            for check in &section.checks {
                for text in [&check.label, &check.detail] {
                    assert!(is_test_language(text), "{check:?}");
                }
            }
        }
    }
    for item in &switched.overview {
        assert!(is_test_language(&item.title), "{:?}", item.title);
        assert!(is_test_language(&item.detail), "{:?}", item.detail);
    }
    // The playlist's row: its name is data, its issue badges are text.
    let playlist = &switched.playlist_details[0];
    assert_eq!(playlist.name, playlist_name);
    for label in &playlist.issue_labels {
        assert!(is_test_language(label), "{label:?}");
    }
}

#[test]
fn the_analysis_coordinator_answers_in_the_current_language() {
    let _test = LanguageTest::start();
    let root = tempdir().expect("temp root");
    let media = root.path().join("media");
    std::fs::create_dir_all(&media).expect("create media dir");
    // Not real audio: its analysis fails, which also gives the job a warning.
    std::fs::write(media.join("Artist - Title.mp3"), b"audio").expect("write track");
    let backend = BackendCommands::new(root.path().join("data")).expect("create backend");
    let scanned = backend.scan_library(ScanLibraryRequest {
        source_roots: vec![media.to_string_lossy().to_string()],
        incremental: true,
    });
    assert!(scanned.ok, "{scanned:?}");
    library_sources::show_only(
        &backend,
        vec![media.to_string_lossy().to_string()],
        false,
        false,
    );
    let (sender, events) = mpsc::channel();
    let sender = Mutex::new(sender);
    backend.set_analysis_observer(Arc::new(move |event| {
        let _ = sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(analysis_event_payload(event));
    }));
    assert!(save_language(&backend, TEST_LANGUAGE).ok);

    let submitted = backend
        .submit_analysis(AnalyzeNewTracksRequest {
            scope_to_library_filter: true,
            ..Default::default()
        })
        .data
        .expect("submitted");
    assert_eq!(submitted.queued, 1, "{submitted:?}");
    assert!(is_test_language(&submitted.message), "{submitted:?}");

    // The job's events, made on the coordinator's thread.
    let mut seen = Vec::new();
    loop {
        let payload = events
            .recv_timeout(Duration::from_secs(60))
            .expect("analysis event");
        let done = payload.event == "job.completed";
        seen.push(payload.event.clone());
        // The status line's " | (1 warning)" is the Event Log link's text, translated apart.
        for part in payload.message.split(" | ").filter(|part| !part.is_empty()) {
            let part = part.trim_start_matches('(').trim_end_matches(')');
            assert!(is_test_language(part), "{payload:?}");
        }
        if let Some(tooltip) = &payload.waiting_tooltip {
            assert!(is_test_language(tooltip), "{payload:?}");
        }
        if done {
            break;
        }
    }
    for event in ["job.started", "job.tracks_waiting", "job.progress"] {
        assert!(seen.iter().any(|seen| seen == event), "{event}: {seen:?}");
    }
}

#[test]
fn numbers_take_the_language_decimal_separator_and_english_logs_keep_the_point() {
    let _language = LanguageTest::start();
    let size = backend::utils::byte_size(1_288_490_189);
    assert_eq!(size.render(), "1.2 GB");
    i18n::set_language(TEST_LANGUAGE).expect("test language");
    let shown = size.render();
    assert!(is_test_language(&shown), "{shown}");
    assert!(shown.contains("1,2"), "{shown}");
    assert_eq!(size.render_english(), "1.2 GB");
    // Whole numbers have no separator and no grouping.
    let shown = backend::utils::track_list_summary(1234, 0);
    assert!(shown.contains("1234"), "{shown}");
}

#[test]
fn the_language_doesnt_change_while_a_job_runs() {
    let _test = LanguageTest::start();
    let root = tempdir().expect("temp root");
    let backend = BackendCommands::new(root.path().join("data")).expect("create backend");
    let language = |value: &str| SetFrontendSettingRequest {
        key: LANGUAGE_SETTING.to_string(),
        value: Some(value.to_string()),
    };

    let refused = backend.set_frontend_setting_unless_job_running(language(TEST_LANGUAGE), true);
    assert!(!refused.ok, "{refused:?}");
    let error = refused.error.expect("error");
    assert_eq!(
        error.message,
        "a job is running. Try again when it has finished."
    );
    assert_eq!(i18n::current_language(), ENGLISH);

    // Other settings save while a job runs.
    let other = backend.set_frontend_setting_unless_job_running(
        SetFrontendSettingRequest {
            key: "ui_key_notation_v1".to_string(),
            value: Some("camelot".to_string()),
        },
        true,
    );
    assert!(other.ok, "{other:?}");

    let saved = backend.set_frontend_setting_unless_job_running(language(TEST_LANGUAGE), false);
    assert!(saved.ok, "{saved:?}");
    assert_eq!(i18n::current_language(), TEST_LANGUAGE);
}

#[test]
fn the_ui_texts_list_every_language_named_in_itself() {
    let _test = LanguageTest::start();
    let texts = i18n::ui_texts();
    let codes: Vec<&str> = texts.languages.iter().map(|l| l.code.as_str()).collect();
    assert_eq!(codes, i18n::languages());
    assert_eq!(texts.languages[0].name, "English");
    let test_language = texts
        .languages
        .iter()
        .find(|l| l.code == TEST_LANGUAGE)
        .expect("the test language, in a debug build");
    assert!(
        is_test_language(&test_language.name),
        "{}",
        test_language.name
    );
}
