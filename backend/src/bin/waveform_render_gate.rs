use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use image::{Rgb, RgbImage};
use rusqlite::{Connection, types::ValueRef};

const DEFAULT_USB_EXPORT_KEY: &str =
    "r8gddnr4k847830ar6cqzbkk0el6qytmb3trbbx805jm74vez64i5o8fnrqryqls";
const DEFAULT_MASTER_KEY: &str = "402fd_d44f42a8_eb0f6d4db0e6b";

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 4 {
        eprintln!(
            "usage: cargo run --features dev-tools --bin waveform_render_gate -- <rb_ref_usb> <test_usb> <out_dir>"
        );
        std::process::exit(2);
    }
    let ref_usb = PathBuf::from(&args[1]);
    let test_usb = PathBuf::from(&args[2]);
    let out_dir = PathBuf::from(&args[3]);
    let max_tracks = env::var("WAVEFORM_GATE_MAX_TRACKS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    fs::create_dir_all(&out_dir).expect("create output dir");

    let ref_map = load_path_to_anlz_map(&ref_usb).expect("load reference analysis paths");
    let test_map = load_path_to_anlz_map(&test_usb).expect("load test analysis paths");

    let common = ref_map
        .keys()
        .filter(|k| test_map.contains_key(*k))
        .cloned()
        .collect::<BTreeSet<_>>();
    if common.is_empty() {
        eprintln!("waveform gate: no common content paths between reference and test eDB");
        std::process::exit(1);
    }

    let mut fail_count = 0usize;
    let mut checked = 0usize;
    for path_key in common {
        if max_tracks > 0 && checked >= max_tracks {
            break;
        }
        let ref_anlz = ref_map.get(&path_key).expect("ref anlz path");
        let test_anlz = test_map.get(&path_key).expect("test anlz path");
        let ref_dat = resolve_ext(ref_usb.as_path(), ref_anlz, "DAT");
        let ref_ext = resolve_ext(ref_usb.as_path(), ref_anlz, "EXT");
        let test_dat = resolve_ext(test_usb.as_path(), test_anlz, "DAT");
        let test_ext = resolve_ext(test_usb.as_path(), test_anlz, "EXT");
        let ref_twoex = resolve_ext(ref_usb.as_path(), ref_anlz, "2EX");
        let test_twoex = resolve_ext(test_usb.as_path(), test_anlz, "2EX");
        let track_slug = slugify(&path_key);

        let have_all_files = ref_dat.is_file()
            && ref_ext.is_file()
            && ref_twoex.is_file()
            && test_dat.is_file()
            && test_ext.is_file()
            && test_twoex.is_file();
        if !have_all_files {
            fail_count += 1;
            render_missing(&out_dir.join(format!("{track_slug}.missing.png")));
            eprintln!("waveform gate fail: {path_key} missing DAT/EXT/2EX on one or both sides");
            continue;
        }

        let Ok(ref_dat_bytes) = fs::read(&ref_dat) else {
            fail_count += 1;
            continue;
        };
        let Ok(test_dat_bytes) = fs::read(&test_dat) else {
            fail_count += 1;
            continue;
        };
        let Ok(ref_ext_bytes) = fs::read(&ref_ext) else {
            fail_count += 1;
            continue;
        };
        let Ok(test_ext_bytes) = fs::read(&test_ext) else {
            fail_count += 1;
            continue;
        };
        let Ok(ref_twoex_bytes) = fs::read(&ref_twoex) else {
            fail_count += 1;
            continue;
        };
        let Ok(test_twoex_bytes) = fs::read(&test_twoex) else {
            fail_count += 1;
            continue;
        };

        let Some(ref_mono) = extract_pwav_levels(&ref_dat_bytes) else {
            fail_count += 1;
            continue;
        };
        let Some(test_mono) = extract_pwav_levels(&test_dat_bytes) else {
            fail_count += 1;
            continue;
        };
        let Some(ref_pwv4) = extract_pwv4_levels(&ref_ext_bytes) else {
            fail_count += 1;
            continue;
        };
        let Some(test_pwv4) = extract_pwv4_levels(&test_ext_bytes) else {
            fail_count += 1;
            continue;
        };
        let Some(ref_three) = extract_pwv6_levels(&ref_twoex_bytes) else {
            fail_count += 1;
            continue;
        };
        let Some(test_three) = extract_pwv6_levels(&test_twoex_bytes) else {
            fail_count += 1;
            continue;
        };

        if ref_mono.is_empty()
            || test_mono.is_empty()
            || ref_pwv4.is_empty()
            || test_pwv4.is_empty()
            || ref_three.is_empty()
            || test_three.is_empty()
        {
            fail_count += 1;
            continue;
        }
        checked += 1;

        let mono_corr = pearson_corr(&to_f64(&ref_mono), &to_f64(&test_mono));
        let mono_mae = mae_norm(&to_f64(&ref_mono), &to_f64(&test_mono), 31.0);

        let (ref_m, ref_h, ref_l) = split_three(&ref_three);
        let (tst_m, tst_h, tst_l) = split_three(&test_three);
        let corr_mid = pearson_corr(&to_f64(&ref_m), &to_f64(&tst_m));
        let corr_high = pearson_corr(&to_f64(&ref_h), &to_f64(&tst_h));
        let corr_low = pearson_corr(&to_f64(&ref_l), &to_f64(&tst_l));
        let mae_mid = mae_norm(&to_f64(&ref_m), &to_f64(&tst_m), 127.0);
        let mae_high = mae_norm(&to_f64(&ref_h), &to_f64(&tst_h), 127.0);
        let mae_low = mae_norm(&to_f64(&ref_l), &to_f64(&tst_l), 127.0);

        render_mono_overlay(
            &out_dir.join(format!("{track_slug}.mono.png")),
            &ref_mono,
            &test_mono,
        );
        render_pwv4_overlay(
            &out_dir.join(format!("{track_slug}.pwv4.png")),
            &ref_pwv4,
            &test_pwv4,
        );
        render_three_overlay(
            &out_dir.join(format!("{track_slug}.3band.png")),
            (&ref_m, &ref_h, &ref_l),
            (&tst_m, &tst_h, &tst_l),
        );

        let (ref_b0, _, _, _, _, _) = split_six(&ref_pwv4);
        let (tst_b0, _, _, _, _, _) = split_six(&test_pwv4);
        let pwv4_corr = pearson_corr(&to_f64(&ref_b0), &to_f64(&tst_b0));
        let pwv4_mae = mae_norm(&to_f64(&ref_b0), &to_f64(&tst_b0), 127.0);

        let mono_ok = mono_corr >= 0.80 && mono_mae <= 0.18;
        let pwv4_ok = pwv4_corr >= 0.78 && pwv4_mae <= 0.20;
        let three_ok = corr_mid >= 0.75
            && corr_high >= 0.75
            && corr_low >= 0.75
            && mae_mid <= 0.20
            && mae_high <= 0.20
            && mae_low <= 0.20;
        if !(mono_ok && pwv4_ok && three_ok) {
            fail_count += 1;
            eprintln!(
                "waveform gate fail: {path_key} mono(corr={mono_corr:.3},mae={mono_mae:.3}) pwv4(corr={pwv4_corr:.3},mae={pwv4_mae:.3}) 3band(corr m/h/l={corr_mid:.3}/{corr_high:.3}/{corr_low:.3}, mae m/h/l={mae_mid:.3}/{mae_high:.3}/{mae_low:.3})"
            );
        }
    }

    if checked == 0 {
        eprintln!("waveform gate: no tracks with comparable DAT+2EX payloads");
        std::process::exit(1);
    }
    println!(
        "waveform gate checked tracks={checked} failed={fail_count} out_dir={}",
        out_dir.display()
    );
    if fail_count > 0 {
        std::process::exit(1);
    }
}

