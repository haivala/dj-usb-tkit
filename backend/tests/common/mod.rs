//! Helpers shared by the functional test crates.
//!
//! Each test binary compiles this module separately and uses only part of it.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use backend::service::usb::vendor_compat::{
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

/// An ANLZ file with no chunks: its 28-byte header, the file length in bytes
/// 8..12. The app accepts it as an undamaged analysis file.
pub fn empty_anlz_file() -> Vec<u8> {
    let mut file = b"PMAI".to_vec();
    file.extend_from_slice(&28u32.to_be_bytes());
    file.extend_from_slice(&28u32.to_be_bytes());
    file.resize(28, 0);
    file
}

/// `value` as a short ASCII PDB string (kind `len * 2 + 3`), as players
/// write their history session names (`HISTORY 001`).
pub fn short_ascii_pdb_string(value: &str) -> Vec<u8> {
    assert!(
        value.is_ascii() && value.len() <= 126,
        "not a short ASCII string"
    );
    let mut out = vec![(value.len() * 2 + 3) as u8];
    out.extend_from_slice(value.as_bytes());
    out
}

/// Writes placeholder (chunkless) DAT/EXT/2EX files into the app's analysis
/// cache and returns the DAT path.
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
    for ext in ["DAT", "EXT", "2EX"] {
        fs::write(dir.join(format!("{safe_stem}.{ext}")), empty_anlz_file())
            .expect("write test analysis file");
    }
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

/// The UI's JS files, relative to `vanilla-ui/`, sorted.
pub fn ui_files(ui_dir: &Path) -> Vec<PathBuf> {
    const SKIP: &[&str] = &[
        "node_modules",
        "dist",
        "tests",
        "scripts",
        "test-results",
        "assets",
        "playwright.config.mjs",
    ];
    let mut files = Vec::new();
    let mut dirs = vec![ui_dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("read vanilla-ui") {
            let path = entry.expect("dir entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if path.is_dir() {
                if !SKIP.contains(&name) && !name.starts_with('.') {
                    dirs.push(path);
                }
            } else if (name.ends_with(".mjs") || name.ends_with(".js")) && !SKIP.contains(&name) {
                files.push(path.strip_prefix(ui_dir).expect("inside").to_path_buf());
            }
        }
    }
    files.sort();
    files
}

// ---------------------------------------------------------------------------
// Player history written into a PDB
// ---------------------------------------------------------------------------

/// Appends a player history session to the stick's PDB as a player writes
/// one: a `t11` session row named `HISTORY 1` (`history_id`) and its `t12`
/// entries for `track_ids`, in order, each on a new page.
pub fn append_history_to_pdb(pdb_path: &Path, history_id: u32, track_ids: &[u32]) {
    let mut bytes = fs::read(pdb_path).expect("read PDB");
    let len_page = read_u32_le(&bytes, 4).expect("len_page") as usize;
    let num_tables = read_u32_le(&bytes, 8).expect("num_tables") as usize;
    let max_page = parse_max_last_page(&bytes, num_tables) as u32;

    let history_playlist_page = max_page + 1;
    let history_entries_page = max_page + 2;

    let playlist_row = build_history_playlist_row(history_id, "HISTORY 1");
    let entry_rows = track_ids
        .iter()
        .enumerate()
        .map(|(idx, track_id)| build_history_entry_row(*track_id, history_id, (idx + 1) as u32))
        .collect::<Vec<_>>();

    let playlist_page = build_pdb_page(
        11,
        history_playlist_page,
        history_playlist_page,
        &[playlist_row],
        len_page,
    );
    let entries_page = build_pdb_page(
        12,
        history_entries_page,
        history_entries_page,
        &entry_rows,
        len_page,
    );

    bytes.extend_from_slice(&playlist_page);
    bytes.extend_from_slice(&entries_page);

    let p1 = 28 + num_tables * 16;
    let p2 = p1 + 16;
    bytes[p1..p1 + 4].copy_from_slice(&11u32.to_le_bytes());
    bytes[p1 + 8..p1 + 12].copy_from_slice(&history_playlist_page.to_le_bytes());
    bytes[p1 + 12..p1 + 16].copy_from_slice(&history_playlist_page.to_le_bytes());

    bytes[p2..p2 + 4].copy_from_slice(&12u32.to_le_bytes());
    bytes[p2 + 8..p2 + 12].copy_from_slice(&history_entries_page.to_le_bytes());
    bytes[p2 + 12..p2 + 16].copy_from_slice(&history_entries_page.to_le_bytes());

    bytes[8..12].copy_from_slice(&((num_tables as u32) + 2).to_le_bytes());
    fs::write(pdb_path, bytes).expect("write PDB with history tables");
}

