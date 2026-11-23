//! Initializing a USB stick: the player folders and an empty PDB and eDB
//! shaped like the ones rekordbox writes to a freshly prepared stick.

use super::root::can_write_file_at;
use super::vendor_compat::{
    DEFAULT_USB_EDB_KEY, USB_CONTENTS_DIR, USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR, vendor_db_dir,
    vendor_pdb_path,
};
use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::pdb_page;
use std::path::Path;

pub fn initialize_usb(root: &Path) -> BackendResult<crate::models::InitializeUsbData> {
    initialize_usb_for(None, root)
}

/// [`initialize_usb`], with `service`'s parse caches dropped once the
/// databases are written.
pub(crate) fn initialize_usb_for(
    service: Option<&crate::service::BackendService>,
    root: &Path,
) -> BackendResult<crate::models::InitializeUsbData> {
    let write_guard = service.map(crate::service::BackendService::lock_usb_writes);
    if !root.is_dir() {
        return Err(BackendError::Internal(format!(
            "USB root does not exist: {}",
            root.display()
        )));
    }
    if !can_write_file_at(root) {
        return Err(BackendError::Validation(tr::usb_error_read_only_init()));
    }

    let dirs_to_create = [
        root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR),
        root.join(USB_CONTENTS_DIR),
    ];

    // The day the USB is initialized: the eDB's `property.createdDate` and
    // the t19 template row carry it, as on a USB rekordbox initializes.
    let created_date = chrono::Local::now().format("%Y-%m-%d").to_string();

    let mut created = Vec::new();
    for dir in &dirs_to_create {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
            created.push(dir.to_string_lossy().to_string());
        }
    }

    // Create each full-shape database the stick lacks (the PDB baseline, the
    // encrypted eDB), through a write session like every other write: built
    // in the staged copy, then written back to the USB.
    // Staged afresh, as on connect: a local copy left from an earlier
    // session isn't this stick's database.
    let mut session = match (service, write_guard.as_ref()) {
        (Some(service), Some(guard)) => service.usb_write_session(
            guard,
            root,
            crate::service::usb::staging::SessionBackup::None,
            &mut Vec::new(),
        ),
        _ => crate::service::usb::staging::UsbWriteSession::without_service(root),
    };
    let present = crate::service::usb::staging::UsbDatabases::on_connect(root);
    let mut created_files = Vec::new();
    for kind in [
        crate::service::usb::staging::DbKind::Pdb,
        crate::service::usb::staging::DbKind::Edb,
    ] {
        if present.as_ref().is_some_and(|dbs| dbs.has(kind)) {
            continue;
        }
        session.create_database(kind, &created_date)?;
        created_files.push(match kind {
            crate::service::usb::staging::DbKind::Pdb => vendor_pdb_path(root),
            crate::service::usb::staging::DbKind::Edb => {
                vendor_db_dir(root).join("exportLibrary.db")
            }
        });
    }
    session.commit()?;
    created.extend(
        created_files
            .iter()
            .map(|file| file.to_string_lossy().to_string()),
    );

    Ok(crate::models::InitializeUsbData {
        path: root.to_string_lossy().into_owned(),
        created_dirs: created,
        status_text: crate::i18n::tr::usb_initialized().render(),
    })
}

pub(crate) fn initialize_fullshape_edb(db_path: &Path, created_date: &str) -> BackendResult<()> {
    let conn = rusqlite::Connection::open(db_path)?;
    conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))?;
    conn.execute_batch(EDB_SCHEMA)?;
    seed_edb_defaults(&conn, created_date)?;
    Ok(())
}