fn load_path_to_anlz_map(usb_root: &Path) -> Result<BTreeMap<String, String>, String> {
    let db = usb_root
        .join("PIONEER")
        .join("rekordbox")
        .join("exportLibrary.db");
    let conn = open_with_known_keys(&db)?;
    let mut stmt = conn
        .prepare("SELECT path, analysisDataFilePath FROM content WHERE path IS NOT NULL AND TRIM(path) != ''")
        .map_err(|e| format!("prepare content query: {e}"))?;
    let mut rows = stmt.query([]).map_err(|e| format!("query content: {e}"))?;
    let mut out = BTreeMap::<String, String>::new();
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

fn find_chunk_payload<'a>(bytes: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
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
            let payload_start = offset + header_len;
            let payload_end = offset + total_len;
            return bytes.get(payload_start..payload_end);
        }
        offset += 1;
    }
    None
}

fn extract_pwav_levels(dat_bytes: &[u8]) -> Option<Vec<u8>> {
    let payload = find_chunk_payload(dat_bytes, b"PWAV")?;
    Some(payload.iter().map(|b| b & 0x1F).collect())
}

fn extract_pwv6_levels(twoex_bytes: &[u8]) -> Option<Vec<u8>> {
    let payload = find_chunk_payload(twoex_bytes, b"PWV6")?;
    if payload.len() < 3 {
        return None;
    }
    Some(payload.to_vec())
}

