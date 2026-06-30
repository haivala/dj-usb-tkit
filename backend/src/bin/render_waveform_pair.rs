use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use image::{Rgb, RgbImage};

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        eprintln!("usage: cargo run --bin render_waveform_pair -- <pair_dir> <out_dir>");
        std::process::exit(2);
    }
    let pair_dir = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);
    fs::create_dir_all(&out_dir).expect("create out_dir");

    let mut tracks = fs::read_dir(&pair_dir)
        .expect("read pair_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with("track"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    tracks.sort();

    if tracks.is_empty() {
        eprintln!("no track* directories found in {}", pair_dir.display());
        std::process::exit(1);
    }

    let mut summary = String::from(
        "track\tmono_corr\tmono_mae\tpwv4_corr\tpwv4_mae\tmid_corr\thigh_corr\tlow_corr\tmid_mae\thigh_mae\tlow_mae\tstatus\n",
    );
    let mut failures = 0usize;

    for track_dir in tracks {
        let track_name = track_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("track")
            .to_string();
        let rb = track_dir.join("rb");
        let ours = track_dir.join("ours");
        let rb_dat = rb.join("ANLZ0000.DAT");
        let rb_ext = rb.join("ANLZ0000.EXT");
        let rb_2ex = rb.join("ANLZ0000.2EX");
        let our_dat = ours.join("ANLZ0000.DAT");
        let our_ext = ours.join("ANLZ0000.EXT");
        let our_2ex = ours.join("ANLZ0000.2EX");

        if ![&rb_dat, &rb_ext, &rb_2ex, &our_dat, &our_ext, &our_2ex]
            .iter()
            .all(|p| p.is_file())
        {
            failures += 1;
            render_missing(&out_dir.join(format!("{track_name}.missing.png")));
            summary.push_str(&format!(
                "{track_name}\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\tmissing_files\n"
            ));
            continue;
        }

        let rb_dat_b = fs::read(&rb_dat).expect("read rb dat");
        let rb_ext_b = fs::read(&rb_ext).expect("read rb ext");
        let rb_2ex_b = fs::read(&rb_2ex).expect("read rb 2ex");
        let our_dat_b = fs::read(&our_dat).expect("read our dat");
        let our_ext_b = fs::read(&our_ext).expect("read our ext");
        let our_2ex_b = fs::read(&our_2ex).expect("read our 2ex");

        let Some(rb_mono) = extract_pwav_levels(&rb_dat_b) else {
            failures += 1;
            summary.push_str(&format!(
                "{track_name}\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\trb_pwav_missing\n"
            ));
            continue;
        };
        let Some(our_mono) = extract_pwav_levels(&our_dat_b) else {
            failures += 1;
            summary.push_str(&format!(
                "{track_name}\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\tour_pwav_missing\n"
            ));
            continue;
        };
        let Some(rb_pwv4) = extract_pwv4_levels(&rb_ext_b) else {
            failures += 1;
            summary.push_str(&format!(
                "{track_name}\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\trb_pwv4_missing\n"
            ));
            continue;
        };
        let Some(our_pwv4) = extract_pwv4_levels(&our_ext_b) else {
            failures += 1;
            summary.push_str(&format!(
                "{track_name}\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\tour_pwv4_missing\n"
            ));
            continue;
        };
        let Some(rb_3) = extract_pwv6_levels(&rb_2ex_b) else {
            failures += 1;
            summary.push_str(&format!(
                "{track_name}\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\trb_pwv6_missing\n"
            ));
            continue;
        };
        let Some(our_3) = extract_pwv6_levels(&our_2ex_b) else {
            failures += 1;
            summary.push_str(&format!(
                "{track_name}\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\tour_pwv6_missing\n"
            ));
            continue;
        };

        let mono_corr = pearson_corr(&to_f64(&rb_mono), &to_f64(&our_mono));
        let mono_mae = mae_norm(&to_f64(&rb_mono), &to_f64(&our_mono), 31.0);

        let (rb_p0, _, _, _, _, _) = split_six(&rb_pwv4);
        let (our_p0, _, _, _, _, _) = split_six(&our_pwv4);
        let pwv4_corr = pearson_corr(&to_f64(&rb_p0), &to_f64(&our_p0));
        let pwv4_mae = mae_norm(&to_f64(&rb_p0), &to_f64(&our_p0), 127.0);

        let (rb_mid, rb_high, rb_low) = split_three(&rb_3);
        let (our_mid, our_high, our_low) = split_three(&our_3);
        let mid_corr = pearson_corr(&to_f64(&rb_mid), &to_f64(&our_mid));
        let high_corr = pearson_corr(&to_f64(&rb_high), &to_f64(&our_high));
        let low_corr = pearson_corr(&to_f64(&rb_low), &to_f64(&our_low));
        let mid_mae = mae_norm(&to_f64(&rb_mid), &to_f64(&our_mid), 127.0);
        let high_mae = mae_norm(&to_f64(&rb_high), &to_f64(&our_high), 127.0);
        let low_mae = mae_norm(&to_f64(&rb_low), &to_f64(&our_low), 127.0);

        render_mono_overlay(
            &out_dir.join(format!("{track_name}.mono.png")),
            &rb_mono,
            &our_mono,
        );
        render_pwv4_overlay(
            &out_dir.join(format!("{track_name}.pwv4.png")),
            &rb_pwv4,
            &our_pwv4,
        );
        render_three_overlay(
            &out_dir.join(format!("{track_name}.3band.png")),
            (&rb_mid, &rb_high, &rb_low),
            (&our_mid, &our_high, &our_low),
        );

        summary.push_str(&format!(
            "{track_name}\t{mono_corr:.4}\t{mono_mae:.4}\t{pwv4_corr:.4}\t{pwv4_mae:.4}\t{mid_corr:.4}\t{high_corr:.4}\t{low_corr:.4}\t{mid_mae:.4}\t{high_mae:.4}\t{low_mae:.4}\tok\n"
        ));
    }

    let summary_path = out_dir.join("summary.tsv");
    fs::write(&summary_path, summary).expect("write summary");
    println!("rendered summary: {}", summary_path.display());
    if failures > 0 {
        std::process::exit(1);
    }
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
            return bytes.get(offset + header_len..offset + total_len);
        }
        offset += 1;
    }
    None
}

fn extract_pwav_levels(dat_bytes: &[u8]) -> Option<Vec<u8>> {
    let payload = find_chunk_payload(dat_bytes, b"PWAV")?;
    Some(payload.iter().map(|b| b & 0x1F).collect())
}

fn extract_pwv4_levels(ext_bytes: &[u8]) -> Option<Vec<u8>> {
    let payload = find_chunk_payload(ext_bytes, b"PWV4")?;
    if payload.len() < 6 {
        return None;
    }
    Some(payload.to_vec())
}

fn extract_pwv6_levels(twoex_bytes: &[u8]) -> Option<Vec<u8>> {
    let payload = find_chunk_payload(twoex_bytes, b"PWV6")?;
    if payload.len() < 3 {
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
    draw_channel(&mut img, ref_ch.2, 127.0, Rgb([50, 120, 255]));
    draw_channel(&mut img, ref_ch.0, 127.0, Rgb([255, 180, 40]));
    draw_channel(&mut img, ref_ch.1, 127.0, Rgb([240, 240, 240]));
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
