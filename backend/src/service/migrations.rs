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

use std::sync::atomic::Ordering;

use rusqlite::params;

use crate::error::{BackendError, BackendResult};
use crate::models::RunDataMigrationsData;

use super::BackendService;
use super::anlz::{GridUpgrade, upgrade_bundle_beat_grid};

pub(crate) type Progress<'a> = &'a mut dyn FnMut(usize, usize, &str);

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
    pub label: &'static str,
    pub run: fn(&BackendService, Progress<'_>) -> BackendResult<Outcome>,
}

pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        id: "0.2-usb-devices-from-legacy-settings",
        phase: Phase::Startup,
        label: "Importing USB devices from old settings",
        run: |svc, _| {
            svc.backfill_usb_devices_from_legacy_settings()
                .map(|()| Outcome::Done)
        },
    },
    Migration {
        id: "0.2-track-match-fingerprints",
        phase: Phase::Startup,
        label: "Filling track fingerprints",
        run: |svc, _| svc.backfill_track_fingerprints().map(|()| Outcome::Done),
    },
    Migration {
        id: "0.2-merge-usb-placeholder-tracks",
        phase: Phase::Startup,
        label: "Merging duplicate USB tracks",
        run: |svc, _| {
            svc.merge_orphaned_usb_placeholder_tracks()
                .map(|_| Outcome::Done)
        },
    },
    Migration {
        id: "0.3.7-cache-beat-grids",
        phase: Phase::Background,
        label: "Upgrading cached beat grids",
        run: upgrade_cached_beat_grids,
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
        F: FnMut(usize, usize, &str),
    {
        run_registry(self, MIGRATIONS, Phase::Background, &mut on_progress)
    }

    /// The `Startup` migrations, from `new()`. A failure is logged and the
    /// migration retried on the next launch; it never stops the app starting.
    pub(crate) fn run_startup_data_migrations(&self) {
        if let Err(err) = run_registry(self, MIGRATIONS, Phase::Startup, &mut |_, _, _| {}) {
            crate::backend_log!(Warn, "migrations", "startup data migrations failed: {err}");
        }
    }

    /// Commands that write the local analysis cache call this first, so a
    /// running migration can't overwrite their fresh files with ones rebuilt
    /// from stale data.
    pub(crate) fn ensure_no_data_migration(&self) -> BackendResult<()> {
        if self.data_migration_running.load(Ordering::SeqCst) {
            return Err(BackendError::Validation(
                "Finishing a data upgrade, try again in a moment".to_string(),
            ));
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
    on_progress: Progress<'_>,
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
                &format!("{}: {current}/{total}", migration.label),
            );
        };
        match (migration.run)(svc, &mut progress) {
            Ok(Outcome::Done) => {
                svc.db.connect()?.execute(
                    "INSERT OR IGNORE INTO data_migrations (id, applied_at) VALUES (?1, ?2)",
                    params![migration.id, super::now()],
                )?;
                crate::backend_log!(Info, "migrations", "data migration done: {}", migration.id);
                summary.ran.push(migration.id.to_string());
            }
            Ok(Outcome::RetryLater) => {
                crate::backend_log!(
                    Warn,
                    "migrations",
                    "data migration {} incomplete; it runs again next launch",
                    migration.id
                );
                summary.retry_later.push(migration.id.to_string());
            }
            Err(err) => {
                crate::backend_log!(
                    Warn,
                    "migrations",
                    "data migration {} failed ({err}); it runs again next launch",
                    migration.id
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
    progress: Progress<'_>,
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
            Ok(GridUpgrade::NotRebuildable) => crate::backend_log!(
                Warn,
                "migrations",
                "cached beat grid left as it is (not a constant-tempo grid): {}",
                dat.display()
            ),
            Err(err) => {
                retry = true;
                crate::backend_log!(
                    Warn,
                    "migrations",
                    "cached beat grid not upgraded ({err}): {}",
                    dat.display()
                );
            }
        }
        progress(i + 1, total, "");
    }
    crate::backend_log!(
        Info,
        "migrations",
        "rewrote {rewritten} cached beat grid(s) in the 0.3.7 format"
    );
    Ok(if retry {
        Outcome::RetryLater
    } else {
        Outcome::Done
    })
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
            label: "A",
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
            label: "B",
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
            label: "S",
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
            [(1, 2, "A: 1/2".to_string()), (2, 2, "A: 2/2".to_string())]
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
            matches!(&analysis, Err(BackendError::Validation(m)) if m.contains("data upgrade")),
            "{analysis:?}"
        );
        let edits: crate::models::SaveTrackAnalysisEditsRequest =
            serde_json::from_value(serde_json::json!({ "trackId": "t1" })).unwrap();
        let saved = svc.save_track_analysis_edits(edits);
        assert!(
            matches!(&saved, Err(BackendError::Validation(m)) if m.contains("data upgrade")),
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
}
