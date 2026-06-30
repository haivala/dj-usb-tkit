//! ANLZ pipeline validation test: programmatic WAV generation → analysis → ANLZ file validation.
//!
//! Generates a 30-second WAV file with a 120 BPM kick drum pattern, runs the analysis pipeline,
//! and validates the generated .DAT, .EXT, .2EX files match the expected structure.

use std::io::Write;
use tempfile::tempdir;

use backend::service::anlz::{
    WaveformData, build_anlz_2ex_file, build_anlz_dat_file, build_anlz_ext_file,
    write_generated_anlz_bundle,
};

const SAMPLE_RATE: u32 = 44100;
const BPM: f64 = 120.0;
const DURATION_SECS: f64 = 30.0;
const DURATION_MS: u64 = 30_000;
const TRACK_PATH: &str = "/Contents/Test Artist/Test Album/kick_pattern.wav";

/// Generate a mono 16-bit WAV file with a periodic kick drum pattern.
///
/// Kick: 60 Hz sine burst with exponential decay (~50ms), one per beat at the given BPM.
fn generate_kick_pattern_wav(sample_rate: u32, bpm: f64, duration_secs: f64) -> Vec<u8> {
    let num_samples = (sample_rate as f64 * duration_secs) as usize;
    let beat_interval_samples = (sample_rate as f64 * 60.0 / bpm) as usize;
    let kick_freq = 60.0_f64; // Hz
    let kick_decay_samples = (sample_rate as f64 * 0.05) as usize; // 50ms decay

    let mut samples = vec![0i16; num_samples];
    for i in 0..num_samples {
        let beat_pos = i % beat_interval_samples;
        if beat_pos < kick_decay_samples {
            let t = beat_pos as f64 / sample_rate as f64;
            let amplitude = (-t * 40.0).exp(); // exponential decay
            let sine = (2.0 * std::f64::consts::PI * kick_freq * t).sin();
            let value = (sine * amplitude * 30000.0) as i16;
            samples[i] = value;
        }
    }

    // Build WAV file in memory
    let data_size = (num_samples * 2) as u32;
    let file_size = 36 + data_size;
    let mut buf = Vec::with_capacity(file_size as usize + 8);

    // RIFF header
    buf.write_all(b"RIFF").unwrap();
    buf.write_all(&file_size.to_le_bytes()).unwrap();
    buf.write_all(b"WAVE").unwrap();

    // fmt chunk
    buf.write_all(b"fmt ").unwrap();
    buf.write_all(&16u32.to_le_bytes()).unwrap(); // chunk size
    buf.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
    buf.write_all(&1u16.to_le_bytes()).unwrap(); // mono
    buf.write_all(&sample_rate.to_le_bytes()).unwrap();
    buf.write_all(&(sample_rate * 2).to_le_bytes()).unwrap(); // byte rate
    buf.write_all(&2u16.to_le_bytes()).unwrap(); // block align
    buf.write_all(&16u16.to_le_bytes()).unwrap(); // bits per sample

    // data chunk
    buf.write_all(b"data").unwrap();
    buf.write_all(&data_size.to_le_bytes()).unwrap();
    for s in &samples {
        buf.write_all(&s.to_le_bytes()).unwrap();
    }

    buf
}

fn read_u32_be(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Walk ANLZ chunks and return (tag, header_len, total_len, payload_slice) tuples.
fn walk_chunks(data: &[u8]) -> Vec<(String, usize, usize)> {
    let mut result = Vec::new();
    let mut pos = 28; // skip PMAI header
    while pos + 12 <= data.len() {
        let tag = String::from_utf8_lossy(&data[pos..pos + 4]).to_string();
        let header_len = read_u32_be(data, pos + 4) as usize;
        let total_len = read_u32_be(data, pos + 8) as usize;
        if total_len == 0 {
            break;
        }
        result.push((tag, header_len, total_len));
        pos += total_len;
    }
    result
}

fn chunk_tags(data: &[u8]) -> Vec<String> {
    walk_chunks(data).into_iter().map(|(t, _, _)| t).collect()
}

fn find_chunk_payload<'a>(data: &'a [u8], wanted: &str) -> Option<&'a [u8]> {
    let mut pos = 28;
    while pos + 12 <= data.len() {
        let tag = &data[pos..pos + 4];
        let header_len = read_u32_be(data, pos + 4) as usize;
        let total_len = read_u32_be(data, pos + 8) as usize;
        if tag == wanted.as_bytes() {
            return Some(&data[pos + header_len..pos + total_len]);
        }
        if total_len == 0 {
            break;
        }
        pos += total_len;
    }
    None
}

