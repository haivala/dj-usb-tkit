//! The USB root: finding it from a requested folder, resolving the paths
//! the stick's databases store against it, and whether it can be written.

use super::vendor_compat::{
    USB_CONTENTS_DIR, USB_ROOT_ENV_KEY, USB_VENDOR_DB_DIR, USB_VENDOR_DB_DIR_LOWER,
    USB_VENDOR_ROOT_DIR, USB_VENDOR_ROOT_DIR_LOWER, USB_VENDOR_ROOT_PREFIX,
};
use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use std::path::{Path, PathBuf};

pub(crate) fn canonicalize_or_self(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or_else(|_| normalize_path_components(&path))
}

/// Resolve `.` and `..` components without touching the filesystem.
/// Used as fallback when `std::fs::canonicalize` fails (path doesn't exist).
fn normalize_path_components(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

pub(crate) fn normalize_usb_root_path(path: PathBuf) -> PathBuf {
    let lower_name = |p: &Path| {
        p.file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default()
    };

    let name = lower_name(&path);
    if (name == USB_CONTENTS_DIR.to_ascii_lowercase() || name == USB_VENDOR_ROOT_DIR_LOWER)
        && let Some(parent) = path.parent()
    {
        return parent.to_path_buf();
    }

    if name == USB_VENDOR_DB_DIR_LOWER
        && let Some(parent) = path.parent()
        && lower_name(parent) == USB_VENDOR_ROOT_DIR_LOWER
        && let Some(root) = parent.parent()
    {
        return root.to_path_buf();
    }

    path
}

pub(crate) fn resolve_usb_root(requested_root: Option<&str>) -> BackendResult<PathBuf> {
    if let Some(requested_root) = requested_root {
        let trimmed = requested_root.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.exists() {
                return Ok(normalize_usb_root_path(canonicalize_or_self(candidate)));
            }
            if candidate.is_relative() {
                let cwd = std::env::current_dir()?;
                let relative_candidates = [
                    cwd.join(trimmed),
                    cwd.join("..").join(trimmed),
                    cwd.join("../..").join(trimmed),
                    cwd.join("../../..").join(trimmed),
                ];
                for alt in relative_candidates {
                    if alt.exists() {
                        return Ok(normalize_usb_root_path(canonicalize_or_self(alt)));
                    }
                }
            }
            return Err(BackendError::NotFound(tr::error_usb_root_missing(
                candidate.display(),
            )));
        }
    }

    if let Ok(override_path) = std::env::var(USB_ROOT_ENV_KEY) {
        let trimmed = override_path.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.exists() {
                return Ok(normalize_usb_root_path(canonicalize_or_self(candidate)));
            }
            return Err(BackendError::Internal(format!(
                "{USB_ROOT_ENV_KEY} is set but path does not exist: {}",
                candidate.display()
            )));
        }
    }

    let cwd = std::env::current_dir()?;
    let candidates = [
        cwd.join("USB"),
        cwd.join("../USB"),
        cwd.join("../../USB"),
        cwd.join("../../../USB"),
    ];
    for candidate in candidates {
        if candidate.exists() {
            return Ok(normalize_usb_root_path(canonicalize_or_self(candidate)));
        }
    }

    Err(BackendError::Internal(
        "USB root not found; set usbRoot in request, DJUSBTKIT_USB_ROOT, or create project USB directory".to_string(),
    ))
}

/// The file a path from the USB's databases names: `/Contents/…` and
/// `/PIONEER/…` (or bare relative paths) resolve under `usb_root` and must stay
/// inside it; absolute paths, `file://` URLs and Windows drive paths pass
/// through. `None` for an empty path or one that escapes `usb_root`.
pub(crate) fn resolve_usb_side_path(usb_root: &Path, raw: &str) -> Option<PathBuf> {
    resolve_usb_side_path_with(usb_root, raw, canonicalize_or_self)
}

/// [`resolve_usb_side_path`] without touching the filesystem: `.`/`..` are
/// resolved by name only, symlinks are not followed. For loops over every
/// track that only need the path as a key -- `canonicalize` costs a directory
/// lookup per path component, which on a cold, large stick is a disk seek per
/// track.
pub(crate) fn resolve_usb_side_path_lexical(usb_root: &Path, raw: &str) -> Option<PathBuf> {
    resolve_usb_side_path_with(usb_root, raw, |path| normalize_path_components(&path))
}

