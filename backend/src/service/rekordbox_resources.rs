//! Locating rekordbox database resources (artwork, analysis files) on disk.

use super::*;

pub(super) fn non_empty_db_value(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub(super) fn looks_like_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

pub(super) fn is_pioneer_virtual_path(value: &str) -> bool {
    value
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
        .starts_with("pioneer/")
}

pub(super) fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|p| p == &path) {
        paths.push(path);
    }
}

pub(super) fn rekordbox_db_resource_candidates(master_path: &Path, db_path: &str) -> Vec<PathBuf> {
    let Some(raw) = non_empty_db_value(db_path) else {
        return Vec::new();
    };

    let master_parent = master_path.parent().unwrap_or(Path::new("."));
    let normalized = raw.replace('\\', "/");
    let relative = normalized.trim_start_matches('/');
    let raw_path = Path::new(raw);
    let mut candidates = Vec::<PathBuf>::new();

    if is_pioneer_virtual_path(raw) {
        // Desktop library stores /PIONEER/... values under <share>/PIONEER/...
        // on Windows. Older layouts may place PIONEER directly beside master.db.
        push_unique_path(&mut candidates, master_parent.join("share").join(relative));
        push_unique_path(&mut candidates, master_parent.join(relative));
    } else {
        if raw_path.is_absolute()
            || looks_like_windows_absolute_path(raw)
            || raw.starts_with("\\\\")
            || raw.starts_with("//")
        {
            push_unique_path(&mut candidates, PathBuf::from(raw));
        }
        push_unique_path(&mut candidates, master_parent.join(relative));
        push_unique_path(&mut candidates, master_parent.join("share").join(relative));
    }

    candidates
}

pub(super) fn resolve_rekordbox_db_resource_path<F>(
    master_path: &Path,
    db_path: &str,
    exists: F,
) -> Option<PathBuf>
where
    F: Fn(&Path) -> bool,
{
    rekordbox_db_resource_candidates(master_path, db_path)
        .into_iter()
        .find(|p| exists(p.as_path()))
}

pub(super) fn rekordbox_db_analysis_file_candidates(
    master_path: &Path,
    db_path: &str,
) -> Vec<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    for base in rekordbox_db_resource_candidates(master_path, db_path) {
        push_unique_path(&mut candidates, base.with_extension("EXT"));
        push_unique_path(&mut candidates, base.with_extension("2EX"));
        push_unique_path(&mut candidates, base.with_extension("DAT"));
        push_unique_path(&mut candidates, base);
    }
    candidates
}

/// The `.DAT` path of a track's bundle in the app's local ANLZ cache, or
/// `None` when `waveform_peaks_path` points anywhere else. A USB-imported
/// track's path can point at a bundle on the stick (possibly rekordbox-made,
/// possibly without `PWV6`/`.2EX`), and cache maintenance must never touch it.
/// Cache files are named by `local_analysis_bundle_paths`' source-path hash,
/// not the track id, so the stored path is the only reliable way to find them.
pub(super) fn local_cache_bundle_dat(
    waveform_dir: &Path,
    waveform_peaks_path: &Path,
) -> Option<PathBuf> {
    (waveform_peaks_path.parent() == Some(waveform_dir)).then(|| waveform_peaks_path.to_path_buf())
}

pub(super) fn remove_bundle_files(dat: &Path) {
    for ext in ["DAT", "EXT", "2EX"] {
        let _ = std::fs::remove_file(dat.with_extension(ext));
    }
}