pub(crate) const EDB_SCHEMA: &str = r#"
CREATE TABLE album(album_id integer primary key, name varchar, artist_id integer, image_id integer, isComplation integer, nameForSearch varchar);
CREATE TABLE artist(artist_id integer primary key, name varchar, nameForSearch varchar);
CREATE TABLE category(category_id integer primary key, menuItem_id integer, sequenceNo integer, isVisible integer);
CREATE TABLE color(color_id integer primary key, name varchar);
CREATE TABLE content(content_id integer primary key, title varchar, titleForSearch varchar, subtitle varchar, bpmx100 integer, length integer, trackNo integer, discNo integer, artist_id_artist integer, artist_id_remixer integer, artist_id_originalArtist integer, artist_id_composer integer, artist_id_lyricist integer, album_id integer, genre_id integer, label_id integer, key_id integer, color_id integer, image_id integer, djComment varchar, rating integer, releaseYear integer, releaseDate varchar, dateCreated varchar, dateAdded varchar, path varchar, fileName varchar, fileSize integer, fileType integer, bitrate integer, bitDepth integer, samplingRate integer, isrc varchar, djPlayCount integer, isHotCueAutoLoadOn integer, isKuvoDeliverStatusOn integer, kuvoDeliveryComment varchar, masterDbId integer, masterContentId integer, analysisDataFilePath varchar, analysedBits integer, contentLink integer, hasModified integer, cueUpdateCount integer, analysisDataUpdateCount integer, informationUpdateCount integer);
CREATE TABLE cue(cue_id integer primary key, content_id integer, kind integer, colorTableIndex integer, cueComment varchar, isActiveLoop integer, beatLoopNumerator integer, beatLoopDenominator integer, inUsec integer, outUsec integer, in150FramePerSec integer, out150FramePerSec integer, inMpegFrameNumber integer, outMpegFrameNumber integer, inMpegAbs integer, outMpegAbs integer, inDecodingStartFramePosition integer, outDecodingStartFramePosition integer, inFileOffsetInBlock integer, OutFileOffsetInBlock integer, inNumberOfSampleInBlock integer, outNumberOfSampleInBlock integer);
CREATE TABLE genre(genre_id integer primary key, name varchar);
CREATE TABLE history(history_id integer primary key, sequenceNo integer, name varchar, attribute integer, history_id_parent integer);
CREATE TABLE history_content(history_id integer, content_id integer, sequenceNo integer);
CREATE TABLE hotCueBankList(hotCueBankList_id integer primary key, sequenceNo integer, name varchar, image_id integer, attribute integer, hotCueBankList_id_parent integer);
CREATE TABLE hotCueBankList_cue(hotCueBankList_id integer, cue_id integer, sequenceNo integer);
CREATE TABLE image(image_id integer primary key, path varchar);
CREATE TABLE key(key_id integer primary key, name varchar);
CREATE TABLE label(label_id integer primary key, name varchar);
CREATE TABLE menuItem(menuItem_id integer primary key, kind integer, name varchar);
CREATE TABLE myTag(myTag_id integer primary key, sequenceNo integer, name varchar, attribute integer, myTag_id_parent integer);
CREATE TABLE myTag_content(myTag_id integer, content_id integer);
CREATE TABLE playlist(playlist_id integer primary key, sequenceNo integer, name varchar, image_id integer, attribute integer, playlist_id_parent integer);
CREATE TABLE playlist_content(playlist_id integer, content_id integer, sequenceNo integer);
CREATE TABLE property(deviceName varchar, dbVersion varchar, numberOfContents integer, createdDate varchar, backGroundColorType integer, myTagMasterDBID integer);
CREATE TABLE recommendedLike(content_id_1 integer, content_id_2 integer, rating integer, createdDate integer);
CREATE TABLE sort(sort_id integer primary key, menuItem_id integer, sequenceNo integer, isVisible integer, isSelectedAsSubColumn integer);
CREATE INDEX index_hotCueBankList_cue_hotCueBankList_id on hotCueBankList_cue(hotCueBankList_id);
CREATE INDEX index_myTag_content_content_id on myTag_content(content_id);
CREATE INDEX index_myTag_content_myTag_id on myTag_content(myTag_id);
CREATE INDEX index_playlist_content_playlist_id on playlist_content(playlist_id);
"#;

