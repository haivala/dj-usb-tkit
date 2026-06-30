use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;

use backend::pdb_reader::parse_pdb;

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("error: {}", message.as_ref());
    std::process::exit(1);
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 || args.len() > 4 {
        eprintln!(
            "usage: cargo run --bin dump_pdb_playlist -- <PDB path> <playlist name> [--json|--duplicates]"
        );
        std::process::exit(2);
    }

    let pdb_path = PathBuf::from(&args[1]);
    let playlist_name = args[2].to_string();
    let emit_json = args.get(3).map(|v| v == "--json").unwrap_or(false);
    let emit_duplicates = args.get(3).map(|v| v == "--duplicates").unwrap_or(false);

    if playlist_name.is_empty() {
        fail("playlist name must not be empty");
    }
    if !pdb_path.is_file() {
        fail(format!(
            "pdb path does not exist or is not a file: {}",
            pdb_path.display()
        ));
    }

    let parsed = parse_pdb(&pdb_path)
        .unwrap_or_else(|err| fail(format!("failed to parse {}: {:?}", pdb_path.display(), err)));

    let matching_nodes = parsed
        .playlist_tree
        .iter()
        .filter(|row| !row.row_is_folder && row.name == playlist_name)
        .cloned()
        .collect::<Vec<_>>();

    if matching_nodes.is_empty() {
        fail(format!(
            "playlist '{}' not found in {}",
            playlist_name,
            pdb_path.display()
        ));
    }

    let artist_by_id = parsed.artists;
    let album_by_id = parsed.albums;
    let key_by_id = parsed.keys;
    let artwork_by_id = parsed.artworks;
    let track_by_id = parsed
        .tracks
        .into_iter()
        .map(|track| (track.id, track))
        .collect::<std::collections::HashMap<_, _>>();

    let mut printed = HashSet::<u32>::new();
    let mut results = Vec::<PlaylistDump>::new();

    for node in matching_nodes {
        if !printed.insert(node.id) {
            continue;
        }

        let mut entries = parsed
            .playlist_entries
            .iter()
            .filter(|entry| entry.playlist_id == node.id)
            .cloned()
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.entry_index);

        let tracks = entries
            .into_iter()
            .map(|entry| {
                let track = track_by_id.get(&entry.track_id);
                let artist = track.and_then(|t| artist_by_id.get(&t.artist_id)).cloned();
                let album = track.and_then(|t| album_by_id.get(&t.album_id)).cloned();
                let key = track.and_then(|t| key_by_id.get(&t.key_id)).cloned();
                let artwork_path = track
                    .and_then(|t| artwork_by_id.get(&t.artwork_id))
                    .cloned();

                TrackDump {
                    entry_index: entry.entry_index,
                    track_id: entry.track_id,
                    title: track.map(|t| t.title.clone()),
                    artist,
                    album,
                    key,
                    track_number: track.map(|t| t.track_number),
                    tempo_x100: track.map(|t| t.tempo_x100),
                    duration_seconds: track.and_then(|t| t.duration_seconds),
                    anlz_path: track.map(|t| t.anlz_path.clone()),
                    track_file_path: track.map(|t| t.track_file_path.clone()),
                    artwork_id: track.map(|t| t.artwork_id),
                    artwork_path,
                    artist_id: track.map(|t| t.artist_id),
                    album_id: track.map(|t| t.album_id),
                    key_id: track.map(|t| t.key_id),
                    missing_track_row: track.is_none(),
                }
            })
            .collect::<Vec<_>>();

        results.push(PlaylistDump {
            playlist_id: node.id,
            playlist_name: node.name,
            parent_id: node.parent_id,
            sort_order: node.sort_order,
            track_count: tracks.len(),
            tracks,
        });
    }

    if emit_duplicates {
        print_duplicates(&results);
    } else if emit_json {
        print_json(&results);
    } else {
        print_text(&results, &pdb_path);
    }
}

#[derive(Debug, Clone)]
struct PlaylistDump {
    playlist_id: u32,
    playlist_name: String,
    parent_id: u32,
    sort_order: u32,
    track_count: usize,
    tracks: Vec<TrackDump>,
}

#[derive(Debug, Clone)]
struct TrackDump {
    entry_index: u32,
    track_id: u32,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    key: Option<String>,
    track_number: Option<u32>,
    tempo_x100: Option<u32>,
    duration_seconds: Option<u32>,
    anlz_path: Option<String>,
    track_file_path: Option<String>,
    artwork_id: Option<u32>,
    artwork_path: Option<String>,
    artist_id: Option<u32>,
    album_id: Option<u32>,
    key_id: Option<u32>,
    missing_track_row: bool,
}

fn print_text(results: &[PlaylistDump], pdb_path: &PathBuf) {
    println!("pdb={}", pdb_path.display());
    for playlist in results {
        println!(
            "playlist|id={}|name={}|parent_id={}|sort_order={}|track_count={}",
            playlist.playlist_id,
            escape_text(&playlist.playlist_name),
            playlist.parent_id,
            playlist.sort_order,
            playlist.track_count
        );
        for track in &playlist.tracks {
            println!(
                concat!(
                    "track|entry_index={}|track_id={}|title={}|artist={}|album={}|key={}|",
                    "track_number={}|tempo_x100={}|duration_seconds={}|anlz_path={}|track_file_path={}|",
                    "artwork_id={}|artwork_path={}|artist_id={}|album_id={}|key_id={}|missing_track_row={}"
                ),
                track.entry_index,
                track.track_id,
                escape_opt(track.title.as_deref()),
                escape_opt(track.artist.as_deref()),
                escape_opt(track.album.as_deref()),
                escape_opt(track.key.as_deref()),
                fmt_opt_u32(track.track_number),
                fmt_opt_u32(track.tempo_x100),
                fmt_opt_u32(track.duration_seconds),
                escape_opt(track.anlz_path.as_deref()),
                escape_opt(track.track_file_path.as_deref()),
                fmt_opt_u32(track.artwork_id),
                escape_opt(track.artwork_path.as_deref()),
                fmt_opt_u32(track.artist_id),
                fmt_opt_u32(track.album_id),
                fmt_opt_u32(track.key_id),
                track.missing_track_row
            );
        }
    }
}

