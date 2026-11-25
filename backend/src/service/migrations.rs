//! One-time data migrations: work that upgrades data an older app version
//! wrote, run once per install and recorded in the `data_migrations` table.
//! See docs/APP_DATA_MODEL.md ("Data migrations").
//!
//! Adding one is a single entry in [`MIGRATIONS`]. Rules:
//! - the id is a stable name, never renamed or reused;
//! - a migration is safe to re-run: it touches only data still in the old
//!   state, with atomic file writes, so a crash before it is recorded just
//!   repeats harmless work;
//! - `Startup` migrations are quick and run before the UI reads anything;
//!   `Background` ones run as a progress-bar job and report progress.

use crate::utils::counted;
use std::sync::atomic::Ordering;

use rusqlite::params;

use crate::error::{BackendError, BackendResult};
use crate::i18n::{Text, tr};
use crate::models::RunDataMigrationsData;

use super::BackendService;
use super::anlz::{GridUpgrade, PvbrFill, fill_mp3_pvbr, upgrade_bundle_beat_grid};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Run synchronously at the end of `BackendService::new`.
    Startup,
    /// Run by the `run_data_migrations` command, as a progress-bar job.
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Done,
    /// Something couldn't be done now (e.g. a file couldn't be read); the
    /// migration is not recorded and runs again on the next launch.
    RetryLater,
}

pub(crate) struct Migration {
    pub id: &'static str,
    pub phase: Phase,
    /// Progress-bar text.
    pub label: fn() -> Text,
    pub run: fn(&BackendService, &mut dyn crate::progress::ProgressFn) -> BackendResult<Outcome>,
}

pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        id: "0.2-usb-devices-from-legacy-settings",
        phase: Phase::Startup,
        label: tr::migration_usb_devices,
        run: |svc, _| {
            svc.backfill_usb_devices_from_legacy_settings()
                .map(|()| Outcome::Done)
        },
    },
    Migration {
        id: "0.2-track-match-fingerprints",
        phase: Phase::Startup,
        label: tr::migration_fingerprints,
        run: |svc, _| svc.backfill_track_fingerprints().map(|()| Outcome::Done),
    },
    Migration {
        id: "0.2-merge-usb-placeholder-tracks",
        phase: Phase::Startup,
        label: tr::migration_merge_usb_tracks,
        run: |svc, _| {
            svc.merge_orphaned_usb_placeholder_tracks()
                .map(|_| Outcome::Done)
        },
    },
    Migration {
        id: "0.3.7-cache-beat-grids",
        phase: Phase::Background,
        label: tr::migration_beat_grids,
        run: upgrade_cached_beat_grids,
    },
    Migration {
        id: "0.3.7-cache-mp3-seek-index",
        phase: Phase::Background,
        label: tr::migration_mp3_seek,
        run: fill_cached_mp3_seek_indexes,
    },
    Migration {
        id: "0.6.7-long-track-waveforms",
        phase: Phase::Startup,
        label: tr::migration_long_track_waveforms,
        run: drop_long_track_waveforms,
    },
    Migration {
        id: "0.6.7-stratum-v1-analysis",
        phase: Phase::Startup,
        label: tr::migration_stratum_v1_analysis,
        run: drop_stratum_v1_analysis,
    },
];

impl BackendService {
    /// Ids of the `Background` migrations still to run (one cheap query).
    pub fn pending_data_migrations(&self) -> BackendResult<Vec<String>> {
        Ok(pending(self, MIGRATIONS, Phase::Background)?
            .into_iter()
            .map(|m| m.id.to_string())
            .collect())
    }

    /// Run the pending `Background` migrations, reporting progress. Analysis
    /// and analysis edits are refused meanwhile (see
    /// [`BackendService::ensure_no_data_migration`]).
    pub fn run_data_migrations<F>(&self, mut on_progress: F) -> BackendResult<RunDataMigrationsData>
    where
        F: crate::progress::ProgressFn,
    {
        run_registry(self, MIGRATIONS, Phase::Background, &mut on_progress)
    }