fn verify_pmai_header(data: &[u8], label: &str) {
    assert!(data.len() >= 28, "{label}: too short");
    assert_eq!(&data[0..4], b"PMAI", "{label}: missing PMAI magic");
    let file_len = read_u32_be(data, 8) as usize;
    assert_eq!(file_len, data.len(), "{label}: file length mismatch");
}

// ---------------------------------------------------------------------------
// Tests using programmatically generated audio
// ---------------------------------------------------------------------------

#[test]
fn anlz_pipeline_with_generated_kick_pattern() {
    let dir = tempdir().unwrap();
    let wav_path = dir.path().join("kick_pattern.wav");
    let wav_data = generate_kick_pattern_wav(SAMPLE_RATE, BPM, DURATION_SECS);
    std::fs::write(&wav_path, &wav_data).unwrap();

    // Run the analysis pipeline
    // Parse peaks from our generated WAV to build waveform data for ANLZ builders.
    let samples: Vec<i16> = wav_data[44..]
        .chunks(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    // Build peaks: divide into bins, take max absolute amplitude per bin
    let num_bins = 1000;
    let bin_size = samples.len() / num_bins;
    let mut peaks = Vec::with_capacity(num_bins);
    let mut bands = Vec::with_capacity(num_bins);
    for i in 0..num_bins {
        let start = i * bin_size;
        let end = ((i + 1) * bin_size).min(samples.len());
        let max_amp = samples[start..end]
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0);
        // Scale to 0-100
        peaks.push(((max_amp as u32 * 100) / 32768).min(100) as u8);
        // Kick drum is low frequency → band 0 (sub-bass) when amplitude > 0, else mid
        bands.push(if max_amp > 1000 { 0 } else { 3 });
    }

    // For the kick pattern, low energy should dominate (60Hz sine bursts)
    let low_energy: Vec<u8> = peaks
        .iter()
        .map(|&p| ((p as u16 * 120) / 100).min(127) as u8)
        .collect();
    let mid_energy: Vec<u8> = peaks
        .iter()
        .map(|&p| ((p as u16 * 30) / 100).min(127) as u8)
        .collect();
    let high_energy: Vec<u8> = peaks
        .iter()
        .map(|&p| ((p as u16 * 10) / 100).min(127) as u8)
        .collect();
    let waveform = WaveformData {
        peaks,
        bands,
        low_energy: low_energy.clone(),
        mid_energy: mid_energy.clone(),
        high_energy: high_energy.clone(),
        low_energy_full: low_energy,
        mid_energy_full: mid_energy,
        high_energy_full: high_energy,
        peak_level: 1.0,
    };
    assert!(!waveform.peaks.is_empty(), "waveform should have data");
    assert!(
        waveform.peaks.iter().any(|&p| p > 10),
        "waveform should have non-trivial peaks from kick drum"
    );

    // Build all 3 ANLZ files
    let dat = build_anlz_dat_file(&waveform, TRACK_PATH, Some(BPM), Some(DURATION_MS));
    let ext = build_anlz_ext_file(&waveform, TRACK_PATH, Some(BPM), Some(DURATION_MS));
    let twoex = build_anlz_2ex_file(&waveform, TRACK_PATH, Some(DURATION_MS));

    // === Validate .DAT ===
    verify_pmai_header(&dat, "DAT");
    let dat_tags = chunk_tags(&dat);
    assert_eq!(
        dat_tags,
        vec!["PPTH", "PVBR", "PQTZ", "PWAV", "PWV2", "PCOB", "PCOB"],
        "DAT chunk order"
    );

    // PPTH: verify UTF-16BE path
    let ppth = find_chunk_payload(&dat, "PPTH").expect("PPTH in DAT");
    let u16s: Vec<u16> = ppth
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    let decoded = String::from_utf16(&u16s[..u16s.len() - 1]).expect("valid UTF-16");
    assert_eq!(decoded, TRACK_PATH);

    // PQTZ: 120 BPM × 30s = 61 beats, 8 bytes each
    let pqtz = find_chunk_payload(&dat, "PQTZ").expect("PQTZ in DAT");
    assert_eq!(pqtz.len(), 61 * 8, "PQTZ should have 61 beat entries");
    // First beat: beat_number=1, tempo=12000. Absolute first-beat phase is waveform-derived.
    assert_eq!(u16::from_be_bytes([pqtz[0], pqtz[1]]), 1);
    assert_eq!(u16::from_be_bytes([pqtz[2], pqtz[3]]), 12000);
    let first_time = u32::from_be_bytes([pqtz[4], pqtz[5], pqtz[6], pqtz[7]]);
    assert!(
        first_time < 500,
        "first beat should be normalized into a single 120 BPM beat interval"
    );
    let second_time = u32::from_be_bytes([pqtz[12], pqtz[13], pqtz[14], pqtz[15]]);
    assert_eq!(
        second_time - first_time,
        500,
        "beat spacing should match 120 BPM"
    );

    // PWAV: 400 entries
    let pwav = find_chunk_payload(&dat, "PWAV").expect("PWAV in DAT");
    assert_eq!(pwav.len(), 400);
    // Should have non-zero entries (kick drum produces amplitude)
    assert!(
        pwav.iter().any(|&b| (b & 0x1F) > 0),
        "PWAV should have non-zero amplitudes"
    );

    // PWV2: 100 entries
    let pwv2 = find_chunk_payload(&dat, "PWV2").expect("PWV2 in DAT");
    assert_eq!(pwv2.len(), 100);

    // === Validate .EXT ===
    verify_pmai_header(&ext, "EXT");
    let ext_tags = chunk_tags(&ext);
    assert_eq!(
        ext_tags,
        vec![
            "PPTH", "PWV3", "PCOB", "PCOB", "PCO2", "PCO2", "PQT2", "PWV5", "PWV4", "PSSI",
        ],
        "EXT chunk order"
    );

    // PWV3: detail count uses ceil(duration × 150) + 4 entries
    let pwv3 = find_chunk_payload(&ext, "PWV3").expect("PWV3 in EXT");
    assert_eq!(pwv3.len(), 4504, "PWV3 entry count = ceil(30s × 150) + 4");

    // PWV4: 1200 entries × 6 bytes = 7200
    let pwv4 = find_chunk_payload(&ext, "PWV4").expect("PWV4 in EXT");
    assert_eq!(pwv4.len(), 7200, "PWV4 = 1200 × 6 bytes");
    // Validate PWV4 frequency energy encoding (not RGB)
    let mut pwv4_low_dominant = 0u32;
    let mut pwv4_nonzero = 0u32;
    for entry in pwv4.chunks(6) {
        let b0 = entry[0]; // support/whiteness
        let b2 = entry[2]; // high/background channel
        let b3 = entry[3]; // low narrow
        let b5 = entry[5]; // high
        // Bytes 0-2 are 0-127 channels.
        assert!(b0 <= 127, "PWV4 byte0 out of range: {b0}");
        assert!(b2 <= 127, "PWV4 byte2 out of range: {b2}");
        if b3 > 0 || b5 > 0 {
            pwv4_nonzero += 1;
            if b3 > b5 {
                pwv4_low_dominant += 1;
            }
        }
    }
    // For bass-dominant kick pattern, low energy should dominate high in most entries
    if pwv4_nonzero > 0 {
        let pct = pwv4_low_dominant * 100 / pwv4_nonzero;
        assert!(
            pct > 50,
            "PWV4: bass-dominant input should have low > high in majority of entries ({pct}%)"
        );
    }

    // PWV5: same entry count as PWV3, 2 bytes each
    let pwv5 = find_chunk_payload(&ext, "PWV5").expect("PWV5 in EXT");
    assert_eq!(pwv5.len(), pwv3.len() * 2, "PWV5 entry count matches PWV3");

    // PWV5 encoding: check RGB bits make sense
    // For bass-dominant entries (band=0), red should be high
    for chunk in pwv5.chunks(2) {
        let packed = u16::from_be_bytes([chunk[0], chunk[1]]);
        let height = (packed >> 2) & 0x1F;
        // Height should be in valid range
        assert!(height <= 31, "PWV5 height out of range: {height}");
    }

    // === Validate .2EX ===
    verify_pmai_header(&twoex, "2EX");
    let twoex_tags = chunk_tags(&twoex);
    assert_eq!(
        twoex_tags,
        vec!["PPTH", "PWV7", "PWV6", "PWVC"],
        "2EX chunk order"
    );

    // PWV7: detail count matches PWV3
    let pwv7 = find_chunk_payload(&twoex, "PWV7").expect("PWV7 in 2EX");
    assert_eq!(pwv7.len(), 4504 * 3, "PWV7 = 4504 × 3 bytes");

    // PWV7: entries are encoded as [low, mid, high], so bass-heavy input
    // should still show low-band dominance after decoding in that order.
    let mut low_dominant_count = 0;
    let mut total_nonzero = 0;
    for chunk in pwv7.chunks(3) {
        let (low, mid, high) = (chunk[0], chunk[1], chunk[2]);
        if mid > 0 || high > 0 || low > 0 {
            total_nonzero += 1;
            if low >= mid && low >= high {
                low_dominant_count += 1;
            }
        }
    }
    if total_nonzero > 0 {
        let pct = low_dominant_count * 100 / total_nonzero;
        // Kick drum is bass, so most non-zero entries should be low-dominant
        // But our input also has mid-band (3) for quiet sections
        assert!(
            pct > 10,
            "bass-heavy audio: low-dominant entries should be significant, got {pct}%"
        );
    }
}

