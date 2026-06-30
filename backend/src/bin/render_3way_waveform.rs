use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use image::{Rgb, RgbImage};
use rusqlite::{Connection, types::ValueRef};

const DEFAULT_USB_EXPORT_KEY: &str =
    "r8gddnr4k847830ar6cqzbkk0el6qytmb3trbbx805jm74vez64i5o8fnrqryqls";
const DEFAULT_MASTER_KEY: &str = "402fd_d44f42a8_eb0f6d4db0e6b";

// 3-way waveform colors
const COLOR_LOW: Rgb<u8> = Rgb([40, 100, 220]);
const COLOR_MID: Rgb<u8> = Rgb([230, 170, 30]);
const COLOR_HIGH: Rgb<u8> = Rgb([230, 230, 230]);
const COLOR_BG: Rgb<u8> = Rgb([12, 12, 16]);

// PWV4 lane colors
const COLOR_B0_AMP: Rgb<u8> = Rgb([200, 200, 200]); // amplitude — white
const COLOR_B1_LUM: Rgb<u8> = Rgb([160, 160, 80]); // luminance — yellow-grey
const COLOR_B2_BLEND: Rgb<u8> = Rgb([80, 160, 160]); // low+mid blend — teal
const COLOR_B3_LOW: Rgb<u8> = Rgb([40, 100, 220]); // low — blue
const COLOR_B4_MID: Rgb<u8> = Rgb([230, 170, 30]); // mid — amber
const COLOR_B5_HIGH: Rgb<u8> = Rgb([230, 230, 230]); // high/front — white

const RENDER_WIDTH: u32 = 1200;
const WAVEFORM_HEIGHT: u32 = 120;
const LANE_HEIGHT: u32 = 48;
const SEPARATOR_HEIGHT: u32 = 4;
const LANE_GAP: u32 = 2;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 {
        eprintln!(
            "usage: cargo run --features dev-tools --bin render_3way_waveform -- <usb_root> <out_dir> [--compare <usb_b>] [--pwv4]"
        );
        std::process::exit(2);
    }
    let usb_a = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);

    let mut usb_b: Option<PathBuf> = None;
    let mut pwv4_mode = false;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--compare" if i + 1 < args.len() => {
                usb_b = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--pwv4" => {
                pwv4_mode = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    fs::create_dir_all(&out_dir).expect("create out_dir");

    if pwv4_mode {
        run_pwv4(&usb_a, usb_b.as_deref(), &out_dir);
    } else {
        run_pwv7(&usb_a, usb_b.as_deref(), &out_dir);
    }
}

// ---------------------------------------------------------------------------
// PWV7 (3-band detail) rendering
// ---------------------------------------------------------------------------

fn run_pwv7(usb_a: &Path, usb_b: Option<&Path>, out_dir: &Path) {
    let tracks_a = load_pwv7_waveforms(usb_a);
    if tracks_a.is_empty() {
        eprintln!("no PWV7 waveforms found in {}", usb_a.display());
        std::process::exit(1);
    }

    if let Some(ub) = usb_b {
        let tracks_b = load_pwv7_waveforms(ub);
        let mut rendered = 0usize;
        for (path_key, (low_a, mid_a, high_a)) in &tracks_a {
            if let Some((low_b, mid_b, high_b)) = tracks_b.get(path_key) {
                let slug = slugify(path_key);
                let total_h = WAVEFORM_HEIGHT * 2 + SEPARATOR_HEIGHT;
                let mut img = RgbImage::from_pixel(RENDER_WIDTH, total_h, COLOR_BG);
                let (dl_a, dm_a, dh_a) =
                    downsample_bands(low_a, mid_a, high_a, RENDER_WIDTH as usize);
                let (dl_b, dm_b, dh_b) =
                    downsample_bands(low_b, mid_b, high_b, RENDER_WIDTH as usize);
                draw_3way_filled(&mut img, &dl_a, &dm_a, &dh_a, 0, WAVEFORM_HEIGHT);
                draw_3way_filled(
                    &mut img,
                    &dl_b,
                    &dm_b,
                    &dh_b,
                    WAVEFORM_HEIGHT + SEPARATOR_HEIGHT,
                    WAVEFORM_HEIGHT,
                );
                let path = out_dir.join(format!("{slug}.3way_compare.png"));
                img.save(&path).expect("save compare png");
                rendered += 1;
            }
        }
        println!(
            "rendered {rendered} PWV7 comparison waveforms to {}",
            out_dir.display()
        );
    } else {
        let mut rendered = 0usize;
        for (path_key, (low, mid, high)) in &tracks_a {
            let slug = slugify(path_key);
            let (dl, dm, dh) = downsample_bands(low, mid, high, RENDER_WIDTH as usize);
            let mut img = RgbImage::from_pixel(RENDER_WIDTH, WAVEFORM_HEIGHT, COLOR_BG);
            draw_3way_filled(&mut img, &dl, &dm, &dh, 0, WAVEFORM_HEIGHT);
            let path = out_dir.join(format!("{slug}.3way.png"));
            img.save(&path).expect("save png");
            rendered += 1;
        }
        println!(
            "rendered {rendered} PWV7 waveforms to {}",
            out_dir.display()
        );
    }
}