    /// The `Startup` migrations, from `new()`. A failure is logged and the
    /// migration retried on the next launch; it never stops the app starting.
    pub(crate) fn run_startup_data_migrations(&self) {
        if let Err(err) = run_registry(self, MIGRATIONS, Phase::Startup, &mut |_, _, _| {}) {
            crate::logging::log_text(
                crate::logging::Level::Warn,
                "migrations",
                "migrations.startup-failed",
                &tr::migration_warning_startup_failed(&err),
            );
        }
    }

    /// Commands that write the local analysis cache call this first, so a
    /// running migration can't overwrite their fresh files with ones rebuilt
    /// from stale data.
    pub(crate) fn ensure_no_data_migration(&self) -> BackendResult<()> {
        if self.data_migration_running.load(Ordering::SeqCst) {
            return Err(BackendError::Validation(tr::error_data_upgrade_running()));
        }
        Ok(())
    }
}

fn pending<'a>(
    svc: &BackendService,
    registry: &'a [Migration],
    phase: Phase,
) -> BackendResult<Vec<&'a Migration>> {
    let conn = svc.db.connect()?;
    let mut stmt = conn.prepare("SELECT 1 FROM data_migrations WHERE id = ?1")?;
    let mut out = Vec::new();
    for migration in registry.iter().filter(|m| m.phase == phase) {
        if !stmt.exists(params![migration.id])? {
            out.push(migration);
        }
    }
    Ok(out)
}

/// Clears the guard flag when a background run ends, however it ends.
struct RunningGuard<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn run_registry(
    svc: &BackendService,
    registry: &[Migration],
    phase: Phase,
    on_progress: &mut dyn crate::progress::ProgressFn,
) -> BackendResult<RunDataMigrationsData> {
    let pending = pending(svc, registry, phase)?;
    let mut summary = RunDataMigrationsData::default();
    if pending.is_empty() {
        return Ok(summary);
    }
    let _guard = (phase == Phase::Background).then(|| {
        svc.data_migration_running.store(true, Ordering::SeqCst);
        RunningGuard(&svc.data_migration_running)
    });
    for migration in pending {
        let mut progress = |current: usize, total: usize, _: &str| {
            on_progress(
                current,
                total,
                &crate::progress::count((migration.label)().render(), current, total),
            );
        };
        match (migration.run)(svc, &mut progress) {
            Ok(Outcome::Done) => {
                svc.db.connect()?.execute(
                    "INSERT OR IGNORE INTO data_migrations (id, applied_at) VALUES (?1, ?2)",
                    params![migration.id, super::now()],
                )?;
                crate::logging::debug(
                    "migrations",
                    format!("data migration done: {}", migration.id),
                );
                summary.ran.push(migration.id.to_string());
            }
            Ok(Outcome::RetryLater) => {
                crate::logging::debug(
                    "migrations",
                    format!(
                        "data migration {} incomplete; it runs again next launch",
                        migration.id
                    ),
                );
                summary.retry_later.push(migration.id.to_string());
            }
            Err(err) => {
                crate::logging::debug(
                    "migrations",
                    format!("data migration {} failed", migration.id),
                );
                crate::logging::log_text(
                    crate::logging::Level::Warn,
                    "migrations",
                    "migrations.failed",
                    &tr::migration_warning_failed((migration.label)(), &err),
                );
                summary.retry_later.push(migration.id.to_string());
            }
        }
    }
    Ok(summary)
}

/// `0.3.7-cache-beat-grids`: rewrite the app's cached bundles whose beat
/// grid is in the pre-0.3.7 format (docs/WAVEFORMS.md "Beat-grid layout").
/// Only the app's own `analysis/waveforms` dir is walked; rekordbox-imported
/// bundles live elsewhere.
fn upgrade_cached_beat_grids(
    svc: &BackendService,
    progress: &mut dyn crate::progress::ProgressFn,
) -> BackendResult<Outcome> {
    let dir = svc.db.data_dir().join("analysis").join("waveforms");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Outcome::Done);
    };
    let mut dats: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("DAT"))
        })
        .collect();
    dats.sort();

    let total = dats.len();
    let mut rewritten = 0usize;
    let mut retry = false;
    for (i, dat) in dats.iter().enumerate() {
        match upgrade_bundle_beat_grid(dat) {
            Ok(GridUpgrade::Rewritten) => rewritten += 1,
            Ok(GridUpgrade::AlreadyCurrent) => {}
            Ok(GridUpgrade::NotRebuildable) => crate::logging::debug(
                "migrations",
                format!(
                    "cached beat grid left as it is (not a constant-tempo grid): {}",
                    dat.display()
                ),
            ),
            Err(err) => {
                retry = true;
                crate::logging::debug(
                    "migrations",
                    format!(
                        "cached beat grid not upgraded ({err}): {}",
                        dat.display(),
                        err = err.english()
                    ),
                );
            }
        }
        progress(i + 1, total, "");
    }
    crate::logging::debug(
        "migrations",
        format!(
            "rewrote {} in the 0.3.7 format",
            counted(rewritten, "cached beat grid", "cached beat grids")
        ),
    );
    Ok(if retry {
        Outcome::RetryLater
    } else {
        Outcome::Done
    })
}

