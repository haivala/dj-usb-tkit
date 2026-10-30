//! Frontend settings and the external tool checks.

use super::*;

impl BackendService {
    pub fn get_frontend_settings(&self) -> BackendResult<GetFrontendSettingsData> {
        let conn = self.db.connect()?;
        let mut values = std::collections::HashMap::<String, String>::new();
        let keys = frontend_ui_setting_keys();
        for key in keys {
            if let Some(value) = conn
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            {
                values.insert(key.to_string(), value);
            }
        }
        let node_available = check_node_available();
        let essentia_installed = check_essentia_installed(&self.db.data_dir());
        Ok(GetFrontendSettingsData {
            values,
            node_available,
            essentia_installed,
        })
    }

    pub fn remove_essentia(&self) -> BackendResult<()> {
        let essentia_dir = self.db.data_dir().join("essentia");
        if essentia_dir.exists() {
            std::fs::remove_dir_all(&essentia_dir).map_err(|e| {
                BackendError::Internal(format!("failed to remove essentia dir: {e}"))
            })?;
        }
        let conn = self.db.connect()?;
        conn.execute(
            "DELETE FROM app_settings WHERE key = ?1",
            params![SETTING_UI_ANALYSIS_ENGINE],
        )?;
        Ok(())
    }

    pub fn set_frontend_setting(
        &self,
        req: SetFrontendSettingRequest,
    ) -> BackendResult<SetFrontendSettingData> {
        let key = req.key.trim();
        if !frontend_ui_setting_keys().contains(&key) {
            return Err(BackendError::Validation(format!(
                "unsupported frontend setting key: {key}"
            )));
        }
        let conn = self.db.connect()?;
        if let Some(raw_value) = req.value {
            let value = raw_value.trim().to_string();
            if value.len() > 262_144 {
                return Err(BackendError::Validation(
                    "frontend setting value too large".to_string(),
                ));
            }
            conn.execute(
                r#"
                INSERT INTO app_settings (key, value, updated_at)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
                "#,
                params![key, value, now()],
            )?;
        } else {
            conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
        }
        Ok(SetFrontendSettingData { saved: true })
    }
}

pub(super) fn frontend_ui_setting_keys() -> &'static [&'static str] {
    &[
        SETTING_UI_THEME,
        SETTING_UI_ACCENT_HUE,
        SETTING_UI_USB_ROOT,
        SETTING_UI_EXPORT_PRUNE_STALE,
        SETTING_UI_EXPORT_BACKUP,
        SETTING_UI_EXPORT_ENGINE_LIBRARY,
        SETTING_UI_BACKUP_RETENTION_COUNT,
        SETTING_UI_ANALYSIS_BPM_RANGE,
        SETTING_UI_ANALYSIS_ENGINE,
        SETTING_UI_SIDEBAR_COLLAPSED,
        SETTING_UI_SIDEBAR_WIDTH,
        SETTING_UI_HELP_SEEN,
        SETTING_UI_KEY_NOTATION,
        SETTING_UI_CUE_START_ON_FIRST_BEAT,
        SETTING_UI_CUE_BEATGRID_LEVEL,
        SETTING_UI_CUE_QUANTIZE,
        SETTING_UI_CUE_METRONOME_MIX,
        SETTING_UI_CUE_FOLLOW_GRID,
    ]
}

/// The user's configured local library folders (`LibrarySources`). An
/// explicit "this is my library folder" assertion from the user outranks
/// stale USB-device history for that same path (see
/// `usb_utils::all_usb_device_root_paths`).
pub(crate) fn configured_source_roots(conn: &rusqlite::Connection) -> BackendResult<Vec<String>> {
    Ok(LibrarySources::load(conn)?.roots)
}

/// The user's currently-configured export sync mode, read directly from
/// `app_settings` (the same `"1"`/`"0"` string the frontend mirrors into
/// `localStorage` under `SETTING_UI_EXPORT_PRUNE_STALE`). Defaults to `true`
/// (mirror/prune-stale) when unset, matching the frontend's own default.
pub(crate) fn export_prune_stale_setting(conn: &rusqlite::Connection) -> BackendResult<bool> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_EXPORT_PRUNE_STALE],
            |row| row.get(0),
        )
        .optional()?;
    Ok(raw.map(|value| value == "1").unwrap_or(true))
}