fn extract_pwv4_levels(ext_bytes: &[u8]) -> Option<Vec<u8>> {
    let payload = find_chunk_payload(ext_bytes, b"PWV4")?;
    if payload.len() < 6 {
        return None;
    }
    Some(payload.to_vec())
}

fn split_three(values: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut mid = Vec::<u8>::new();
    let mut high = Vec::<u8>::new();
    let mut low = Vec::<u8>::new();
    for chunk in values.chunks(3) {
        if chunk.len() == 3 {
            mid.push(chunk[0]);
            high.push(chunk[1]);
            low.push(chunk[2]);
        }
    }
    (mid, high, low)
}

fn split_six(values: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut b0 = Vec::<u8>::new();
    let mut b1 = Vec::<u8>::new();
    let mut b2 = Vec::<u8>::new();
    let mut b3 = Vec::<u8>::new();
    let mut b4 = Vec::<u8>::new();
    let mut b5 = Vec::<u8>::new();
    for chunk in values.chunks(6) {
        if chunk.len() == 6 {
            b0.push(chunk[0]);
            b1.push(chunk[1]);
            b2.push(chunk[2]);
            b3.push(chunk[3]);
            b4.push(chunk[4]);
            b5.push(chunk[5]);
        }
    }
    (b0, b1, b2, b3, b4, b5)
}

fn to_f64(values: &[u8]) -> Vec<f64> {
    values.iter().map(|&v| v as f64).collect()
}

fn pearson_corr(a: &[f64], b: &[f64]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mean_a = a.iter().sum::<f64>() / n as f64;
    let mean_b = b.iter().sum::<f64>() / n as f64;
    let mut num = 0.0f64;
    let mut da = 0.0f64;
    let mut db = 0.0f64;
    for i in 0..n {
        let xa = a[i] - mean_a;
        let xb = b[i] - mean_b;
        num += xa * xb;
        da += xa * xa;
        db += xb * xb;
    }
    if da <= f64::EPSILON || db <= f64::EPSILON {
        return 0.0;
    }
    (num / (da.sqrt() * db.sqrt())).clamp(-1.0, 1.0)
}

fn mae_norm(a: &[f64], b: &[f64], denom: f64) -> f64 {
    if a.is_empty() || b.is_empty() || denom <= 0.0 {
        return 1.0;
    }
    let n = a.len().min(b.len());
    let mut sum = 0.0f64;
    for i in 0..n {
        sum += (a[i] - b[i]).abs();
    }
    (sum / n as f64) / denom
}