fn resolve_usb_side_path_with(
    usb_root: &Path,
    raw: &str,
    normalize: impl Fn(PathBuf) -> PathBuf,
) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.replace('\\', "/");
    if normalized.is_empty() {
        return None;
    }

    if normalized.starts_with("file://") {
        return Some(PathBuf::from(normalized));
    }

    let is_windows_abs = normalized.len() > 2
        && normalized.as_bytes().get(1) == Some(&b':')
        && normalized.as_bytes().get(2) == Some(&b'/');
    if is_windows_abs {
        return Some(PathBuf::from(normalized));
    }

    // Resolve relative paths under usb_root, then verify containment
    let canon_root = normalize(usb_root.to_path_buf());

    let looks_usb_relative = normalized.starts_with(USB_VENDOR_ROOT_PREFIX)
        || normalized.starts_with(format!("/{USB_CONTENTS_DIR}/").as_str())
        || normalized.starts_with(format!("{USB_VENDOR_ROOT_DIR}/").as_str())
        || normalized.starts_with(format!("{USB_CONTENTS_DIR}/").as_str());

    if looks_usb_relative {
        let rel = normalized.trim_start_matches('/');
        let resolved = normalize(usb_root.join(rel));
        if !resolved.starts_with(&canon_root) {
            return None; // path traversal attempt
        }
        return Some(resolved);
    }

    let normalized = PathBuf::from(normalized);
    if normalized.is_absolute() {
        return Some(normalize(normalized));
    }

    let resolved = normalize(usb_root.join(&normalized));
    if !resolved.starts_with(&canon_root) {
        return None; // path traversal attempt
    }
    Some(resolved)
}

/// [`resolve_usb_side_path`] as text, for API fields and DB keys.
pub(crate) fn resolve_usb_side_path_text(usb_root: &Path, raw: &str) -> Option<String> {
    resolve_usb_side_path(usb_root, raw).map(|path| path.to_string_lossy().into_owned())
}

/// [`resolve_usb_side_path_lexical`] as text: for loops over every track of a
/// stick that build display paths and lookup keys. `usb_root` comes
/// canonical from `resolve_usb_root` and player sticks hold no symlinks, so
/// an existing file gets the same path as [`resolve_usb_side_path_text`].
pub(crate) fn resolve_usb_side_path_lexical_text(usb_root: &Path, raw: &str) -> Option<String> {
    resolve_usb_side_path_lexical(usb_root, raw).map(|path| path.to_string_lossy().into_owned())
}

/// Whether the USB looks writable, without writing to it: for what the UI
/// shows when a USB is selected or a page loads, where a write probe on slow
/// media would cost too much. A command that writes checks with
/// `can_write_file_at` instead, right before it writes.
pub(crate) fn has_write_access(root: &Path) -> bool {
    let probe_dir = root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
    let target = if probe_dir.is_dir() {
        probe_dir
    } else {
        root.to_path_buf()
    };
    match std::fs::metadata(&target) {
        Ok(metadata) => folder_looks_writable(&target, &metadata),
        Err(_) => false,
    }
}

/// A read-only mount keeps its folders' normal permission bits, so it's asked
/// of the filesystem (`statvfs`), which answers without touching the drive.
#[cfg(unix)]
fn folder_looks_writable(path: &Path, metadata: &std::fs::Metadata) -> bool {
    !metadata.permissions().readonly() && !mount_is_read_only(path)
}

/// A Windows folder's read-only attribute doesn't stop writes to it, so only
/// a folder that can't be read at all counts.
#[cfg(not(unix))]
fn folder_looks_writable(_path: &Path, _metadata: &std::fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn mount_is_read_only(path: &Path) -> bool {
    statvfs(path).is_some_and(|stats| stats.f_flag & libc::ST_RDONLY != 0)
}

/// `statvfs` for `path`: the filesystem's flags and size, answered without
/// touching the drive.
#[cfg(unix)]
pub(crate) fn statvfs(path: &Path) -> Option<libc::statvfs> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is NUL-terminated and outlives the call, and `stats`
    // is only read after `statvfs` reports it filled in.
    if unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    Some(unsafe { stats.assume_init() })
}