// ---------------------------------------------------------------------------
// PWV4 (6-lane preview) rendering
// ---------------------------------------------------------------------------

fn run_pwv4(usb_a: &Path, usb_b: Option<&Path>, out_dir: &Path) {
    let tracks_a = load_pwv4_waveforms(usb_a);
    if tracks_a.is_empty() {
        eprintln!("no PWV4 waveforms found in {}", usb_a.display());
        std::process::exit(1);
    }

    let lane_labels = ["b0:amp", "b1:lum", "b2:lo+mi", "b3:low", "b4:mid", "b5:hi"];
    let lane_colors = [
        COLOR_B0_AMP,
        COLOR_B1_LUM,
        COLOR_B2_BLEND,
        COLOR_B3_LOW,
        COLOR_B4_MID,
        COLOR_B5_HIGH,
    ];

    if let Some(ub) = usb_b {
        let tracks_b = load_pwv4_waveforms(ub);
        let mut rendered = 0usize;
        // Two sets of 6 lanes stacked vertically with a separator
        let one_set_h = 6 * LANE_HEIGHT + 5 * LANE_GAP;
        let total_h = one_set_h * 2 + SEPARATOR_HEIGHT;

        for (path_key, lanes_a) in &tracks_a {
            if let Some(lanes_b) = tracks_b.get(path_key) {
                let slug = slugify(path_key);
                let width = RENDER_WIDTH;
                let mut img = RgbImage::from_pixel(width, total_h, COLOR_BG);

                for lane_idx in 0..6 {
                    let y_off = (LANE_HEIGHT + LANE_GAP) * lane_idx as u32;
                    draw_lane_filled(
                        &mut img,
                        &lanes_a[lane_idx],
                        y_off,
                        LANE_HEIGHT,
                        lane_colors[lane_idx],
                        lane_max(lane_idx),
                    );
                }
                for lane_idx in 0..6 {
                    let y_off =
                        one_set_h + SEPARATOR_HEIGHT + (LANE_HEIGHT + LANE_GAP) * lane_idx as u32;
                    draw_lane_filled(
                        &mut img,
                        &lanes_b[lane_idx],
                        y_off,
                        LANE_HEIGHT,
                        lane_colors[lane_idx],
                        lane_max(lane_idx),
                    );
                }

                let path = out_dir.join(format!("{slug}.pwv4_compare.png"));
                img.save(&path).expect("save pwv4 compare png");
                rendered += 1;
                eprintln!(
                    "  {}: {} lanes: {}",
                    slug,
                    lane_labels.join(", "),
                    lane_labels
                        .iter()
                        .enumerate()
                        .map(|(li, l)| {
                            let max_a = lanes_a[li].iter().copied().max().unwrap_or(0);
                            let max_b = lanes_b[li].iter().copied().max().unwrap_or(0);
                            format!("{l} max={max_a}/{max_b}")
                        })
                        .collect::<Vec<_>>()
                        .join("  ")
                );
            }
        }
        println!(
            "rendered {rendered} PWV4 comparison waveforms to {}",
            out_dir.display()
        );
    } else {
        let mut rendered = 0usize;
        let total_h = 6 * LANE_HEIGHT + 5 * LANE_GAP;

        for (path_key, lanes) in &tracks_a {
            let slug = slugify(path_key);
            let mut img = RgbImage::from_pixel(RENDER_WIDTH, total_h, COLOR_BG);

            for lane_idx in 0..6 {
                let y_off = (LANE_HEIGHT + LANE_GAP) * lane_idx as u32;
                draw_lane_filled(
                    &mut img,
                    &lanes[lane_idx],
                    y_off,
                    LANE_HEIGHT,
                    lane_colors[lane_idx],
                    lane_max(lane_idx),
                );
            }

            let path = out_dir.join(format!("{slug}.pwv4.png"));
            img.save(&path).expect("save pwv4 png");
            rendered += 1;
        }
        println!(
            "rendered {rendered} PWV4 waveforms to {}",
            out_dir.display()
        );
    }
}