/// `0.3.7-cache-mp3-seek-index`: give cached MP3 bundles the `PVBR` seek
/// index rekordbox writes (docs/WAVEFORMS.md "Seek-index chunks"), which
/// analysis writes from 0.3.7 on. Needs each source file: when one is
/// missing, the cached bundle is deleted and the track marked for analysis
/// (as a stale cache is in `scan_library`), so it is rebuilt, seek data
/// included, once the file is back.
fn fill_cached_mp3_seek_indexes(
    svc: &BackendService,
    progress: &mut dyn crate::progress::ProgressFn,
) -> BackendResult<Outcome> {
    let waveform_dir = svc.db.data_dir().join("analysis").join("waveforms");
    let conn = svc.db.connect()?;
    let bundles: Vec<(String, std::path::PathBuf, std::path::PathBuf)> = {
        let mut stmt = conn.prepare(
            "SELECT id, file_path, waveform_peaks_path FROM tracks
             WHERE waveform_peaks_path IS NOT NULL AND waveform_peaks_path != ''",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.filter_map(Result::ok)
            .filter_map(|(id, audio, dat)| {
                let audio = std::path::PathBuf::from(audio.trim());
                let is_mp3 = crate::audio_format::AudioFormat::of_path(&audio)
                    == Some(crate::audio_format::AudioFormat::Mp3);
                let dat =
                    super::local_cache_bundle_dat(&waveform_dir, std::path::Path::new(dat.trim()))?;
                is_mp3.then_some((id, dat, audio))
            })
            .collect()
    };

    let total = bundles.len();
    let (mut filled, mut missing) = (0usize, 0usize);
    let mut retry = false;
    for (i, (id, dat, audio)) in bundles.iter().enumerate() {
        if !audio.is_file() {
            conn.execute(
                "UPDATE tracks SET waveform_peaks_path = NULL WHERE id = ?1",
                params![id],
            )?;
            let still_used: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM tracks WHERE waveform_peaks_path = ?1)",
                params![dat.to_string_lossy()],
                |row| row.get(0),
            )?;
            if !still_used {
                super::remove_bundle_files(dat);
            }
            missing += 1;
        } else if dat.is_file() {
            match fill_mp3_pvbr(dat, audio) {
                Ok(PvbrFill::Filled) => filled += 1,
                Ok(_) => {}
                Err(_) => retry = true,
            }
        }
        progress(i + 1, total, "");
    }
    crate::logging::debug(
        "migrations",
        format!(
            "added the MP3 seek index to {}; marked {} for analysis (file not found)",
            counted(filled, "cached bundle", "cached bundles"),
            counted(missing, "track", "tracks")
        ),
    );
    Ok(if retry {
        Outcome::RetryLater
    } else {
        Outcome::Done
    })
}

/// The mono samples analysis decoded before 0.6.7, waveform included. Fixed
/// here: it says which waveforms that version cut, whatever the analysis cap
/// (`BPM_KEY_MAX_SAMPLES`) becomes.
const DECODE_CAP_BEFORE_0_6_7: i64 = 24_000_000;

/// Whether analysis before 0.6.7 decoded only part of audio this long:
/// `duration_ms` at `sample_rate_hz` (48 kHz when unknown, the lower length)
/// reaches the decode cap, about 9 minutes at 44.1 kHz.
fn reached_decode_cap(duration_ms: i64, sample_rate_hz: Option<i64>) -> bool {
    let rate = sample_rate_hz.filter(|rate| *rate > 0).unwrap_or(48_000);
    duration_ms > 0 && duration_ms.saturating_mul(rate) / 1000 >= DECODE_CAP_BEFORE_0_6_7
}