/// Whether a file can really be written at `root`. Unlike `has_write_access`
/// this sees a read-only mount, whose folders keep their normal permission
/// bits, and ignores a Windows folder's read-only attribute, which doesn't stop
/// writes. It writes to the USB, so only for a command that's about to anyway.
pub(crate) fn can_write_file_at(root: &Path) -> bool {
    let probe = root.join(".dj-usb-tkit-write-probe");
    let written = std::fs::write(&probe, b"").is_ok();
    if written {
        let _ = std::fs::remove_file(&probe);
    }
    written
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::service::usb::vendor_compat::{
        USB_CONTENTS_DIR, USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR,
    };
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    #[test]
    fn normalize_usb_root_path_from_subfolders() {
        let root = PathBuf::from("/tmp/USB");
        assert_eq!(normalize_usb_root_path(root.join(USB_CONTENTS_DIR)), root);
        assert_eq!(
            normalize_usb_root_path(root.join(USB_VENDOR_ROOT_DIR)),
            root
        );
        assert_eq!(
            normalize_usb_root_path(root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)),
            root
        );
    }

    /// Runs `check` on a folder whose permissions say read-only, then makes it
    /// writable again so the temp dir can be removed.
    pub(in crate::service::usb) fn with_read_only_folder(check: impl FnOnce(&Path)) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut perms = std::fs::metadata(dir.path())
            .expect("metadata")
            .permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(dir.path(), perms.clone()).expect("set read-only");
        check(dir.path());
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(dir.path(), perms).expect("restore permissions");
    }

    #[test]
    fn has_write_access_looks_without_writing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(has_write_access(dir.path()));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(!has_write_access(&dir.path().join("missing")));
        // A Windows folder's read-only attribute doesn't stop writes, so only
        // Unix permission bits count.
        with_read_only_folder(|path| assert_eq!(has_write_access(path), cfg!(windows)));
    }

    #[test]
    fn resolve_usb_side_path_allows_valid_vendor_relative() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_resolve_{}__", Uuid::now_v7()));
        let vendor_root = tmp.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR);
        std::fs::create_dir_all(&vendor_root).unwrap();
        std::fs::write(vendor_root.join("test.db"), b"data").unwrap();

        let vendor_rel = format!("/{USB_VENDOR_ROOT_DIR}/{USB_VENDOR_DB_DIR}/test.db");
        let result = resolve_usb_side_path(&tmp, &vendor_rel);
        assert!(
            result.is_some(),
            "should resolve valid vendor-relative path"
        );
        let resolved = result.unwrap().to_string_lossy().into_owned();
        assert!(
            resolved.contains(USB_VENDOR_ROOT_DIR),
            "resolved path should contain vendor root segment: {resolved}"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_usb_side_path_allows_valid_contents_relative() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_resolve_c_{}__", Uuid::now_v7()));
        let contents = tmp.join(USB_CONTENTS_DIR).join("Artist");
        std::fs::create_dir_all(&contents).unwrap();
        std::fs::write(contents.join("track.mp3"), b"audio").unwrap();

        let result = resolve_usb_side_path(&tmp, "/Contents/Artist/track.mp3");
        assert!(
            result.is_some(),
            "should resolve valid Contents-relative path"
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_usb_side_path_rejects_traversal_outside_root() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_traversal_{}__", Uuid::now_v7()));
        let vendor_root = tmp.join(USB_VENDOR_ROOT_DIR);
        std::fs::create_dir_all(&vendor_root).unwrap();

        // Attempt to escape via ../
        let traversal = format!("{USB_VENDOR_ROOT_DIR}/../../etc/passwd");
        let result = resolve_usb_side_path(&tmp, &traversal);
        // Should be None (blocked by containment check) since resolved path escapes root
        assert!(
            result.is_none(),
            "path traversal via ../ should be rejected, got: {:?}",
            result
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_usb_side_path_rejects_contents_traversal() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_ct_{}__", Uuid::now_v7()));
        let contents = tmp.join(USB_CONTENTS_DIR);
        std::fs::create_dir_all(&contents).unwrap();

        let result = resolve_usb_side_path(&tmp, "Contents/../../etc/shadow");
        assert!(
            result.is_none(),
            "Contents traversal via ../ should be rejected, got: {:?}",
            result
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_usb_side_path_empty_returns_none() {
        let tmp = std::env::temp_dir();
        assert!(resolve_usb_side_path(&tmp, "").is_none());
        assert!(resolve_usb_side_path(&tmp, "   ").is_none());
    }

    #[test]
    fn resolve_usb_side_path_absolute_outside_root_is_passthrough() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_abs_{}__", Uuid::now_v7()));
        std::fs::create_dir_all(&tmp).unwrap();

        // Absolute paths that don't start with vendor/contents roots are returned as-is
        let result = resolve_usb_side_path(&tmp, "/etc/hostname");
        // This is an absolute path → goes through the is_absolute() branch (passthrough)
        assert!(result.is_some(), "absolute paths are passed through");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn lexical_text_matches_the_canonical_path_of_a_file_on_the_stick() {
        let tmp = tempfile::tempdir().unwrap();
        let root = canonicalize_or_self(tmp.path().to_path_buf());
        let file = root.join("Contents/Artist/Album/track.mp3");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"x").unwrap();

        let raw = "/Contents/Artist/Album/track.mp3";
        assert_eq!(
            resolve_usb_side_path_lexical_text(&root, raw),
            resolve_usb_side_path_text(&root, raw)
        );
        assert!(resolve_usb_side_path_lexical_text(&root, "/Contents/../../x").is_none());
    }

    #[test]
    fn resolve_usb_side_path_non_relative_traversal_blocked() {
        let tmp = std::env::temp_dir().join(format!("__test_usb_nrt_{}__", Uuid::now_v7()));
        let vendor_root = tmp.join(USB_VENDOR_ROOT_DIR);
        std::fs::create_dir_all(&vendor_root).unwrap();

        // Non-vendor/contents relative path with traversal
        let result = resolve_usb_side_path(&tmp, "../../etc/passwd");
        assert!(
            result.is_none(),
            "non-USB-relative traversal should be rejected, got: {:?}",
            result
        );

        std::fs::remove_dir_all(&tmp).ok();
    }
}