/// b1 (luminance) ranges 0-255, all others 0-127.
fn lane_max(lane_idx: usize) -> f32 {
    if lane_idx == 1 { 255.0 } else { 127.0 }
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

/// Downsample 3 bands to target_bins using RMS per bin.
fn downsample_bands(
    low: &[u8],
    mid: &[u8],
    high: &[u8],
    target_bins: usize,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let len = low.len().min(mid.len()).min(high.len());
    if len == 0 || target_bins == 0 {
        return (vec![], vec![], vec![]);
    }
    let mut dl = Vec::with_capacity(target_bins);
    let mut dm = Vec::with_capacity(target_bins);
    let mut dh = Vec::with_capacity(target_bins);
    for i in 0..target_bins {
        let start = i * len / target_bins;
        let end = ((i + 1) * len / target_bins).max(start + 1).min(len);
        let mut sl = 0.0f64;
        let mut sm = 0.0f64;
        let mut sh = 0.0f64;
        let count = (end - start) as f64;
        for j in start..end {
            sl += (low[j] as f64) * (low[j] as f64);
            sm += (mid[j] as f64) * (mid[j] as f64);
            sh += (high[j] as f64) * (high[j] as f64);
        }
        dl.push((sl / count).sqrt().round().min(127.0) as u8);
        dm.push((sm / count).sqrt().round().min(127.0) as u8);
        dh.push((sh / count).sqrt().round().min(127.0) as u8);
    }
    (dl, dm, dh)
}

fn draw_3way_filled(
    img: &mut RgbImage,
    low: &[u8],
    mid: &[u8],
    high: &[u8],
    y_offset: u32,
    height: u32,
) {
    let len = low.len().min(mid.len()).min(high.len());
    for x in 0..len {
        if x as u32 >= img.width() {
            break;
        }
        let l = low[x] as f32;
        let m = mid[x] as f32;
        let h = high[x] as f32;

        let scale = height as f32 / 127.0;
        let low_px = (l * scale * 0.33).round() as u32;
        let mid_px = (m * scale * 0.33).round() as u32;
        let high_px = (h * scale * 0.33).round() as u32;

        let bottom = y_offset + height;

        for dy in 0..low_px.min(height) {
            let y = bottom.saturating_sub(1 + dy);
            if y >= y_offset {
                img.put_pixel(x as u32, y, COLOR_LOW);
            }
        }
        for dy in 0..mid_px.min(height) {
            let y = bottom.saturating_sub(1 + low_px + dy);
            if y >= y_offset {
                img.put_pixel(x as u32, y, COLOR_MID);
            }
        }
        for dy in 0..high_px.min(height) {
            let y = bottom.saturating_sub(1 + low_px + mid_px + dy);
            if y >= y_offset {
                img.put_pixel(x as u32, y, COLOR_HIGH);
            }
        }
    }
}

/// Draw a single lane as filled bars from bottom up.
fn draw_lane_filled(
    img: &mut RgbImage,
    values: &[u8],
    y_offset: u32,
    height: u32,
    color: Rgb<u8>,
    max_val: f32,
) {
    let width = img.width() as usize;
    let len = values.len();
    if len == 0 {
        return;
    }
    let bottom = y_offset + height;
    for x in 0..width {
        // Map pixel to value range (handles len != width)
        let start = x * len / width;
        let end = ((x + 1) * len / width).max(start + 1).min(len);
        let mut sum_sq = 0.0f64;
        for j in start..end {
            sum_sq += (values[j] as f64) * (values[j] as f64);
        }
        let rms = (sum_sq / (end - start) as f64).sqrt();
        let bar_h = ((rms / max_val as f64) * height as f64).round() as u32;
        for dy in 0..bar_h.min(height) {
            let y = bottom.saturating_sub(1 + dy);
            if y >= y_offset {
                img.put_pixel(x as u32, y, color);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Data loading — PWV7
// ---------------------------------------------------------------------------

fn load_pwv7_waveforms(usb_root: &Path) -> BTreeMap<String, (Vec<u8>, Vec<u8>, Vec<u8>)> {
    let mut result = BTreeMap::new();
    let anlz_map = match load_path_to_anlz_map(usb_root) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("warning: cannot load eDB from {}: {e}", usb_root.display());
            return result;
        }
    };
    for (path_key, anlz_path) in &anlz_map {
        let twoex = resolve_ext(usb_root, anlz_path, "2EX");
        let Ok(bytes) = fs::read(&twoex) else {
            continue;
        };
        let Some(payload) = find_chunk_payload_vec(&bytes, b"PWV7") else {
            continue;
        };
        if payload.len() < 3 {
            continue;
        }
        let (low, mid, high) = split_three(&payload);
        if !low.is_empty() {
            result.insert(path_key.clone(), (low, mid, high));
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Data loading — PWV4
// ---------------------------------------------------------------------------

fn load_pwv4_waveforms(usb_root: &Path) -> BTreeMap<String, [Vec<u8>; 6]> {
    let mut result = BTreeMap::new();
    let anlz_map = match load_path_to_anlz_map(usb_root) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("warning: cannot load eDB from {}: {e}", usb_root.display());
            return result;
        }
    };
    for (path_key, anlz_path) in &anlz_map {
        let ext_path = resolve_ext(usb_root, anlz_path, "EXT");
        let Ok(bytes) = fs::read(&ext_path) else {
            continue;
        };
        let Some(payload) = find_chunk_payload_vec(&bytes, b"PWV4") else {
            continue;
        };
        if payload.len() < 6 {
            continue;
        }
        let lanes = split_six(&payload);
        if !lanes[0].is_empty() {
            result.insert(path_key.clone(), lanes);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// eDB + ANLZ helpers
// ---------------------------------------------------------------------------

fn load_path_to_anlz_map(usb_root: &Path) -> Result<BTreeMap<String, String>, String> {
    let db = usb_root
        .join("PIONEER")
        .join("rekordbox")
        .join("exportLibrary.db");
    let conn = open_with_known_keys(&db)?;
    let mut stmt = conn
        .prepare("SELECT path, analysisDataFilePath FROM content WHERE path IS NOT NULL AND TRIM(path) != ''")
        .map_err(|e| format!("prepare: {e}"))?;
    let mut rows = stmt.query([]).map_err(|e| format!("query: {e}"))?;
    let mut out = BTreeMap::new();
    while let Ok(Some(row)) = rows.next() {
        let path = render_value_ref(row.get_ref(0).map_err(|e| e.to_string())?);
        let anlz = render_value_ref(row.get_ref(1).map_err(|e| e.to_string())?);
        if path.is_empty() || anlz.is_empty() {
            continue;
        }
        out.insert(path.to_ascii_lowercase(), anlz);
    }
    Ok(out)
}

fn open_with_known_keys(path: &Path) -> Result<Connection, String> {
    let open_plain = || Connection::open(path).map_err(|e| e.to_string());
    let has_schema = |conn: &Connection| {
        conn.query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type IN ('table','view')",
            [],
            |r| r.get::<_, i64>(0),
        )
        .ok()
        .unwrap_or(0)
            > 0
    };
    let plain = open_plain()?;
    if has_schema(&plain) {
        return Ok(plain);
    }
    for key in [DEFAULT_MASTER_KEY, DEFAULT_USB_EXPORT_KEY] {
        let conn = open_plain()?;
        if conn.execute_batch(&format!("PRAGMA key='{key}';")).is_err() {
            continue;
        }
        if has_schema(&conn) {
            return Ok(conn);
        }
    }
    Err(format!("cannot open database {}", path.display()))
}

fn render_value_ref(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(x) => x.to_string(),
        ValueRef::Real(x) => x.to_string(),
        ValueRef::Text(x) => String::from_utf8_lossy(x).to_string(),
        ValueRef::Blob(x) => format!("<blob:{}>", x.len()),
    }
}

fn resolve_ext(usb_root: &Path, analysis_path: &str, ext: &str) -> PathBuf {
    let rel = analysis_path.trim_start_matches('/').replace('\\', "/");
    let base = usb_root.join(rel);
    base.with_extension(ext)
}

fn find_chunk_payload_vec(bytes: &[u8], tag: &[u8; 4]) -> Option<Vec<u8>> {
    let mut offset = 0usize;
    while offset + 12 <= bytes.len() {
        let t = &bytes[offset..offset + 4];
        let header_len = u32::from_be_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let total_len = u32::from_be_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
        ]) as usize;
        if total_len < header_len || total_len < 12 || offset + total_len > bytes.len() {
            offset += 1;
            continue;
        }
        if t == tag {
            return bytes
                .get(offset + header_len..offset + total_len)
                .map(|s| s.to_vec());
        }
        offset += 1;
    }
    None
}

fn split_three(values: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    for chunk in values.chunks(3) {
        if chunk.len() == 3 {
            a.push(chunk[0]);
            b.push(chunk[1]);
            c.push(chunk[2]);
        }
    }
    (a, b, c)
}

fn split_six(values: &[u8]) -> [Vec<u8>; 6] {
    let mut lanes: [Vec<u8>; 6] = Default::default();
    for chunk in values.chunks(6) {
        if chunk.len() == 6 {
            for (i, &v) in chunk.iter().enumerate() {
                lanes[i].push(v);
            }
        }
    }
    lanes
}

fn slugify(path: &str) -> String {
    path.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}