/// All known USB device root paths, minus any that are also currently
/// configured as a local `sourceRoots` folder. `all_usb_device_root_paths`
/// intentionally never forgets a path once it's been seen as a USB root
/// (see its doc comment) -- an explicit "this is my library folder"
/// assertion from the user is what overrides that stale taint for reused
/// mount paths (e.g. `/mnt/data` remounted as a permanent local folder),
/// not the passage of time or a prune action.
pub(crate) fn untainted_usb_root_paths(conn: &rusqlite::Connection) -> BackendResult<Vec<String>> {
    let source_roots = configured_source_roots(conn)?;
    let all_roots = usb_utils::all_usb_device_root_paths(conn)?;
    Ok(all_roots
        .into_iter()
        .filter(|root| {
            !source_roots.iter().any(|sr| {
                normalize_source_root_for_matching(sr) == normalize_source_root_for_matching(root)
            })
        })
        .collect())
}

/// Whether a path is on an exported USB stick: under a known device root
/// (`untainted_usb_root_paths`), or in a `Contents` folder next to the
/// vendor folder -- a stick the app never registered, or a copy of one.
/// Paths under a configured source root only take the known-root check:
/// the user said that folder is their library.
pub(crate) struct UsbPathCheck {
    device_roots: Vec<String>,
    source_roots: Vec<String>,
    /// Per `Contents` parent: whether it holds the vendor folder.
    export_roots: std::cell::RefCell<std::collections::HashMap<PathBuf, bool>>,
}

impl UsbPathCheck {
    pub(crate) fn load(conn: &rusqlite::Connection) -> BackendResult<Self> {
        Ok(Self {
            device_roots: untainted_usb_root_paths(conn)?,
            source_roots: configured_source_roots(conn)?,
            export_roots: Default::default(),
        })
    }

    pub(crate) fn is_usb_path(&self, path: &str) -> bool {
        if self
            .device_roots
            .iter()
            .any(|root| browse_path_matches_root(path, root))
        {
            return true;
        }
        if self
            .source_roots
            .iter()
            .any(|root| browse_path_matches_root(path, root))
        {
            return false;
        }
        let mut export_roots = self.export_roots.borrow_mut();
        stored_file_path(path)
            .ancestors()
            .filter(|dir| dir.file_name().is_some_and(|name| name == "Contents"))
            .filter_map(Path::parent)
            .any(|root| {
                *export_roots
                    .entry(root.to_path_buf())
                    .or_insert_with(|| root.join(usb_vendor_compat::USB_VENDOR_ROOT_DIR).is_dir())
            })
    }
}

/// Fills the derived fields every frontend-bound `Track` carries, one query
/// each for the whole batch (not per-row). Call this from any method that
/// returns `Track` rows to the frontend.
///
/// - `is_usb_path` mirrors `resolve_playback_source`'s `is_usb_rooted` check
///   exactly: [`UsbPathCheck`] on `file_path` only.
/// - `key_display` / `key_color` follow the user's key notation setting
///   (display only; `key` itself is what gets edited and exported).
pub(crate) fn apply_frontend_track_fields(
    conn: &rusqlite::Connection,
    tracks: &mut [Track],
) -> BackendResult<()> {
    let usb_paths = UsbPathCheck::load(conn)?;
    let notation = key_notation::key_notation_setting(conn)?;
    for track in tracks.iter_mut() {
        track.is_usb_path = usb_paths.is_usb_path(&track.file_path);
        (track.key_display, track.key_color) =
            key_notation::key_display_fields(track.key.as_deref(), notation);
    }
    Ok(())
}

pub fn check_essentia_installed(data_dir: &std::path::Path) -> bool {
    let node_modules = data_dir.join("essentia/node_modules");
    node_modules.join("essentia.js/package.json").is_file()
        && node_modules
            .join("essentia.js/dist/essentia-wasm.umd.js")
            .is_file()
        && node_modules.join("node-wav/package.json").is_file()
}

pub(super) fn check_node_available() -> bool {
    let node_bin = crate::essentia_runtime::node_bin();
    let mut cmd = std::process::Command::new(&node_bin);
    cmd.arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::utils::suppress_console_window(&mut cmd);
    cmd.status().map(|s| s.success()).unwrap_or(false)
}