fn build_history_playlist_row(id: u32, name: &str) -> Vec<u8> {
    let mut row = Vec::<u8>::new();
    row.extend_from_slice(&id.to_le_bytes());
    row.extend_from_slice(&short_ascii_pdb_string(name));
    row
}

fn build_history_entry_row(track_id: u32, playlist_id: u32, entry_index: u32) -> Vec<u8> {
    let mut row = vec![0u8; 12];
    row[0..4].copy_from_slice(&track_id.to_le_bytes());
    row[4..8].copy_from_slice(&playlist_id.to_le_bytes());
    row[8..12].copy_from_slice(&entry_index.to_le_bytes());
    row
}

fn build_pdb_page(
    table_type: u32,
    page_index: u32,
    seq: u32,
    rows: &[Vec<u8>],
    len_page: usize,
) -> Vec<u8> {
    let mut page = vec![0u8; len_page];
    page[4..8].copy_from_slice(&page_index.to_le_bytes());
    page[8..12].copy_from_slice(&table_type.to_le_bytes());
    page[12..16].copy_from_slice(&0u32.to_le_bytes());
    page[16..20].copy_from_slice(&seq.to_le_bytes());

    let mut payload_offset = 0usize;
    let mut row_offsets = Vec::<u16>::new();
    for row in rows {
        row_offsets.push(payload_offset as u16);
        let start = 40 + payload_offset;
        let end = start + row.len();
        page[start..end].copy_from_slice(row);
        payload_offset += row.len();
    }

    page[24] = (rows.len() % 256) as u8;
    page[30..32].copy_from_slice(&(payload_offset as u16).to_le_bytes());
    page[34..36].copy_from_slice(&((rows.len().saturating_sub(1)) as u16).to_le_bytes());

    let mut cursor = len_page;
    for group_start in (0..rows.len()).step_by(16) {
        cursor -= 4;
        let group_len = (rows.len() - group_start).min(16);
        let bits = ((1u32 << group_len) - 1) as u16;
        page[cursor..cursor + 2].copy_from_slice(&bits.to_le_bytes());
        for j in 0..group_len {
            cursor -= 2;
            page[cursor..cursor + 2].copy_from_slice(&row_offsets[group_start + j].to_le_bytes());
        }
    }

    page
}

fn parse_max_last_page(bytes: &[u8], num_tables: usize) -> usize {
    let mut cursor = 28usize;
    let mut max_page = 0usize;
    for _ in 0..num_tables {
        let Some(last_page) = read_u32_le(bytes, cursor + 12) else {
            break;
        };
        max_page = max_page.max(last_page as usize);
        cursor += 16;
    }
    max_page
}

/// The stick's eDB.
pub fn edb_path(usb_root: &Path) -> PathBuf {
    vendor_db_dir(usb_root).join("exportLibrary.db")
}

/// Points a library track at `artwork_path` in the app's database.
pub fn seed_track_artwork_path(data_dir: &Path, track_id: &str, artwork_path: &Path) {
    let conn = rusqlite::Connection::open(data_dir.join("backend.db")).expect("open backend db");
    conn.execute(
        "UPDATE tracks SET artwork_path = ?1 WHERE id = ?2",
        rusqlite::params![artwork_path.to_string_lossy().as_ref(), track_id],
    )
    .expect("seed track artwork path");
}

pub fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

pub fn write_u16_le(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