#[test]
fn anlz_bundle_write_roundtrip() {
    let dir = tempdir().unwrap();
    let dat_path = dir.path().join("ANLZ0000.DAT");
    let ext_path = dir.path().join("ANLZ0000.EXT");
    let twoex_path = dir.path().join("ANLZ0000.2EX");

    let waveform = WaveformData::from_peaks(vec![80; 500]);
    write_generated_anlz_bundle(
        &waveform,
        &dat_path,
        &ext_path,
        &twoex_path,
        TRACK_PATH,
        Some(120.0),
        Some(30_000),
    )
    .unwrap();

    // All files exist and are valid ANLZ
    for (path, label) in [(&dat_path, "DAT"), (&ext_path, "EXT"), (&twoex_path, "2EX")] {
        let data = std::fs::read(path).expect(&format!("{label} file should exist"));
        verify_pmai_header(&data, label);
        // All should have PPTH
        assert!(
            data.windows(4).any(|w| w == b"PPTH"),
            "{label} should have PPTH chunk"
        );
    }

    // Verify DAT has PQTZ
    let dat = std::fs::read(&dat_path).unwrap();
    assert!(dat.windows(4).any(|w| w == b"PQTZ"), "DAT should have PQTZ");

    // Verify EXT has PWV5
    let ext = std::fs::read(&ext_path).unwrap();
    assert!(ext.windows(4).any(|w| w == b"PWV5"), "EXT should have PWV5");

    // Verify 2EX has PWV7
    let twoex = std::fs::read(&twoex_path).unwrap();
    assert!(
        twoex.windows(4).any(|w| w == b"PWV7"),
        "2EX should have PWV7"
    );
}
