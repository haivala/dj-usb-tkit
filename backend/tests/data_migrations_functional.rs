//! One-time data migrations (`service/migrations.rs`), end to end through
//! `BackendCommands`.

use std::fs;
use std::path::{Path, PathBuf};

use backend::commands::BackendCommands;
use backend::service::anlz::{WaveformData, build_anlz_dat_file, build_anlz_ext_file};
use tempfile::tempdir;

const BEAT_GRIDS: &str = "0.3.7-cache-beat-grids";
const MP3_SEEK: &str = "0.3.7-cache-mp3-seek-index";

/// A bundle's `.DAT` and `.EXT` bytes.
type Bundle = (Vec<u8>, Vec<u8>);

/// A bundle as the app writes it now, and the same bundle as a pre-0.3.7 app
/// wrote it: the PQTZ value two bytes early, and PQT2 without its checksum
/// or sub-millisecond body.
fn bundle_pair(bpm: f64) -> (Bundle, Bundle) {
    let band = vec![100u8; 400];
    let waveform = WaveformData {
        peaks: vec![128; 400],
        bands: vec![3; 400],
        low_energy: band.clone(),
        mid_energy: band.clone(),
        high_energy: band.clone(),
        low_energy_full: band.clone(),
        mid_energy_full: band.clone(),
        high_energy_full: band,
        peak_level: 1.0,
    };
    let dat = build_anlz_dat_file(&waveform, "", Some(bpm), 200_000, Some(437), &[]);
    let ext = build_anlz_ext_file(&waveform, "", Some(bpm), 200_000, Some(437), &[]);

    let mut old_dat = dat.clone();
    let pqtz = old_dat.windows(4).position(|w| w == b"PQTZ").unwrap();
    old_dat[pqtz + 12..pqtz + 20].copy_from_slice(&[0, 0, 0, 8, 0, 0, 0, 0]);
    let mut old_ext = ext.clone();
    let pqt2 = old_ext.windows(4).position(|w| w == b"PQT2").unwrap();
    old_ext[pqt2 + 44..pqt2 + 48].fill(0);
    let tag_len = u32::from_be_bytes(old_ext[pqt2 + 8..pqt2 + 12].try_into().unwrap()) as usize;
    for (i, at) in (pqt2 + 56..pqt2 + tag_len).step_by(2).enumerate() {
        old_ext[at..at + 2].copy_from_slice(&[(i % 4) as u8, 0]);
    }
    ((dat, ext), (old_dat, old_ext))
}

fn write_bundle(dir: &Path, name: &str, (dat, ext): &Bundle) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let dat_path = dir.join(format!("{name}.DAT"));
    fs::write(&dat_path, dat).unwrap();
    fs::write(dat_path.with_extension("EXT"), ext).unwrap();
    dat_path
}

fn read_bundle(dat_path: &Path) -> Bundle {
    (
        fs::read(dat_path).unwrap(),
        fs::read(dat_path.with_extension("EXT")).unwrap(),
    )
}

fn run(commands: &BackendCommands) -> backend::models::RunDataMigrationsData {
    let mut steps = Vec::new();
    let response = commands.run_data_migrations_with_progress(|c, t, m| {
        steps.push((c, t, m.to_string()));
    });
    assert!(response.ok, "{response:?}");
    let data = response.data.unwrap();
    if !data.ran.is_empty() {
        assert!(steps.last().is_some_and(|(c, t, _)| c == t), "{steps:?}");
    }
    data
}