fn seed_edb_defaults(conn: &rusqlite::Connection, created_date: &str) -> BackendResult<()> {
    // property
    conn.execute(
        "INSERT INTO property (deviceName, dbVersion, numberOfContents, createdDate, backGroundColorType, myTagMasterDBID) VALUES ('', ?1, 0, ?2, ?3, 969967066)",
        rusqlite::params![
            crate::pdb_writer::T19_DB_VERSION,
            created_date,
            crate::pdb_writer::INIT_BACKGROUND_COLOR
        ],
    )?;

    // color (8 standard player colors)
    let colors = [
        (1, "Pink"),
        (2, "Red"),
        (3, "Orange"),
        (4, "Yellow"),
        (5, "Green"),
        (6, "Aqua"),
        (7, "Blue"),
        (8, "Purple"),
    ];
    for (id, name) in &colors {
        conn.execute(
            "INSERT INTO color (color_id, name) VALUES (?1, ?2)",
            rusqlite::params![id, name],
        )?;
    }

    // menuItem/category/sort defaults must match the working reference export baseline.
    // Freshly initialized USBs rely on these tables before any additive export can
    // preserve reference rows from an existing device database.
    let menu_items: [(i32, i32, &str); 27] = [
        (1, 128, "\u{FFFA}GENRE\u{FFFB}"),
        (2, 129, "\u{FFFA}ARTIST\u{FFFB}"),
        (3, 130, "\u{FFFA}ALBUM\u{FFFB}"),
        (4, 131, "\u{FFFA}TRACK\u{FFFB}"),
        (5, 133, "\u{FFFA}BPM\u{FFFB}"),
        (6, 134, "\u{FFFA}RATING\u{FFFB}"),
        (7, 135, "\u{FFFA}YEAR\u{FFFB}"),
        (8, 136, "\u{FFFA}REMIXER\u{FFFB}"),
        (9, 137, "\u{FFFA}LABEL\u{FFFB}"),
        (10, 138, "\u{FFFA}ORIGINAL ARTIST\u{FFFB}"),
        (11, 139, "\u{FFFA}KEY\u{FFFB}"),
        (12, 141, "\u{FFFA}CUE\u{FFFB}"),
        (13, 142, "\u{FFFA}COLOR\u{FFFB}"),
        (14, 146, "\u{FFFA}TIME\u{FFFB}"),
        (15, 147, "\u{FFFA}BITRATE\u{FFFB}"),
        (16, 148, "\u{FFFA}FILE NAME\u{FFFB}"),
        (17, 132, "\u{FFFA}PLAYLIST\u{FFFB}"),
        (18, 152, "\u{FFFA}HOT CUE BANK\u{FFFB}"),
        (19, 149, "\u{FFFA}HISTORY\u{FFFB}"),
        (20, 145, "\u{FFFA}SEARCH\u{FFFB}"),
        (21, 150, "\u{FFFA}COMMENTS\u{FFFB}"),
        (22, 140, "\u{FFFA}DATE ADDED\u{FFFB}"),
        (23, 151, "\u{FFFA}DJ PLAY COUNT\u{FFFB}"),
        (24, 144, "\u{FFFA}FOLDER\u{FFFB}"),
        (25, 161, "\u{FFFA}DEFAULT\u{FFFB}"),
        (26, 162, "\u{FFFA}ALPHABET\u{FFFB}"),
        (27, 170, "\u{FFFA}MATCHING\u{FFFB}"),
    ];
    for (id, kind, name) in &menu_items {
        conn.execute(
            "INSERT INTO menuItem (menuItem_id, kind, name) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, kind, name],
        )?;
    }

    // category seed mirrors PDB t16 (the master menu source) and matches the
    // Default visible set: ARTIST, ALBUM, TRACK, KEY, PLAYLIST, HISTORY,
    // SEARCH, MATCHING, FOLDER, DATE ADDED. eDB.menuItem keeps the full 27
    // catalog so the Menu Editor can promote a hidden kind back to visible
    // (which patches PDB t16 + eDB.category atomically). The 5 menuItems
    // observed without a category row in vendor exports (CUE, COMMENTS, DJ PLAY COUNT,
    // DEFAULT, ALPHABET) are intentionally omitted here to keep the eDB
    // shape byte-comparable to vendor exports.
    let category_rows: [(i32, i32, i32, i32); 22] = [
        (1, 1, 0, 0),    // GENRE
        (2, 2, 1, 1),    // ARTIST
        (3, 3, 2, 1),    // ALBUM
        (4, 4, 3, 1),    // TRACK
        (5, 17, 5, 1),   // PLAYLIST
        (6, 5, 0, 0),    // BPM
        (7, 6, 0, 0),    // RATING
        (8, 7, 0, 0),    // YEAR
        (9, 8, 0, 0),    // REMIXER
        (10, 9, 0, 0),   // LABEL
        (11, 10, 0, 0),  // ORIGINAL ARTIST
        (12, 11, 4, 1),  // KEY
        (15, 13, 0, 0),  // COLOR
        (17, 24, 9, 1),  // FOLDER
        (18, 20, 7, 1),  // SEARCH
        (19, 14, 0, 0),  // TIME
        (20, 15, 0, 0),  // BITRATE
        (21, 16, 0, 0),  // FILE NAME
        (22, 19, 6, 1),  // HISTORY
        (23, 18, 0, 0),  // HOT CUE BANK
        (26, 27, 8, 1),  // MATCHING
        (27, 22, 10, 1), // DATE ADDED
    ];
    for (id, mi, seq, vis) in &category_rows {
        conn.execute(
            "INSERT INTO category (category_id, menuItem_id, sequenceNo, isVisible) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, mi, seq, vis],
        )?;
    }

    // sort (17 rows)
    let sorts: [(i32, i32, i32, i32, i32); 17] = [
        (0, 25, 1, 1, 0),
        (1, 26, 2, 1, 0),
        (2, 2, 3, 1, 0),
        (3, 3, 4, 1, 0),
        (4, 5, 5, 1, 0),
        (5, 6, 6, 1, 0),
        (6, 1, 0, 0, 0),
        (7, 21, 0, 0, 0),
        (8, 14, 0, 0, 0),
        (9, 8, 0, 0, 0),
        (10, 9, 0, 0, 0),
        (11, 10, 0, 0, 0),
        (12, 11, 7, 1, 0),
        (13, 15, 0, 0, 0),
        (15, 13, 0, 0, 0),
        (16, 23, 0, 0, 0),
        (17, 22, 0, 0, 0),
    ];
    for (id, mi, seq, vis, sel) in &sorts {
        conn.execute("INSERT INTO sort (sort_id, menuItem_id, sequenceNo, isVisible, isSelectedAsSubColumn) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![id, mi, seq, vis, sel])?;
    }

    // myTag (28 rows — 4 top-level groups + children)
    let my_tags: [(i64, i32, &str, i32, i64); 28] = [
        (1, 1, "Genre", 1, 0),
        (2, 2, "Components", 1, 0),
        (3, 3, "Situation", 1, 0),
        (4, 4, "Untitled Column", 1, 0),
        (302295111, 1, "Acid House", 0, 1),
        (550969514, 2, "Deep House", 0, 1),
        (674668498, 3, "Techno", 0, 1),
        (886498498, 4, "Nu Disco", 0, 1),
        (986312498, 5, "Electro House", 0, 1),
        (1001102498, 6, "Bass Music", 0, 1),
        (1133242498, 7, "Trap", 0, 1),
        (268245111, 1, "Synth", 0, 2),
        (468969514, 2, "Vocal", 0, 2),
        (574668498, 3, "Beat", 0, 2),
        (786498498, 4, "Sub Bass", 0, 2),
        (886312498, 5, "Percussion", 0, 2),
        (901102498, 6, "Piano", 0, 2),
        (1033242498, 7, "Dark", 0, 2),
        (1133242499, 8, "Upper", 0, 2),
        (202295111, 1, "Main Floor", 0, 3),
        (350969514, 2, "Second Floor", 0, 3),
        (474668498, 3, "Lounge", 0, 3),
        (686498498, 4, "Mid Night", 0, 3),
        (786312498, 5, "Morning", 0, 3),
        (801102498, 6, "Build up", 0, 3),
        (933242498, 7, "Peak Time", 0, 3),
        (1033242499, 8, "Build down", 0, 3),
        (168245111, 1, "My Comment", 0, 4),
    ];
    for (id, seq, name, attr, parent) in &my_tags {
        conn.execute("INSERT INTO myTag (myTag_id, sequenceNo, name, attribute, myTag_id_parent) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![id, seq, name, attr, parent])?;
    }

    Ok(())
}

pub(crate) fn build_fullshape_pdb_bytes(created_date: &str) -> Vec<u8> {
    use crate::pdb_writer::{PdbData, standard_colors, standard_columns_raw, write_pdb};
    let data = PdbData {
        colors: standard_colors(),
        columns_raw_rows: standard_columns_raw(),
        ..PdbData::empty()
    };
    let mut bytes = write_pdb(&data).expect("build empty PDB");
    seed_rb_initialized_history_shape(&mut bytes, created_date);
    // The fresh writer starts seq_counter at 100, leaving seqdb=102 in the template.
    // Reset seqdb to 34 — template max_seqpage is now 31 (from tt=19 seq=31, matching
    // reference exporter template), so seqdb=34 satisfies seqdb > max_seqpage. The additive writer
    // will assign new pages seq=32+ on first export, matching reference export range.
    if let Some(sl) = bytes.get_mut(0x14..0x18) {
        sl.copy_from_slice(&34u32.to_le_bytes());
    }
    bytes
}

fn seed_rb_initialized_history_shape(bytes: &mut [u8], created_date: &str) {
    use crate::utils::{set_table_ptr_fields, write_u8_at, write_u16_le_at, write_u32_le_at};
    const PAGE_SIZE: usize = 4096;
    if bytes.len() < PAGE_SIZE * 40 {
        return;
    }

    struct PageHeaderFields {
        page_idx: u32,
        table_type: u32,
        next_page: u32,
        seq: u32,
        u3: u8,
    }

    fn write_data_page(
        bytes: &mut [u8],
        header: PageHeaderFields,
        rows: &[&[u8]],
        rowpf_words: &[u16],
        tranrf_words: &[u16],
    ) {
        let PageHeaderFields {
            page_idx,
            table_type,
            next_page,
            seq,
            u3,
        } = header;
        let base = page_idx as usize * PAGE_SIZE;
        let Some(page) = bytes.get_mut(base..base + PAGE_SIZE) else {
            return;
        };
        page.fill(0);

        let nrs = rows.len();
        let mut payload = Vec::<u8>::new();
        let mut offsets = Vec::<u16>::with_capacity(nrs);
        for row in rows {
            offsets.push(payload.len() as u16);
            payload.extend_from_slice(row);
        }
        let used_s = payload.len();
        let groups = nrs.div_ceil(pdb_page::GROUP_SLOTS);
        let free_s = PAGE_SIZE
            .saturating_sub(pdb_page::HEAP_START)
            .saturating_sub(used_s)
            .saturating_sub(pdb_page::footer_size(nrs));

        write_u32_le_at(page, 0x04, page_idx);
        write_u32_le_at(page, 0x08, table_type);
        write_u32_le_at(page, 0x0c, next_page);
        write_u32_le_at(page, 0x2c, next_page);
        write_u32_le_at(page, 0x10, seq);
        page[0x18] = nrs as u8;
        page[0x19] = u3;
        page[0x1a] = if groups >= 2 { 2 } else { 0 };
        page[0x1b] = 36;
        write_u16_le_at(page, 0x1c, free_s as u16);
        write_u16_le_at(page, 0x1e, used_s as u16);
        write_u16_le_at(page, 0x20, nrs as u16);
        write_u16_le_at(page, 0x22, 0);

        page[pdb_page::HEAP_START..pdb_page::HEAP_START + used_s].copy_from_slice(&payload);

        pdb_page::write_footer(page, &offsets, |group| {
            (
                rowpf_words.get(group.index).copied().unwrap_or(0),
                tranrf_words.get(group.index).copied().unwrap_or(0),
            )
        });
    }

    // Row order and u4 values match reference exporter history_playlist_rows constants.
    let t17_rows: [[u8; 8]; 22] = [
        [0x0f, 0x00, 0x14, 0x00, 0x06, 0x01, 0x00, 0x00], // u1=15
        [0x10, 0x00, 0x15, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=16
        [0x12, 0x00, 0x17, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=18
        [0x08, 0x00, 0x09, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=8
        [0x09, 0x00, 0x0a, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=9
        [0x0a, 0x00, 0x0b, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=10
        [0x0d, 0x00, 0x0f, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=13
        [0x0e, 0x00, 0x13, 0x00, 0x04, 0x01, 0x00, 0x00], // u1=14
        [0x01, 0x00, 0x01, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=1
        [0x05, 0x00, 0x06, 0x00, 0x05, 0x01, 0x00, 0x00], // u1=5
        [0x06, 0x00, 0x07, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=6
        [0x07, 0x00, 0x08, 0x00, 0x63, 0x01, 0x00, 0x00], // u1=7
        [0x02, 0x00, 0x02, 0x00, 0x02, 0x00, 0x01, 0x00], // u1=2,  u5=1
        [0x03, 0x00, 0x03, 0x00, 0x03, 0x00, 0x02, 0x00], // u1=3,  u5=2
        [0x04, 0x00, 0x04, 0x00, 0x01, 0x00, 0x03, 0x00], // u1=4,  u5=3
        [0x0b, 0x00, 0x0c, 0x00, 0x63, 0x00, 0x04, 0x00], // u1=11, u5=4
        [0x11, 0x00, 0x05, 0x00, 0x63, 0x00, 0x05, 0x00], // u1=17, u5=5
        [0x13, 0x00, 0x16, 0x00, 0x63, 0x00, 0x06, 0x00], // u1=19, u5=6
        [0x14, 0x00, 0x12, 0x00, 0x63, 0x00, 0x07, 0x00], // u1=20, u5=7
        [0x1b, 0x00, 0x1a, 0x00, 0x63, 0x02, 0x08, 0x00], // u1=27, u5=8, u4=2
        [0x18, 0x00, 0x11, 0x00, 0x63, 0x00, 0x09, 0x00], // u1=24, u5=9
        [0x16, 0x00, 0x1b, 0x00, 0x63, 0x05, 0x0a, 0x00], // u1=22, u5=10, u4=5
    ];
    let t18_rows: [[u8; 8]; 17] = [
        [0x01, 0x00, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x15, 0x00, 0x07, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x0e, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x08, 0x00, 0x09, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x09, 0x00, 0x0a, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x0a, 0x00, 0x0b, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x0f, 0x00, 0x0d, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x0d, 0x00, 0x0f, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x17, 0x00, 0x10, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x16, 0x00, 0x11, 0x00, 0x01, 0x00, 0x00, 0x00],
        [0x19, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00],
        [0x1a, 0x00, 0x01, 0x00, 0x00, 0x02, 0x00, 0x00],
        [0x02, 0x00, 0x02, 0x00, 0x00, 0x03, 0x00, 0x00],
        [0x03, 0x00, 0x03, 0x00, 0x00, 0x04, 0x00, 0x00],
        [0x05, 0x00, 0x04, 0x00, 0x00, 0x05, 0x00, 0x00],
        [0x06, 0x00, 0x05, 0x00, 0x00, 0x06, 0x00, 0x00],
        [0x0b, 0x00, 0x0c, 0x00, 0x00, 0x07, 0x00, 0x00],
    ];

    let t17_refs = t17_rows.iter().map(|r| r.as_slice()).collect::<Vec<_>>();
    let t18_refs = t18_rows.iter().map(|r| r.as_slice()).collect::<Vec<_>>();
    write_data_page(
        bytes,
        PageHeaderFields {
            page_idx: 36,
            table_type: 17,
            next_page: 44,
            seq: 4,
            u3: 0xc0,
        },
        &t17_refs,
        &[0xffff, 0x003f],
        &[0xffff, 0x003f],
    );
    write_data_page(
        bytes,
        PageHeaderFields {
            page_idx: 38,
            table_type: 18,
            next_page: 45,
            seq: 5,
            u3: 0x20,
        },
        &t18_refs,
        &[0xffff, 0x0001],
        &[0xffff, 0x0001],
    );
    write_data_page(
        bytes,
        PageHeaderFields {
            page_idx: 40,
            table_type: 19,
            next_page: 41,
            seq: 31,
            u3: 0x20,
        },
        &[&crate::pdb_writer::t19_template_row(created_date)],
        &[0x0001],
        &[0x0001],
    );

    // Repoint empty-candidate chains to reference export shape.
    set_table_ptr_fields(bytes, 6, 42, 13, 14);
    set_table_ptr_fields(bytes, 16, 43, 33, 34);
    set_table_ptr_fields(bytes, 17, 44, 35, 36);
    set_table_ptr_fields(bytes, 18, 45, 37, 38);
    set_table_ptr_fields(bytes, 19, 41, 39, 40);

    // Reference baseline increments seq/next on populated colors/columns pages.
    write_u32_le_at(bytes, 14 * PAGE_SIZE + 0x0c, 42);
    write_u32_le_at(bytes, 14 * PAGE_SIZE + 0x10, 2);

    write_u32_le_at(bytes, 34 * PAGE_SIZE + 0x0c, 43);
    write_u32_le_at(bytes, 34 * PAGE_SIZE + 0x10, 3);

    // Match observed reference flags on these two pages.
    write_u8_at(bytes, 14 * PAGE_SIZE + 0x19, 0x00);
    write_u8_at(bytes, 34 * PAGE_SIZE + 0x19, 0x60);

    // Keep file-level next-unallocated-page pointer consistent with highest virtual EC.
    write_u32_le_at(bytes, 0x0c, 46);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BackendError;
    use crate::pdb_reader::parse_pdb;
    use crate::service::usb::root::can_write_file_at;
    use crate::service::usb::root::tests::with_read_only_folder;
    use crate::service::usb::vendor_compat::{
        DEFAULT_USB_EDB_KEY, USB_CONTENTS_DIR, USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR,
        vendor_pdb_path,
    };

    #[test]
    fn initialize_usb_refuses_a_usb_it_cannot_write() {
        with_read_only_folder(|path| {
            // Root writes anyway, so there's nothing to refuse.
            if can_write_file_at(path) {
                return;
            }
            match initialize_usb(path).unwrap_err() {
                BackendError::Validation(message) => {
                    assert_eq!(
                        message.render_english(),
                        "The USB is read-only, so it can't be initialized."
                    );
                }
                other => panic!("unexpected error: {other:?}"),
            }
            assert_eq!(std::fs::read_dir(path).unwrap().count(), 0);
        });
    }

    #[test]
    fn initialize_usb_creates_structure() {
        let tmp = std::env::temp_dir().join("__test_init_usb__");
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp).ok();
        }
        std::fs::create_dir_all(&tmp).unwrap();

        let result = initialize_usb(&tmp).unwrap();
        assert_eq!(result.path, tmp.to_str().unwrap());
        assert!(!result.created_dirs.is_empty());

        // Verify structure
        assert!(
            tmp.join(USB_VENDOR_ROOT_DIR)
                .join(USB_VENDOR_DB_DIR)
                .is_dir()
        );
        assert!(tmp.join(USB_CONTENTS_DIR).is_dir());
        assert!(
            tmp.join(USB_VENDOR_ROOT_DIR)
                .join(USB_VENDOR_DB_DIR)
                .join("export.pdb")
                .is_file()
        );
        let parsed = parse_pdb(
            &tmp.join(USB_VENDOR_ROOT_DIR)
                .join(USB_VENDOR_DB_DIR)
                .join("export.pdb"),
        )
        .expect("parse initialized PDB");
        assert!(
            parsed.tracks.is_empty()
                && parsed.playlist_tree.is_empty()
                && parsed.playlist_entries.is_empty(),
            "initialized PDB should be valid and empty"
        );

        // The t19 template row and the eDB's createdDate both carry today.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let pdb = std::fs::read(vendor_pdb_path(&tmp)).unwrap();
        let row0 = 40 * 4096 + 0x28;
        assert_eq!(&pdb[row0 + 13..row0 + 23], today.as_bytes());
        let property = crate::service::export_helpers::edb_t19_property(&tmp).expect("eDB");
        assert_eq!(property.created_date, today);
        // Both databases start purple.
        assert_eq!(pdb[row0 + 9], crate::pdb_writer::INIT_BACKGROUND_COLOR);
        assert_eq!(
            property.background_color,
            crate::pdb_writer::INIT_BACKGROUND_COLOR
        );

        // Running initialize again should be idempotent (no error)
        let result2 = initialize_usb(&tmp).unwrap();
        assert!(
            result2.created_dirs.is_empty(),
            "second run creates nothing"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn initialize_usb_writes_pdb_and_edb_through_staging_when_enabled() {
        let usb = tempfile::tempdir().expect("usb tempdir");
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));

        let result = initialize_usb(usb.path()).unwrap();
        assert!(!result.created_dirs.is_empty());

        let db_dir = usb.path().join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        let pdb_usb_path = db_dir.join("export.pdb");
        let edb_usb_path = db_dir.join("exportLibrary.db");
        assert!(pdb_usb_path.is_file(), "PDB must be committed to the USB");
        assert!(edb_usb_path.is_file(), "eDB must be committed to the USB");
        parse_pdb(&pdb_usb_path).expect("committed PDB must parse");
        rusqlite::Connection::open(&edb_usb_path).expect("committed eDB must open");

        // No leftover atomic-commit temp files beside either database.
        let leftover_tmp = std::fs::read_dir(&db_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("tmp-"));
        assert!(
            !leftover_tmp,
            "atomic commit must not leave a temp file behind"
        );

        // Both files must actually have gone through the local cache, not
        // just been written straight to the USB.
        let staged_pdb =
            crate::service::usb::staging::stage_pdb(usb.path()).expect("stage pdb after init");
        let staged_edb =
            crate::service::usb::staging::stage_edb(usb.path()).expect("stage edb after init");
        assert_ne!(
            staged_pdb, pdb_usb_path,
            "PDB should be staged to a local cache path"
        );
        assert_ne!(
            staged_edb, edb_usb_path,
            "eDB should be staged to a local cache path"
        );
        assert_eq!(
            std::fs::read(&staged_pdb).unwrap(),
            std::fs::read(&pdb_usb_path).unwrap()
        );
        assert_eq!(
            std::fs::read(&staged_edb).unwrap(),
            std::fs::read(&edb_usb_path).unwrap()
        );

        // Re-staging must not need to re-copy: the write-back already
        // recorded the synced state, so the cached copy is trusted as-is.
        let mtime_before = std::fs::metadata(&staged_edb).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let staged_edb_again =
            crate::service::usb::staging::stage_edb(usb.path()).expect("stage edb again");
        let mtime_after = std::fs::metadata(&staged_edb_again)
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "unchanged USB source after init should not trigger a re-copy"
        );
    }

    #[test]
    fn initialize_usb_recovers_when_usb_wiped_but_local_cache_is_stale() {
        let usb = tempfile::tempdir().expect("usb tempdir");
        let cache_dir = tempfile::tempdir().expect("cache dir").keep();
        let _guard = crate::service::usb::staging::set_cache_root_for_test(Some(cache_dir));

        // First init: populates both the USB and the local cache with a
        // full PDB/eDB.
        initialize_usb(usb.path()).unwrap();

        // Simulate the user manually deleting PIONEER and Contents directly
        // on the USB, then reconnecting -- the local cache directory (same
        // identity, since it's the same mount path) is left untouched and
        // now holds a stale, fully-populated eDB.
        std::fs::remove_dir_all(usb.path().join(USB_VENDOR_ROOT_DIR)).unwrap();
        std::fs::remove_dir_all(usb.path().join(USB_CONTENTS_DIR)).unwrap();

        // Re-initializing must succeed, not fail with a stale-schema DB error.
        let result =
            initialize_usb(usb.path()).expect("re-initialize after manual wipe should succeed");
        assert!(!result.created_dirs.is_empty());

        let edb_usb_path = usb
            .path()
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db");
        let conn = rusqlite::Connection::open(&edb_usb_path).expect("re-created eDB must open");
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
            .expect("re-created eDB must decrypt with the standard key");
        let content_count: i64 = conn
            .query_row("SELECT count(*) FROM content", [], |row| row.get(0))
            .expect("content table must exist and be queryable");
        assert_eq!(content_count, 0, "re-created eDB should be freshly empty");
    }

    #[test]
    fn t19_property_keeps_the_app_version_whatever_the_edb_says() {
        // Libraries rekordbox upgraded say `10000` in `dbVersion` while
        // their t19 rows keep `"1000"`; t19 never takes the eDB's.
        let tmp = tempfile::tempdir().expect("tempdir");
        initialize_usb(tmp.path()).expect("initialize usb");
        let db_path = tmp
            .path()
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db");
        let conn = rusqlite::Connection::open(db_path).expect("open exportLibrary.db");
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
            .expect("apply SQLCipher key");
        conn.execute(
            "UPDATE property SET dbVersion = '10000', deviceName = 'Stick'",
            [],
        )
        .expect("update property");
        drop(conn);
        let property = crate::service::export_helpers::edb_t19_property(tmp.path()).expect("eDB");
        assert_eq!(property.db_version, crate::pdb_writer::T19_DB_VERSION);
        assert_eq!(property.device_name.as_deref(), Some("Stick"));
    }

    #[test]
    fn initialize_usb_seeds_reference_menu_defaults() {
        let tmp = tempfile::tempdir().expect("tempdir");
        initialize_usb(tmp.path()).expect("initialize usb");

        let db_path = tmp
            .path()
            .join(USB_VENDOR_ROOT_DIR)
            .join(USB_VENDOR_DB_DIR)
            .join("exportLibrary.db");
        let conn = rusqlite::Connection::open(db_path).expect("open exportLibrary.db");
        conn.execute_batch(&format!("PRAGMA key='{DEFAULT_USB_EDB_KEY}';"))
            .expect("apply SQLCipher key");

        let db_version: String = conn
            .query_row("SELECT dbVersion FROM property LIMIT 1", [], |row| {
                row.get(0)
            })
            .expect("property row");
        assert_eq!(db_version, "1000");

        let menu_rows = conn
            .prepare("SELECT menuItem_id, kind, name FROM menuItem ORDER BY menuItem_id")
            .expect("prepare menuItem")
            .query_map([], |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("query menuItem")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect menuItem");
        assert_eq!(
            menu_rows.iter().find(|(id, _, _)| *id == 17),
            Some(&(17, 132, "\u{FFFA}PLAYLIST\u{FFFB}".to_string()))
        );
        assert_eq!(
            menu_rows.iter().find(|(id, _, _)| *id == 5),
            Some(&(5, 133, "\u{FFFA}BPM\u{FFFB}".to_string()))
        );

        let category_rows = conn
            .prepare("SELECT category_id, menuItem_id, sequenceNo, isVisible FROM category ORDER BY category_id")
            .expect("prepare category")
            .query_map([], |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, i32>(3)?,
                ))
            })
            .expect("query category")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect category");
        // Init must produce the default visible set: 10 visible
        // (ARTIST/ALBUM/TRACK/KEY/PLAYLIST/HISTORY/SEARCH/MATCHING/FOLDER/
        // DATE ADDED) and 12 hidden, mirrored on the PDB side so older and
        // newer players see the same menu out of the box.
        assert_eq!(
            category_rows.len(),
            22,
            "expected 22 category rows in default layout"
        );
        let visible = category_rows
            .iter()
            .filter(|(_, _, _, vis)| *vis == 1)
            .count();
        assert_eq!(visible, 10, "default has 10 visible category rows");
        // Spot-check key rows.
        assert_eq!(
            category_rows.iter().find(|(id, _, _, _)| *id == 5),
            Some(&(5, 17, 5, 1)),
            "PLAYLIST visible at seq 5"
        );
        assert_eq!(
            category_rows.iter().find(|(id, _, _, _)| *id == 27),
            Some(&(27, 22, 10, 1)),
            "DATE ADDED visible at seq 10"
        );
        assert_eq!(
            category_rows.iter().find(|(id, _, _, _)| *id == 1),
            Some(&(1, 1, 0, 0)),
            "GENRE hidden by default"
        );

        let sort_rows = conn
            .prepare("SELECT sort_id, menuItem_id, sequenceNo, isVisible, isSelectedAsSubColumn FROM sort ORDER BY sort_id")
            .expect("prepare sort")
            .query_map([], |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, i32>(3)?,
                    row.get::<_, i32>(4)?,
                ))
            })
            .expect("query sort")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect sort");
        assert_eq!(
            sort_rows.iter().find(|(id, _, _, _, _)| *id == 0),
            Some(&(0, 25, 1, 1, 0))
        );
        assert_eq!(
            sort_rows.iter().find(|(id, _, _, _, _)| *id == 12),
            Some(&(12, 11, 7, 1, 0))
        );
        assert_eq!(
            sort_rows.iter().find(|(id, _, _, _, _)| *id == 17),
            Some(&(17, 22, 0, 0, 0))
        );
    }
}