/// `0.6.7-long-track-waveforms`: analysis before 0.6.7 drew a track longer
/// than its decode cap as the first part of its audio stretched over its
/// whole length (docs/WAVEFORMS.md "Decode Bounds"). Such a track's cached
/// bundle is deleted and the track marked for analysis, which draws it again
/// whole; its BPM, key, cues and edits stay. A transcode that long loses its
/// waveform bundle, which export draws again when it needs it. Bundles from
/// rekordbox or a USB are not the app's and stay.
fn drop_long_track_waveforms(
    svc: &BackendService,
    _: &mut dyn crate::progress::ProgressFn,
) -> BackendResult<Outcome> {
    let data_dir = svc.db.data_dir();
    let waveform_dir = data_dir.join("analysis").join("waveforms");
    let conn = svc.db.connect()?;
    let tracks: Vec<(String, std::path::PathBuf)> = {
        let mut stmt = conn.prepare(
            "SELECT id, waveform_peaks_path, duration_ms, sample_rate_hz FROM tracks
             WHERE waveform_peaks_path IS NOT NULL AND waveform_peaks_path != ''
               AND duration_ms > 0",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        rows.filter_map(Result::ok)
            .filter(|(_, _, duration_ms, rate)| reached_decode_cap(*duration_ms, *rate))
            .filter_map(|(id, dat, _, _)| {
                let dat =
                    super::local_cache_bundle_dat(&waveform_dir, std::path::Path::new(dat.trim()))?;
                Some((id, dat))
            })
            .collect()
    };
    drop_cached_bundles(&conn, &tracks)?;

    let transcodes: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT file_path, duration_ms, sample_rate_hz FROM track_transcodes
             WHERE duration_ms > 0",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?;
        rows.filter_map(Result::ok)
            .filter(|(_, duration_ms, rate)| reached_decode_cap(*duration_ms, *rate))
            .map(|(path, _, _)| path)
            .collect()
    };
    for path in &transcodes {
        let dir = super::transcode::transcode_bundle_dir(&data_dir, std::path::Path::new(path));
        let bundle = super::transcode::transcode_waveform_bundle(&dir);
        for file in [&bundle.dat_path, &bundle.ext_path, &bundle.twoex_path] {
            let _ = std::fs::remove_file(file);
        }
    }

    crate::logging::debug(
        "migrations",
        format!(
            "marked {} longer than the decode cap for analysis; dropped the waveforms of {}",
            counted(tracks.len(), "track", "tracks"),
            counted(transcodes.len(), "transcode", "transcodes")
        ),
    );
    Ok(Outcome::Done)
}

/// Marks each track for analysis by clearing its `waveform_peaks_path`, and
/// deletes its cached bundle (`.DAT` path) once no track uses it.
fn drop_cached_bundles(
    conn: &rusqlite::Connection,
    tracks: &[(String, std::path::PathBuf)],
) -> BackendResult<()> {
    for (id, dat) in tracks {
        conn.execute(
            "UPDATE tracks SET waveform_peaks_path = NULL WHERE id = ?1",
            params![id],
        )?;
        let still_used: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tracks WHERE waveform_peaks_path = ?1)",
            params![dat.to_string_lossy()],
            |row| row.get(0),
        )?;
        if !still_used {
            super::remove_bundle_files(dat);
        }
    }
    Ok(())
}