fn render_mono_overlay(path: &Path, ref_levels: &[u8], test_levels: &[u8]) {
    let width = ref_levels.len().max(test_levels.len()).max(1) as u32;
    let height = 96u32;
    let mut img = RgbImage::from_pixel(width, height, Rgb([14, 14, 18]));
    for (i, &h) in ref_levels.iter().enumerate() {
        let y = (height as i32 - 1 - ((h as f32 / 31.0) * (height as f32 - 1.0)).round() as i32)
            .clamp(0, height as i32 - 1) as u32;
        img.put_pixel(i as u32, y, Rgb([60, 220, 120]));
    }
    for (i, &h) in test_levels.iter().enumerate() {
        let y = (height as i32 - 1 - ((h as f32 / 31.0) * (height as f32 - 1.0)).round() as i32)
            .clamp(0, height as i32 - 1) as u32;
        let p = img.get_pixel_mut(i as u32, y);
        let [r, g, b] = p.0;
        *p = Rgb([
            r.saturating_add(180),
            g.saturating_sub(40),
            b.saturating_add(180),
        ]);
    }
    let _ = img.save(path);
}

fn render_three_overlay(path: &Path, ref_ch: (&[u8], &[u8], &[u8]), tst_ch: (&[u8], &[u8], &[u8])) {
    let width = ref_ch.0.len().max(tst_ch.0.len()).max(1) as u32;
    let height = 192u32;
    let mut img = RgbImage::from_pixel(width, height, Rgb([12, 12, 16]));
    draw_channel(&mut img, ref_ch.2, 127.0, Rgb([50, 120, 255])); // low
    draw_channel(&mut img, ref_ch.0, 127.0, Rgb([255, 180, 40])); // mid
    draw_channel(&mut img, ref_ch.1, 127.0, Rgb([240, 240, 240])); // high
    draw_channel(&mut img, tst_ch.2, 127.0, Rgb([190, 60, 255]));
    draw_channel(&mut img, tst_ch.0, 127.0, Rgb([255, 70, 70]));
    draw_channel(&mut img, tst_ch.1, 127.0, Rgb([120, 255, 120]));
    let _ = img.save(path);
}

fn render_pwv4_overlay(path: &Path, ref_values: &[u8], tst_values: &[u8]) {
    let (ref_b0, _, _, ref_b3, ref_b4, ref_b5) = split_six(ref_values);
    let (tst_b0, _, _, tst_b3, tst_b4, tst_b5) = split_six(tst_values);
    let width = ref_b0.len().max(tst_b0.len()).max(1) as u32;
    let height = 192u32;
    let mut img = RgbImage::from_pixel(width, height, Rgb([10, 10, 14]));
    draw_channel(&mut img, &ref_b0, 127.0, Rgb([230, 230, 230]));
    draw_channel(&mut img, &ref_b3, 127.0, Rgb([80, 130, 255]));
    draw_channel(&mut img, &ref_b4, 127.0, Rgb([255, 180, 40]));
    draw_channel(&mut img, &ref_b5, 127.0, Rgb([245, 245, 245]));
    draw_channel(&mut img, &tst_b0, 127.0, Rgb([255, 80, 80]));
    draw_channel(&mut img, &tst_b3, 127.0, Rgb([180, 60, 255]));
    draw_channel(&mut img, &tst_b4, 127.0, Rgb([100, 255, 100]));
    draw_channel(&mut img, &tst_b5, 127.0, Rgb([255, 120, 255]));
    let _ = img.save(path);
}

fn render_missing(path: &Path) {
    let img = RgbImage::from_pixel(64, 32, Rgb([120, 20, 20]));
    let _ = img.save(path);
}

fn draw_channel(img: &mut RgbImage, levels: &[u8], max_level: f32, color: Rgb<u8>) {
    let h = img.height();
    for (i, &v) in levels.iter().enumerate() {
        if i as u32 >= img.width() {
            break;
        }
        let y = (h as i32 - 1 - ((v as f32 / max_level) * (h as f32 - 1.0)).round() as i32)
            .clamp(0, h as i32 - 1) as u32;
        img.put_pixel(i as u32, y, color);
    }
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