fn print_json(results: &[PlaylistDump]) {
    println!("[");
    for (idx, playlist) in results.iter().enumerate() {
        if idx > 0 {
            println!(",");
        }
        println!("  {{");
        println!("    \"playlist_id\": {},", playlist.playlist_id);
        println!(
            "    \"playlist_name\": \"{}\",",
            escape_json(&playlist.playlist_name)
        );
        println!("    \"parent_id\": {},", playlist.parent_id);
        println!("    \"sort_order\": {},", playlist.sort_order);
        println!("    \"track_count\": {},", playlist.track_count);
        println!("    \"tracks\": [");
        for (tidx, track) in playlist.tracks.iter().enumerate() {
            if tidx > 0 {
                println!(",");
            }
            println!("      {{");
            println!("        \"entry_index\": {},", track.entry_index);
            println!("        \"track_id\": {},", track.track_id);
            println!(
                "        \"title\": {},",
                json_opt_str(track.title.as_deref())
            );
            println!(
                "        \"artist\": {},",
                json_opt_str(track.artist.as_deref())
            );
            println!(
                "        \"album\": {},",
                json_opt_str(track.album.as_deref())
            );
            println!("        \"key\": {},", json_opt_str(track.key.as_deref()));
            println!(
                "        \"track_number\": {},",
                json_opt_u32(track.track_number)
            );
            println!(
                "        \"tempo_x100\": {},",
                json_opt_u32(track.tempo_x100)
            );
            println!(
                "        \"duration_seconds\": {},",
                json_opt_u32(track.duration_seconds)
            );
            println!(
                "        \"anlz_path\": {},",
                json_opt_str(track.anlz_path.as_deref())
            );
            println!(
                "        \"track_file_path\": {},",
                json_opt_str(track.track_file_path.as_deref())
            );
            println!(
                "        \"artwork_id\": {},",
                json_opt_u32(track.artwork_id)
            );
            println!(
                "        \"artwork_path\": {},",
                json_opt_str(track.artwork_path.as_deref())
            );
            println!("        \"artist_id\": {},", json_opt_u32(track.artist_id));
            println!("        \"album_id\": {},", json_opt_u32(track.album_id));
            println!("        \"key_id\": {},", json_opt_u32(track.key_id));
            println!("        \"missing_track_row\": {}", track.missing_track_row);
            print!("      }}");
        }
        println!();
        println!("    ]");
        print!("  }}");
    }
    println!();
    println!("]");
}

fn print_duplicates(results: &[PlaylistDump]) {
    for playlist in results {
        let mut counts = HashMap::<String, Vec<&TrackDump>>::new();
        for track in &playlist.tracks {
            let key = track_identity_key(
                track.track_file_path.as_deref().unwrap_or(""),
                track.title.as_deref().unwrap_or(""),
                track.artist.as_deref().unwrap_or(""),
                Some(&track.track_id.to_string()),
            );
            counts.entry(key).or_default().push(track);
        }

        println!(
            "playlist|id={}|name={}|track_count={}",
            playlist.playlist_id,
            escape_text(&playlist.playlist_name),
            playlist.track_count
        );
        for (identity, tracks) in counts.into_iter().filter(|(_, rows)| rows.len() > 1) {
            println!(
                "duplicate|identity={}|count={}",
                escape_text(&identity),
                tracks.len()
            );
            for track in tracks {
                println!(
                    "track|entry_index={}|track_id={}|title={}|artist={}|track_file_path={}",
                    track.entry_index,
                    track.track_id,
                    escape_opt(track.title.as_deref()),
                    escape_opt(track.artist.as_deref()),
                    escape_opt(track.track_file_path.as_deref())
                );
            }
        }
    }
}

fn track_identity_key(
    file_path: &str,
    title: &str,
    artist: &str,
    id_fallback: Option<&str>,
) -> String {
    let path = normalize_track_path_for_identity(file_path);
    if !path.is_empty() {
        return format!("path:{path}");
    }

    let meta = format!(
        "meta:{}|{}",
        canonicalize_identity_part(title),
        canonicalize_identity_part(artist)
    );
    if meta != "meta:|" {
        return meta;
    }

    if let Some(id) = id_fallback.map(str::trim).filter(|id| !id.is_empty()) {
        return format!("id:{id}");
    }

    "unknown".to_string()
}

fn normalize_track_path_for_identity(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty() {
        return String::new();
    }

    let lower = normalized.to_ascii_lowercase();
    if let Some(idx) = lower.rfind("/contents/") {
        normalized[idx..].to_string()
    } else if lower.starts_with("contents/") {
        format!("/{normalized}")
    } else {
        normalized
    }
}

fn canonicalize_identity_part(value: &str) -> String {
    value
        .trim()
        .chars()
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn fmt_opt_u32(value: Option<u32>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

fn json_opt_u32(value: Option<u32>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn json_opt_str(value: Option<&str>) -> String {
    value
        .map(|v| format!("\"{}\"", escape_json(v)))
        .unwrap_or_else(|| "null".to_string())
}

fn escape_opt(value: Option<&str>) -> String {
    value.map(escape_text).unwrap_or_else(|| "NULL".to_string())
}

fn escape_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', "\\n")
}

fn escape_json(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
