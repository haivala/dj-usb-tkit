//! rekordbox running under Wine: finding its `master.db` inside a Wine prefix
//! and mapping the Windows paths it stores (`C:/Users/...`, `Z:/home/...`) to
//! the Linux paths they name.

use std::path::{Path, PathBuf};

use super::looks_like_windows_absolute_path;
use super::usb_vendor_compat::desktop_master_db_rel_path;

/// The Wine prefix `path` lives in: its nearest ancestor holding
/// `dosdevices/`, where Wine keeps one symlink per drive letter.
pub(crate) fn wine_prefix_of(path: &Path) -> Option<&Path> {
    path.ancestors()
        .skip(1)
        .find(|dir| dir.join("dosdevices").is_dir())
}

/// The Linux path a drive-absolute Windows path names in `prefix`, resolved
/// through the prefix's drive symlink (`dosdevices/c:` -> `../drive_c`,
/// `z:` -> `/`). Only the drive link is resolved, so a file that's missing
/// still maps to where it would be. `None` when `windows_path` has no drive
/// letter or the prefix doesn't map that drive.
pub(crate) fn translate_wine_path(prefix: &Path, windows_path: &Path) -> Option<PathBuf> {
    let raw = windows_path.to_str()?;
    if !looks_like_windows_absolute_path(raw) {
        return None;
    }
    let drive = format!("{}:", raw[..1].to_ascii_lowercase());
    let root = prefix.join("dosdevices").join(drive).canonicalize().ok()?;
    Some(
        raw[3..]
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .fold(root, |path, part| path.join(part)),
    )
}

/// Where rekordbox's `master.db` would be in the usual Wine prefixes:
/// `$WINEPREFIX`, `~/.wine`, and each Bottles bottle (native or Flatpak).
pub(crate) fn wine_rekordbox_db_candidates() -> Vec<PathBuf> {
    let mut prefixes = Vec::new();
    if let Some(prefix) = std::env::var_os("WINEPREFIX").filter(|p| !p.is_empty()) {
        prefixes.push(PathBuf::from(prefix));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        prefixes.push(home.join(".wine"));
        prefixes.extend(subdirs(&home.join(".local/share/bottles/bottles")));
        prefixes.extend(subdirs(
            &home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
        ));
    }
    rekordbox_db_candidates_in(&prefixes)
}

/// `master.db` under each Windows user's roaming AppData in `prefixes`.
fn rekordbox_db_candidates_in(prefixes: &[PathBuf]) -> Vec<PathBuf> {
    prefixes
        .iter()
        .flat_map(|prefix| subdirs(&prefix.join("drive_c/users")))
        .filter(|user| user.file_name() != Some("Public".as_ref()))
        .map(|user| {
            user.join("AppData/Roaming")
                .join(desktop_master_db_rel_path())
        })
        .collect()
}

/// The directories directly under `dir`, sorted; none when it can't be read.
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prefix with `c:` -> `../drive_c` and `z:` -> `/`, like `wineboot`
    /// makes.
    fn fake_prefix(root: &Path) -> PathBuf {
        let prefix = root.join("prefix");
        std::fs::create_dir_all(prefix.join("dosdevices")).expect("dosdevices");
        std::fs::create_dir_all(prefix.join("drive_c")).expect("drive_c");
        std::os::unix::fs::symlink("../drive_c", prefix.join("dosdevices/c:")).expect("c:");
        std::os::unix::fs::symlink("/", prefix.join("dosdevices/z:")).expect("z:");
        prefix
    }

    #[test]
    fn wine_prefix_of_finds_the_ancestor_with_dosdevices() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let prefix = fake_prefix(tmp.path());
        let master = prefix.join("drive_c/users/dj/AppData/Roaming/Pioneer/rekordbox/master.db");
        assert_eq!(wine_prefix_of(&master), Some(prefix.as_path()));
        assert_eq!(
            wine_prefix_of(&tmp.path().join("rekordbox/master.db")),
            None
        );
    }

    #[test]
    fn translate_wine_path_maps_drive_letters_through_dosdevices() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let prefix = fake_prefix(tmp.path());
        let drive_c = prefix.join("drive_c").canonicalize().expect("drive_c");

        assert_eq!(
            translate_wine_path(&prefix, Path::new("C:/Users/dj/Music/a.mp3")),
            Some(drive_c.join("Users/dj/Music/a.mp3"))
        );
        assert_eq!(
            translate_wine_path(&prefix, Path::new("Z:\\home\\dj\\Music\\a.mp3")),
            Some(PathBuf::from("/home/dj/Music/a.mp3"))
        );
        assert_eq!(
            translate_wine_path(&prefix, Path::new("D:/Music/a.mp3")),
            None
        );
        assert_eq!(
            translate_wine_path(&prefix, Path::new("/home/dj/a.mp3")),
            None
        );
    }

    #[test]
    fn rekordbox_db_candidates_in_lists_each_user_but_public() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let prefix = fake_prefix(tmp.path());
        for user in ["dj", "Public"] {
            std::fs::create_dir_all(prefix.join("drive_c/users").join(user)).expect("user");
        }
        assert_eq!(
            rekordbox_db_candidates_in(std::slice::from_ref(&prefix)),
            vec![prefix.join("drive_c/users/dj/AppData/Roaming/Pioneer/rekordbox/master.db")]
        );
    }
}
