//! The process-wide language: saving it switches every thread, cached USB reads answer in it,
//! and errors show it while the log gets English.
//!
//! In its own test binary (its own process) because the language is process-wide: switching it
//! beside the tests that assert English would break them. The tests here take `LANGUAGE_LOCK`,
//! so they don't run in parallel with each other either, and put English back when done.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use backend::commands::BackendCommands;
use backend::i18n::{self, ENGLISH, TEST_LANGUAGE, tr};
use backend::models::{FetchUsbHistoriesRequest, InitializeUsbRequest, SetFrontendSettingRequest};
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
    let from_worker = std::thread::spawn(|| tr::usb_warning_count(2usize).to_string())
        .join()
        .expect("worker thread");
    assert!(is_test_language(&from_worker), "{from_worker:?}");
    assert!(from_worker.contains('2'), "{from_worker:?}");
    assert_eq!(tr::usb_warning_count(2usize).render_english(), "2 warnings");

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

    // The stick hasn't changed, so this read comes from the parse cache.
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
    let logged: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let sink_lines = Arc::clone(&logged);
    // The only sink in this test binary.
    backend::logging::set_sink(Box::new(move |line| {
        sink_lines
            .lock()
            .unwrap()
            .push((line.message.to_string(), line.english.to_string()));
    }));
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

    let lines = logged.lock().unwrap();
    assert!(
        lines.contains(&(shown.clone(), "Unknown language: xx".to_string())),
        "{lines:?}"
    );
}
