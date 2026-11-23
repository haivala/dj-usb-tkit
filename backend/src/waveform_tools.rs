//! Helpers the waveform dev tools share (`waveform_render_gate`,
//! `render_waveform_pair`, `render_3way_waveform`, `compare_anlz_beatgrids`):
//! finding a stick's analysis bundles, reading their waveform chunks through
//! the app's ANLZ reader, comparing two waveforms and drawing overlays.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::{Rgb, RgbImage};
use rusqlite::types::ValueRef;

pub fn load_path_to_anlz_map(usb_root: &Path) -> Result<BTreeMap<String, String>, String> {
    let conn = crate::edb::open_rekordbox_db_for_tools(
        &crate::service::usb::vendor_compat::vendor_edb_path(usb_root),
    )?;
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

pub fn render_value_ref(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(x) => x.to_string(),
        ValueRef::Real(x) => x.to_string(),
        ValueRef::Text(x) => String::from_utf8_lossy(x).to_string(),
        ValueRef::Blob(x) => format!("<blob:{}>", x.len()),
    }
}

pub fn resolve_ext(usb_root: &Path, analysis_path: &str, ext: &str) -> PathBuf {
    let rel = analysis_path.trim_start_matches('/').replace('\\', "/");
    let base = usb_root.join(rel);
    base.with_extension(ext)
}

/// The payload of the first `tag` chunk in an ANLZ file, found by walking
/// its chunks as the app does.
pub fn find_chunk_payload<'a>(bytes: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    crate::service::anlz::anlz_chunk_payload(bytes, tag)
}

pub fn extract_pwav_levels(dat_bytes: &[u8]) -> Option<Vec<u8>> {
    let payload = find_chunk_payload(dat_bytes, b"PWAV")?;
    Some(payload.iter().map(|b| b & 0x1F).collect())
}

pub fn extract_pwv6_levels(twoex_bytes: &[u8]) -> Option<Vec<u8>> {
    let payload = find_chunk_payload(twoex_bytes, b"PWV6")?;
    if payload.len() < 3 {
        return None;
    }
    Some(payload.to_vec())
}

pub fn extract_pwv4_levels(ext_bytes: &[u8]) -> Option<Vec<u8>> {
    let payload = find_chunk_payload(ext_bytes, b"PWV4")?;
    if payload.len() < 6 {
        return None;
    }
    Some(payload.to_vec())
}

pub fn split_three(values: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
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

/// (b0, b1, b2, b3, b4, b5) waveform band data.
pub type SixBands = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

pub fn split_six(values: &[u8]) -> SixBands {
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

pub fn to_f64(values: &[u8]) -> Vec<f64> {
    values.iter().map(|&v| v as f64).collect()
}

pub fn pearson_corr(a: &[f64], b: &[f64]) -> f64 {
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

pub fn mae_norm(a: &[f64], b: &[f64], denom: f64) -> f64 {
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

pub fn render_mono_overlay(path: &Path, ref_levels: &[u8], test_levels: &[u8]) {
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

pub fn render_three_overlay(
    path: &Path,
    ref_ch: (&[u8], &[u8], &[u8]),
    tst_ch: (&[u8], &[u8], &[u8]),
) {
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

pub fn render_pwv4_overlay(path: &Path, ref_values: &[u8], tst_values: &[u8]) {
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

pub fn render_missing(path: &Path) {
    let img = RgbImage::from_pixel(64, 32, Rgb([120, 20, 20]));
    let _ = img.save(path);
}

pub fn draw_channel(img: &mut RgbImage, levels: &[u8], max_level: f32, color: Rgb<u8>) {
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

pub fn slugify(path: &str) -> String {
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
