/// regenerate_pdb <source_usb_or_pdb> <output_pdb>
///
/// Parse all data from an existing PDB and write a structurally fresh PDB
/// containing the same tracks, playlists, artists, albums, etc.
///
/// Diagnostic tool: if the regenerated PDB is accepted by DJ software but the
/// original (repaired) PDB is rejected, the issue is in the original page
/// structure / row encoding, not in the data values themselves.
///
/// WARNING: rebuilding a PDB from scratch relocates table chains. This breaks
/// player hardware compatibility. Only use for DJ software diagnostic testing.
use std::env;
use std::path::{Path, PathBuf};

use backend::pdb_reader::parse_pdb;
use backend::pdb_writer::{
    PdbAlbumRow, PdbArtistRow, PdbArtworkRow, PdbData, PdbDictRow, PdbHistoryEntryRow, PdbKeyRow,
    PdbPlaylistEntryRow, PdbPlaylistTreeRow, standard_colors, standard_columns_raw, write_pdb,
};
use backend::service::export_helpers::PdbTrackRowData;

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: regenerate_pdb <source_usb_or_pdb> <output_pdb>");
        std::process::exit(2);
    }
    let source = Path::new(&args[1]);
    let dest = Path::new(&args[2]);

    let src_pdb = resolve_pdb(source);
    let parsed = match parse_pdb(&src_pdb) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("parse error: {e}");
            std::process::exit(1);
        }
    };

    eprintln!(
        "parsed: tracks={} artists={} albums={} genres={} labels={} \
         keys={} artworks={} playlist_tree={} playlist_entries={} \
         history_pl={} history_ent={}",
        parsed.tracks.len(),
        parsed.artists.len(),
        parsed.albums.len(),
        parsed.genres.len(),
        parsed.labels.len(),
        parsed.keys.len(),
        parsed.artworks.len(),
        parsed.playlist_tree.len(),
        parsed.playlist_entries.len(),
        parsed.history_playlists.len(),
        parsed.history_entries.len(),
    );

    // Build PdbData from parsed content
    let mut data = PdbData::empty();

    // Tracks
    for t in &parsed.tracks {
        data.tracks.push(PdbTrackRowData {
            header_flags_u32: None,
            id: t.id,
            artist_id: t.artist_id,
            album_id: t.album_id,
            artwork_id: t.artwork_id,
            key_id: t.key_id,
            genre_id: t.genre_id,
            title: t.title.clone(),
            anlz_path: t.anlz_path.clone(),
            file_path: t.track_file_path.clone(),
            content_link: t.content_link,
            sample_rate_hz: t.sample_rate_hz,
            file_size_bytes: t.file_size_bytes,
            master_content_id: t.master_content_id,
            master_db_id: t.master_db_id,
            bitrate_kbps: t.bitrate_kbps,
            track_number: if t.track_number > 0 {
                Some(t.track_number)
            } else {
                None
            },
            bpm: if t.tempo_x100 > 0 {
                Some(t.tempo_x100 as f64 / 100.0)
            } else {
                None
            },
            release_year: t.release_year,
            bit_depth: t.bit_depth,
            duration_seconds: t.duration_seconds,
            file_type: t.file_type,
            isrc: t.isrc.clone(),
            date_added: t.date_added.clone(),
            release_date: t.release_date.clone(),
            dj_comment: t.dj_comment.clone(),
            file_name: t.file_name.clone(),
            publish_track_info_on: None,
            autoload_hotcues_on: None,
        });
    }

    // Artists
    let mut artist_ids: Vec<u32> = parsed.artists.keys().copied().collect();
    artist_ids.sort();
    for id in artist_ids {
        data.artists.push(PdbArtistRow {
            id,
            name: parsed.artists[&id].clone(),
        });
    }

    // Albums
    let mut album_ids: Vec<u32> = parsed.albums.keys().copied().collect();
    album_ids.sort();
    for id in album_ids {
        // album artist_id not easily recoverable from ParsedPdb — use 0
        data.albums.push(PdbAlbumRow {
            id,
            name: parsed.albums[&id].clone(),
            artist_id: 0,
        });
    }

    // Genres
    let mut genre_ids: Vec<u32> = parsed.genres.keys().copied().collect();
    genre_ids.sort();
    for id in genre_ids {
        data.genres.push(PdbDictRow {
            id,
            name: parsed.genres[&id].clone(),
        });
    }

    // Labels
    let mut label_ids: Vec<u32> = parsed.labels.keys().copied().collect();
    label_ids.sort();
    for id in label_ids {
        data.labels.push(PdbDictRow {
            id,
            name: parsed.labels[&id].clone(),
        });
    }

    // Keys
    let mut key_ids: Vec<u32> = parsed.keys.keys().copied().collect();
    key_ids.sort();
    for id in key_ids {
        data.keys.push(PdbKeyRow {
            id,
            name: parsed.keys[&id].clone(),
        });
    }

    // Colors — always use standard set
    data.colors = standard_colors();

    // Artworks
    let mut artwork_ids: Vec<u32> = parsed.artworks.keys().copied().collect();
    artwork_ids.sort();
    for id in artwork_ids {
        data.artwork.push(PdbArtworkRow {
            id,
            path: parsed.artworks[&id].clone(),
        });
    }

    // Playlist tree
    for p in &parsed.playlist_tree {
        data.playlist_tree.push(PdbPlaylistTreeRow {
            id: p.id,
            parent_id: p.parent_id,
            sort_order: p.sort_order,
            is_folder: p.row_is_folder,
            name: p.name.clone(),
        });
    }

    // Playlist entries
    for e in &parsed.playlist_entries {
        data.playlist_entries.push(PdbPlaylistEntryRow {
            entry_index: e.entry_index,
            track_id: e.track_id,
            playlist_id: e.playlist_id,
        });
    }

    // Columns — always use standard set
    data.columns_raw_rows = standard_columns_raw();

    // History playlists
    for h in &parsed.history_playlists {
        data.history_playlists.push(PdbDictRow {
            id: h.id,
            name: h.name.clone(),
        });
    }

    // History entries
    for h in &parsed.history_entries {
        if let Some(track_id) = h.track_id {
            data.history_entries.push(PdbHistoryEntryRow {
                track_id,
                playlist_id: h.playlist_id,
                entry_index: h.entry_index,
            });
        }
    }

    // History raw rows
    data.history_raw_rows = parsed.history_raw_rows_bytes.clone();

    eprintln!(
        "writing fresh PDB: tracks={} artists={} albums={} keys={} artworks={} \
         playlist_tree={} playlist_entries={}",
        data.tracks.len(),
        data.artists.len(),
        data.albums.len(),
        data.keys.len(),
        data.artwork.len(),
        data.playlist_tree.len(),
        data.playlist_entries.len(),
    );

    let bytes = match write_pdb(&data) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("write error: {e}");
            std::process::exit(1);
        }
    };

    std::fs::write(dest, &bytes).unwrap_or_else(|e| {
        eprintln!("write file error: {e}");
        std::process::exit(1);
    });

    eprintln!(
        "wrote {} bytes ({} pages) to {}",
        bytes.len(),
        bytes.len() / 4096,
        dest.display()
    );
}

fn resolve_pdb(input: &Path) -> PathBuf {
    if input
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("export.pdb"))
        .unwrap_or(false)
    {
        return input.to_path_buf();
    }
    input.join("PIONEER").join("rekordbox").join("export.pdb")
}