/// `0.6.7-stratum-v1-analysis`: a track whose BPM is still stratum's raw
/// estimate (`bpm_analyzer = "stratum"`, analysis before `refine_bpm`, up to
/// 0.3.3) loses its cached bundle and is marked for analysis, like a long
/// track in `drop_long_track_waveforms`. The new analysis replaces that BPM,
/// its beat grid and first beat (a `"stratum"` BPM is not one analysis keeps),
/// so the library's bundles are all from the current analysis. Cues and edits
/// stay. Bundles from rekordbox or a USB are not the app's and stay.
fn drop_stratum_v1_analysis(
    svc: &BackendService,
    _: &mut dyn crate::progress::ProgressFn,
) -> BackendResult<Outcome> {
    let waveform_dir = svc.db.data_dir().join("analysis").join("waveforms");
    let conn = svc.db.connect()?;
    let tracks: Vec<(String, std::path::PathBuf)> = {
        let mut stmt = conn.prepare(
            "SELECT id, waveform_peaks_path FROM tracks
             WHERE bpm_analyzer = 'stratum'
               AND waveform_peaks_path IS NOT NULL AND waveform_peaks_path != ''",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.filter_map(Result::ok)
            .filter_map(|(id, dat)| {
                let dat =
                    super::local_cache_bundle_dat(&waveform_dir, std::path::Path::new(dat.trim()))?;
                Some((id, dat))
            })
            .collect()
    };
    drop_cached_bundles(&conn, &tracks)?;
    crate::logging::debug(
        "migrations",
        format!(
            "marked {} with stratum's raw BPM for analysis",
            counted(tracks.len(), "track", "tracks")
        ),
    );
    Ok(Outcome::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    static CALLS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    static FAILS_LEFT: AtomicUsize = AtomicUsize::new(0);
    static SAW_GUARD: AtomicUsize = AtomicUsize::new(0);

    fn service() -> (tempfile::TempDir, BackendService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = BackendService::new(dir.path()).unwrap();
        (dir, svc)
    }

    fn record(id: &'static str) {
        CALLS.lock().unwrap().push(id);
    }

    const REGISTRY: &[Migration] = &[
        Migration {
            id: "a",
            phase: Phase::Background,
            label: tr::migration_beat_grids,
            run: |svc, progress| {
                record("a");
                if svc.data_migration_running.load(Ordering::SeqCst) {
                    SAW_GUARD.fetch_add(1, Ordering::SeqCst);
                }
                progress(1, 2, "");
                progress(2, 2, "");
                Ok(Outcome::Done)
            },
        },
        Migration {
            id: "b",
            phase: Phase::Background,
            label: tr::migration_mp3_seek,
            run: |_, _| {
                record("b");
                if FAILS_LEFT.load(Ordering::SeqCst) > 0 {
                    FAILS_LEFT.fetch_sub(1, Ordering::SeqCst);
                    return Err(BackendError::Internal("boom".to_string()));
                }
                Ok(Outcome::RetryLater)
            },
        },
        Migration {
            id: "s",
            phase: Phase::Startup,
            label: tr::migration_usb_devices,
            run: |_, _| {
                record("s");
                Ok(Outcome::Done)
            },
        },
    ];

    // One test, so the shared statics aren't raced by parallel tests.
    #[test]
    fn registry_runs_each_migration_once_and_retries_unfinished_ones() {
        let (_dir, svc) = service();
        CALLS.lock().unwrap().clear();
        FAILS_LEFT.store(1, Ordering::SeqCst);

        let mut messages = Vec::new();
        let first = run_registry(&svc, REGISTRY, Phase::Background, &mut |c, t, m| {
            messages.push((c, t, m.to_string()))
        })
        .unwrap();
        // "b" failed with an error: not recorded, not an error for the caller.
        assert_eq!(first.ran, ["a"]);
        assert_eq!(first.retry_later, ["b"]);
        assert_eq!(
            messages,
            [
                (
                    1,
                    2,
                    crate::progress::with_detail("Upgrading cached beat grids", "1/2")
                ),
                (
                    2,
                    2,
                    crate::progress::with_detail("Upgrading cached beat grids", "2/2")
                )
            ]
        );
        assert_eq!(
            SAW_GUARD.load(Ordering::SeqCst),
            1,
            "guard set while running"
        );
        assert!(
            !svc.data_migration_running.load(Ordering::SeqCst),
            "guard cleared"
        );

        // "a" is recorded and skipped; "b" (RetryLater now) runs again.
        let second = run_registry(&svc, REGISTRY, Phase::Background, &mut |_, _, _| {}).unwrap();
        assert!(second.ran.is_empty());
        assert_eq!(second.retry_later, ["b"]);
        assert_eq!(*CALLS.lock().unwrap(), ["a", "b", "b"]);

        // Phases are separate; startup ones are recorded too.
        let startup = run_registry(&svc, REGISTRY, Phase::Startup, &mut |_, _, _| {}).unwrap();
        assert_eq!(startup.ran, ["s"]);
        assert!(pending(&svc, REGISTRY, Phase::Startup).unwrap().is_empty());
        assert_eq!(
            pending(&svc, REGISTRY, Phase::Background)
                .unwrap()
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            ["b"]
        );
    }

    #[test]
    fn guard_refuses_cache_writes_while_a_migration_runs() {
        let (_dir, svc) = service();
        assert!(svc.ensure_no_data_migration().is_ok());
        svc.data_migration_running.store(true, Ordering::SeqCst);
        assert!(matches!(
            svc.ensure_no_data_migration(),
            Err(BackendError::Validation(_))
        ));
        // The commands that write the analysis cache.
        let analysis = svc.analyze_new_tracks(crate::models::AnalyzeNewTracksRequest::default());
        assert!(
            matches!(&analysis, Err(BackendError::Validation(m)) if m.render_english().contains("data upgrade")),
            "{analysis:?}"
        );
        let edits: crate::models::SaveTrackAnalysisEditsRequest =
            serde_json::from_value(serde_json::json!({ "trackId": "t1" })).unwrap();
        let saved = svc.save_track_analysis_edits(edits);
        assert!(
            matches!(&saved, Err(BackendError::Validation(m)) if m.render_english().contains("data upgrade")),
            "{saved:?}"
        );
    }

    #[test]
    fn real_registry_ids_are_unique() {
        let mut ids: Vec<_> = MIGRATIONS.iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), MIGRATIONS.len());
    }

    // Before 0.6.7, analysis drew a track longer than its decode cap as its
    // first part stretched: those cached bundles go and the tracks need
    // analysis again; shorter tracks, other apps' bundles and tracks with no
    // known length stay. Long transcodes lose their waveform bundle.
    #[test]
    fn long_track_waveforms_are_dropped_and_their_tracks_need_analysis() {
        let (dir, svc) = service();
        let cache = dir.path().join("analysis").join("waveforms");
        std::fs::create_dir_all(&cache).unwrap();
        let foreign = dir.path().join("rekordbox");
        std::fs::create_dir_all(&foreign).unwrap();
        let bundle = |dir: &std::path::Path, stem: &str| {
            for ext in ["DAT", "EXT", "2EX"] {
                std::fs::write(dir.join(format!("{stem}.{ext}")), b"x").unwrap();
            }
            dir.join(format!("{stem}.DAT"))
        };
        // (id, bundle, duration_ms, sample rate, dropped)
        let cases = [
            (
                "long44",
                bundle(&cache, "LONG44"),
                Some(545_000),
                Some(44_100),
                true,
            ),
            (
                "short44",
                bundle(&cache, "SHORT44"),
                Some(540_000),
                Some(44_100),
                false,
            ),
            (
                "long48",
                bundle(&cache, "LONG48"),
                Some(510_000),
                Some(48_000),
                true,
            ),
            (
                "unknown_rate",
                bundle(&cache, "NORATE"),
                Some(505_000),
                None,
                true,
            ),
            (
                "foreign",
                bundle(&foreign, "FOREIGN"),
                Some(900_000),
                Some(44_100),
                false,
            ),
            (
                "no_length",
                bundle(&cache, "NOLEN"),
                None,
                Some(44_100),
                false,
            ),
        ];
        let conn = svc.db.connect().unwrap();
        for (id, dat, duration_ms, rate, _) in &cases {
            conn.execute(
                "INSERT INTO tracks (id, title, artist, file_path, bpm, duration_ms,
                   sample_rate_hz, waveform_peaks_path, created_at, updated_at)
                 VALUES (?1, ?1, 'A', ?2, 120.0, ?3, ?4, ?5, '1', '1')",
                params![
                    id,
                    format!("/music/{id}.flac"),
                    duration_ms,
                    rate,
                    dat.to_string_lossy()
                ],
            )
            .unwrap();
        }
        let transcode_bundle = |path: &str, duration_ms: i64| {
            conn.execute(
                "INSERT INTO track_transcodes (track_id, format, file_path, file_size_bytes,
                   sample_rate_hz, duration_ms, created_at)
                 VALUES ('long44', ?1, ?1, 1, 44100, ?2, '1')",
                params![path, duration_ms],
            )
            .unwrap();
            let dir = super::super::transcode::transcode_bundle_dir(
                dir.path(),
                std::path::Path::new(path),
            );
            std::fs::create_dir_all(&dir).unwrap();
            super::super::transcode::transcode_waveform_bundle(&dir)
        };
        let long_transcode = transcode_bundle("/t/long.aiff", 600_000);
        let short_transcode = transcode_bundle("/t/short.aiff", 300_000);
        for b in [&long_transcode, &short_transcode] {
            std::fs::write(&b.dat_path, b"x").unwrap();
            std::fs::write(&b.ext_path, b"x").unwrap();
        }

        assert_eq!(
            drop_long_track_waveforms(&svc, &mut |_, _, _| {}).unwrap(),
            Outcome::Done
        );

        for (id, dat, _, _, dropped) in &cases {
            let path: Option<String> = conn
                .query_row(
                    "SELECT waveform_peaks_path FROM tracks WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(path.is_none(), *dropped, "{id}");
            for ext in ["DAT", "EXT", "2EX"] {
                assert_eq!(!dat.with_extension(ext).exists(), *dropped, "{id} {ext}");
            }
        }
        assert!(!long_transcode.dat_path.exists() && !long_transcode.ext_path.exists());
        assert!(short_transcode.dat_path.exists() && short_transcode.ext_path.exists());
    }

    // A track with stratum's raw BPM loses its cached bundle and needs
    // analysis again; later analyzers, imports, user edits and bundles that
    // aren't the app's stay.
    #[test]
    fn stratum_v1_analysis_is_dropped_and_its_tracks_need_analysis() {
        let (dir, svc) = service();
        let cache = dir.path().join("analysis").join("waveforms");
        std::fs::create_dir_all(&cache).unwrap();
        let foreign = dir.path().join("usb");
        std::fs::create_dir_all(&foreign).unwrap();
        let bundle = |dir: &std::path::Path, stem: &str| {
            for ext in ["DAT", "EXT", "2EX"] {
                std::fs::write(dir.join(format!("{stem}.{ext}")), b"x").unwrap();
            }
            dir.join(format!("{stem}.DAT"))
        };
        // (id, bundle, bpm_analyzer, dropped)
        let cases = [
            ("v1", bundle(&cache, "V1"), Some("stratum"), true),
            ("v2", bundle(&cache, "V2"), Some("stratum-v2"), false),
            ("essentia", bundle(&cache, "ESS"), Some("essentia"), false),
            ("user", bundle(&cache, "USER"), Some("user"), false),
            ("rekordbox", bundle(&cache, "RB"), Some("rekordbox"), false),
            ("unmarked", bundle(&cache, "NONE"), None, false),
            (
                "foreign",
                bundle(&foreign, "FOREIGN"),
                Some("stratum"),
                false,
            ),
        ];
        let conn = svc.db.connect().unwrap();
        for (id, dat, analyzer, _) in &cases {
            conn.execute(
                "INSERT INTO tracks (id, title, artist, file_path, bpm, bpm_analyzer,
                   duration_ms, waveform_peaks_path, created_at, updated_at)
                 VALUES (?1, ?1, 'A', ?2, 120.0, ?3, 300000, ?4, '1', '1')",
                params![
                    id,
                    format!("/music/{id}.flac"),
                    analyzer,
                    dat.to_string_lossy()
                ],
            )
            .unwrap();
        }

        assert_eq!(
            drop_stratum_v1_analysis(&svc, &mut |_, _, _| {}).unwrap(),
            Outcome::Done
        );

        for (id, dat, _, dropped) in &cases {
            let (path, bpm): (Option<String>, f64) = conn
                .query_row(
                    "SELECT waveform_peaks_path, bpm FROM tracks WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(path.is_none(), *dropped, "{id}");
            assert_eq!(bpm, 120.0, "{id}: the BPM stays until the analysis");
            for ext in ["DAT", "EXT", "2EX"] {
                assert_eq!(!dat.with_extension(ext).exists(), *dropped, "{id} {ext}");
            }
        }
    }
}
