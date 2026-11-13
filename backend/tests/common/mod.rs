//! Helpers shared by the functional test crates.
//!
//! Each test binary compiles this module separately and uses only part of it.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use backend::service::usb_vendor_compat::{
    DEFAULT_USB_EDB_KEY, USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR,
};

/// Path of an audio file under `tests/fixtures/audio`.
pub fn fixture_audio_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("audio")
        .join(relative)
}

/// Copies an audio fixture into `media_dir` as `target_name`, as a distinct
/// file (see [`copy_unique_audio`]).
pub fn copy_audio_fixture(media_dir: &Path, fixture_relative: &str, target_name: &str) {
    copy_unique_audio(
        &fixture_audio_path(fixture_relative),
        &media_dir.join(target_name),
    );
}

/// Writes placeholder DAT/EXT/2EX files into the app's analysis cache and
/// returns the DAT path.
pub fn seed_test_analysis_bundle(data_dir: &Path, stem: &str) -> PathBuf {
    let dir = data_dir.join("analysis").join("waveforms");
    fs::create_dir_all(&dir).expect("create test analysis dir");
    let safe_stem = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let dat = dir.join(format!("{safe_stem}.DAT"));
    fs::write(&dat, b"test-dat").expect("write test DAT");
    fs::write(dir.join(format!("{safe_stem}.EXT")), b"test-ext").expect("write test EXT");
    fs::write(dir.join(format!("{safe_stem}.2EX")), b"test-2ex").expect("write test 2EX");
    dat
}

/// Marks tracks as analyzed (BPM, key, duration, analysis bundle) so export
/// takes them without running real analysis.
pub fn seed_tracks_as_analyzed(data_dir: &Path, track_ids: &[String]) {
    let conn = rusqlite::Connection::open(data_dir.join("backend.db")).expect("open backend db");
    for (idx, track_id) in track_ids.iter().enumerate() {
        let bundle = seed_test_analysis_bundle(data_dir, &format!("test-waveform-{idx}"));
        conn.execute(
            "UPDATE tracks
             SET bpm = 120.0,
                 tonality = '8A',
                 duration_ms = 180000,
                 waveform_peaks_path = ?1
             WHERE id = ?2",
            rusqlite::params![bundle.to_string_lossy().to_string(), track_id],
        )
        .expect("seed analyzed track fields");
    }
}

/// The stick's vendor database folder.
pub fn vendor_db_dir(usb_root: &Path) -> PathBuf {
    usb_root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)
}

/// The stick's PDB.
pub fn pdb_path(usb_root: &Path) -> PathBuf {
    vendor_db_dir(usb_root).join("export.pdb")
}

/// Opens an eDB, unlocking it with the SQLCipher key when it is encrypted.
pub fn open_edb(path: &Path) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(path).expect("open eDB");
    let has_schema = conn
        .query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    if has_schema == 0 {
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
            .expect("apply SQLCipher key");
    }
    conn
}

pub fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

/// Copies an audio fixture to `target`, adding the target's file name as an
/// extra metadata block so every copy is a different file. The library keeps
/// one track per file content, so tests that use one fixture for several
/// tracks need distinct bytes. Audio data and existing metadata are kept.
pub fn copy_unique_audio(source: &Path, target: &Path) {
    let bytes = fs::read(source).expect("read audio fixture");
    let marker = target
        .file_name()
        .expect("target file name")
        .to_string_lossy()
        .into_owned()
        .into_bytes();
    let unique = match &bytes[..4] {
        b"RIFF" => with_trailing_chunk(&bytes, *b"JUNK", &marker, u32::to_le_bytes),
        b"FORM" => with_trailing_chunk(&bytes, *b"ANNO", &marker, u32::to_be_bytes),
        b"fLaC" => with_flac_application_block(&bytes, &marker),
        [b'I', b'D', b'3', _] => with_id3v23_priv_frame(&bytes, &marker),
        _ => panic!("unsupported audio fixture: {}", source.display()),
    };
    fs::write(target, unique).expect("write audio fixture copy");
}

/// RIFF (little-endian sizes) and AIFF FORM (big-endian) share the layout:
/// append a chunk and grow the container size at offset 4.
fn with_trailing_chunk(
    bytes: &[u8],
    id: [u8; 4],
    data: &[u8],
    encode: fn(u32) -> [u8; 4],
) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out.extend_from_slice(&id);
    out.extend_from_slice(&encode(data.len() as u32));
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    let container = encode((out.len() - 8) as u32);
    out[4..8].copy_from_slice(&container);
    out
}

/// Inserts an APPLICATION block right after STREAMINFO (which must stay first).
fn with_flac_application_block(bytes: &[u8], data: &[u8]) -> Vec<u8> {
    let streaminfo_header = 4;
    let streaminfo_len = u32::from_be_bytes([0, bytes[5], bytes[6], bytes[7]]) as usize;
    let streaminfo_end = streaminfo_header + 4 + streaminfo_len;
    let streaminfo_was_last = bytes[4] & 0x80 != 0;

    let mut block = vec![if streaminfo_was_last { 0x82 } else { 0x02 }];
    let len = (4 + data.len()) as u32;
    block.extend_from_slice(&len.to_be_bytes()[1..]);
    block.extend_from_slice(b"test");
    block.extend_from_slice(data);

    let mut out = bytes[..streaminfo_end].to_vec();
    out[4] &= 0x7f;
    out.extend_from_slice(&block);
    out.extend_from_slice(&bytes[streaminfo_end..]);
    out
}

/// Inserts a PRIV frame at the start of an ID3v2.3 tag and grows the tag size.
fn with_id3v23_priv_frame(bytes: &[u8], data: &[u8]) -> Vec<u8> {
    assert_eq!(bytes[3], 3, "fixture tag must be ID3v2.3");
    assert_eq!(bytes[5], 0, "fixture tag must have no header flags");
    let syncsafe = |b: &[u8]| {
        b.iter()
            .fold(0u32, |size, &byte| (size << 7) | u32::from(byte & 0x7f))
    };
    let tag_size = syncsafe(&bytes[6..10]);

    let mut payload = b"dj-usb-tkit-test\0".to_vec();
    payload.extend_from_slice(data);
    let mut frame = b"PRIV".to_vec();
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&payload);

    let new_size = tag_size + frame.len() as u32;
    let mut out = bytes[..6].to_vec();
    out.extend((0..4).rev().map(|i| ((new_size >> (7 * i)) & 0x7f) as u8));
    out.extend_from_slice(&frame);
    out.extend_from_slice(&bytes[10..]);
    out
}
