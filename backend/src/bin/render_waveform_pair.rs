use std::env;
use std::fs;
use std::path::PathBuf;

use backend::waveform_tools::*;

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