#[test]
fn cached_beat_grids_are_upgraded_once() {
    let root = tempdir().unwrap();
    let data_dir = root.path().join("data");
    let waveforms = data_dir.join("analysis").join("waveforms");
    let commands = BackendCommands::new(&data_dir).unwrap();
    let (fresh_a, old_a) = bundle_pair(120.0);
    let a = write_bundle(&waveforms, "A", &old_a);

    // 1. Pending, then rewritten to exactly what the app writes today.
    assert_eq!(
        commands.pending_data_migrations().data.unwrap(),
        [BEAT_GRIDS, MP3_SEEK]
    );
    let first = run(&commands);
    assert_eq!(first.ran, [BEAT_GRIDS, MP3_SEEK]);
    assert_eq!(read_bundle(&a), fresh_a);
    assert!(commands.pending_data_migrations().data.unwrap().is_empty());

    // 2. Recorded: an old bundle put back is left alone, and nothing runs.
    fs::write(&a, &old_a.0).unwrap();
    let second = run(&commands);
    assert!(second.ran.is_empty() && second.retry_later.is_empty());
    assert_eq!(fs::read(&a).unwrap(), old_a.0);

    // 3. Record cleared: a re-run rewrites only what is still old; an
    //    already-converted bundle is byte-for-byte untouched.
    let (fresh_b, _) = bundle_pair(128.0);
    let b = write_bundle(&waveforms, "B", &fresh_b);
    let b_before = fs::metadata(&b).unwrap().modified().unwrap();
    rusqlite::Connection::open(data_dir.join("backend.db"))
        .unwrap()
        .execute("DELETE FROM data_migrations WHERE id = ?1", [BEAT_GRIDS])
        .unwrap();
    let third = run(&commands);
    assert_eq!(third.ran, [BEAT_GRIDS]);
    assert_eq!(read_bundle(&a), fresh_a);
    assert_eq!(read_bundle(&b), fresh_b);
    assert_eq!(fs::metadata(&b).unwrap().modified().unwrap(), b_before);

    // A new install of the app (same data dir) doesn't run it again.
    let reopened = BackendCommands::new(&data_dir).unwrap();
    assert!(reopened.pending_data_migrations().data.unwrap().is_empty());
}

#[test]
fn startup_migrations_are_recorded_when_the_service_opens() {
    let root = tempdir().unwrap();
    let data_dir = root.path().join("data");
    let _commands = BackendCommands::new(&data_dir).unwrap();
    let conn = rusqlite::Connection::open(data_dir.join("backend.db")).unwrap();
    let mut ids: Vec<String> = conn
        .prepare("SELECT id FROM data_migrations ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            "0.2-merge-usb-placeholder-tracks",
            "0.2-track-match-fingerprints",
            "0.2-usb-devices-from-legacy-settings",
        ]
    );
}

/// The sample total at the end of a `.DAT`'s `PVBR` (zero when empty).
fn pvbr_total(dat: &[u8]) -> u32 {
    let at = dat.windows(4).position(|w| w == b"PVBR").expect("PVBR");
    u32::from_be_bytes(dat[at + 16 + 1600..at + 16 + 1604].try_into().unwrap())
}

#[test]
fn cached_mp3_bundles_get_their_seek_index_once() {
    let root = tempdir().unwrap();
    let data_dir = root.path().join("data");
    let waveforms = data_dir.join("analysis").join("waveforms");
    let commands = BackendCommands::new(&data_dir).unwrap();

    let media = root.path().join("media");
    fs::create_dir_all(&media).unwrap();
    let mp3 = media.join("track.mp3");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio/embedded/track_embedded.mp3"),
        &mp3,
    )
    .unwrap();
    let (fresh, _) = bundle_pair(120.0);
    assert_eq!(
        pvbr_total(&fresh.0),
        0,
        "generated bundles start with an empty PVBR"
    );
    let with_audio = write_bundle(&waveforms, "M1", &fresh);
    let without_audio = write_bundle(&waveforms, "M2", &fresh);

    let conn = rusqlite::Connection::open(data_dir.join("backend.db")).unwrap();
    for (id, audio, dat) in [
        ("t1", mp3.clone(), &with_audio),
        ("t2", media.join("unplugged.mp3"), &without_audio),
    ] {
        conn.execute(
            "INSERT INTO tracks (id, title, artist, file_path, waveform_peaks_path, created_at, updated_at)
             VALUES (?1, ?1, 'A', ?2, ?3, '2026-01-01', '2026-01-01')",
            rusqlite::params![id, audio.to_string_lossy(), dat.to_string_lossy()],
        )
        .unwrap();
    }

    let first = run(&commands);
    assert!(first.ran.contains(&MP3_SEEK.to_string()), "{first:?}");
    let filled = fs::read(&with_audio).unwrap();
    assert!(pvbr_total(&filled) > 0);
    assert_eq!(filled.len(), fresh.0.len(), "filled in place");
    // A missing source file: the bundle is deleted and the track marked for
    // analysis, so it is rebuilt with seek data once the file is back.
    for ext in ["DAT", "EXT"] {
        assert!(!without_audio.with_extension(ext).exists(), "{ext} deleted");
    }
    let path: Option<String> = conn
        .query_row(
            "SELECT waveform_peaks_path FROM tracks WHERE id = 't2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(path, None);
    let kept: Option<String> = conn
        .query_row(
            "SELECT waveform_peaks_path FROM tracks WHERE id = 't1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept.as_deref(), Some(with_audio.to_string_lossy().as_ref()));
    assert!(commands.pending_data_migrations().data.unwrap().is_empty());
}
