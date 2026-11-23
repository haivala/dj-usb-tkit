use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;

use backend::waveform_tools::*;

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
