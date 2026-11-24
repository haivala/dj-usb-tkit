//! The selected USB stick: validating and selecting a root, and the slow-stage
//! timing log its reads share. The modules below read, cache, edit and back up
//! what is on it.

pub(crate) mod audio_formats;
pub mod backups;
pub(crate) mod cache;
pub(crate) mod display_files;
pub(crate) mod history;
pub(super) mod history_tracklist;
pub(crate) mod identity;
pub(crate) mod initialize;
pub(crate) mod layout;
pub(super) mod order_edit;
pub(crate) mod playlist_tree;
mod playlists;
pub(crate) mod root;
pub mod staging;
pub(crate) mod stick_files;
pub(crate) mod text;
pub(crate) mod track_facts;
mod tracks;
pub mod vendor_compat;

use self::root::{
    canonicalize_or_self, has_write_access, normalize_usb_root_path, resolve_usb_root,
};
use self::staging::{UsbDatabases, UsbDatabasesExt};
use self::vendor_compat::{USB_CONTENTS_DIR, USB_VENDOR_ROOT_DIR};
use crate::error::{BackendError, BackendResult};
use crate::i18n::tr;
use crate::logging::{self, Level};
use crate::models::{ValidateUsbRootData, ValidateUsbRootRequest, WarningEntry};
use crate::service::{BackendService, now};

const SLOW_USB_STAGE_MS: u128 = 8_000;

/// What the UI says about a selected folder, and whether it can be set up: a
/// folder that exists and is writable, but isn't a USB library yet.
fn with_usb_selection_text(mut data: ValidateUsbRootData) -> ValidateUsbRootData {
    let warning_text = data
        .warnings
        .iter()
        .map(|warning| warning.message.trim())
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    let warned = !warning_text.is_empty();
    data.can_initialize = data.normalized_root.is_some() && !data.valid && data.has_write_access;
    if data.valid {
        let path = data.normalized_root.as_deref().unwrap_or_default();
        data.status_text = if warned {
            tr::usb_root_selected_with_warnings(path, &warning_text)
        } else {
            tr::usb_root_selected(path)
        }
        .render();
    } else if data.can_initialize {
        data.status_text = tr::usb_root_not_initialized().render();
        data.init_hint_text = if warned {
            tr::usb_root_init_hint_missing_structure_with_warnings(&warning_text)
        } else {
            tr::usb_root_init_hint_missing_structure()
        }
        .render();
    } else {
        let why = if warned {
            warning_text.clone()
        } else {
            tr::usb_root_invalid_unknown().render()
        };
        data.status_text = tr::usb_root_invalid(why).render();
        data.init_hint_text = if warned {
            tr::usb_root_init_hint_not_ready_with_warnings(&warning_text)
        } else {
            tr::usb_root_init_hint_not_ready()
        }
        .render();
    }
    data
}

fn push_usb_stage_timing(stage: &str, started: &mut std::time::Instant) {
    push_usb_stage_timing_with_threshold(stage, started, SLOW_USB_STAGE_MS);
}

/// Extra allowance for eDB-heavy stages whose wall-clock cost scales with
/// library size (SQL joins over `content`/`playlist`/etc. that grow with
/// track count), not media speed -- see `push_usb_stage_timing_with_threshold`
/// for why a flat threshold alone misdiagnoses "slow media" on large-but-fast
/// libraries.
const SLOW_USB_STAGE_PER_TRACK_MS: u128 = 1;

fn slow_stage_threshold_ms(item_count: usize) -> u128 {
    SLOW_USB_STAGE_MS + (item_count as u128) * SLOW_USB_STAGE_PER_TRACK_MS
}

/// Same as `push_usb_stage_timing`, but with an explicit threshold instead
/// of the flat `SLOW_USB_STAGE_MS` default. Stages whose cost is genuinely
/// proportional to library size (not media speed) should pass a threshold
/// computed via `slow_stage_threshold_ms` instead of assuming every stage
/// costs the same regardless of how much data it reads.
/// Logs the stage's time to the log file; returns whether it looked slow.
fn push_usb_stage_timing_with_threshold(
    stage: &str,
    started: &mut std::time::Instant,
    threshold_ms: u128,
) -> bool {
    log_stage_timing("usb-import", stage, started, threshold_ms)
}

/// Logs how long `stage` took since `started` under `category`, flags it as
/// slow media at `threshold_ms` or more, and restarts `started` for the next
/// stage. Returns whether it looked slow.
pub(crate) fn log_stage_timing(
    category: &str,
    stage: &str,
    started: &mut std::time::Instant,
    threshold_ms: u128,
) -> bool {
    let elapsed = started.elapsed().as_millis();
    logging::debug(category, format!("stage timing: {stage}: {elapsed}ms"));
    let slow = elapsed >= threshold_ms;
    if slow {
        logging::debug(
            category,
            format!("slow-media suspected: stage '{stage}' took {elapsed}ms"),
        );
    }
    *started = std::time::Instant::now();
    slow
}

impl BackendService {
    /// Checks the USB at `req.path`, and makes it the selected USB when it's valid (no USB
    /// selected otherwise).
    pub fn validate_usb_root(
        &self,
        req: ValidateUsbRootRequest,
    ) -> BackendResult<ValidateUsbRootData> {
        let _usb_write_guard = self
            .try_lock_usb_writes()
            .ok_or_else(|| BackendError::Validation(tr::usb_write_busy()))?;
        let result = self.check_usb_root(req);
        let selected = result
            .as_ref()
            .ok()
            .filter(|data| data.valid)
            .and_then(|data| data.normalized_root.as_ref())
            .map(std::path::PathBuf::from);
        self.set_selected_usb_root(selected);
        result
    }

    fn check_usb_root(&self, req: ValidateUsbRootRequest) -> BackendResult<ValidateUsbRootData> {
        let mut warnings = Vec::<WarningEntry>::new();
        let trimmed = req.path.trim();
        if trimmed.is_empty() {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.path-empty",
                &tr::usb_warning_path_empty(),
            ));
            return Ok(with_usb_selection_text(ValidateUsbRootData {
                valid: false,
                has_write_access: false,
                normalized_root: None,
                has_vendor_root: false,
                has_contents: false,
                has_pdb: false,
                has_edb: false,
                can_initialize: false,
                status_text: String::new(),
                init_hint_text: String::new(),
                warnings,
            }));
        }

        let raw = std::path::PathBuf::from(trimmed);
        let candidate = if raw.exists() {
            raw
        } else if raw.is_relative() {
            std::env::current_dir()?.join(raw)
        } else {
            raw
        };
        if !candidate.exists() {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.path-not-found",
                &tr::usb_warning_path_not_found(candidate.display()),
            ));
            return Ok(with_usb_selection_text(ValidateUsbRootData {
                valid: false,
                has_write_access: false,
                normalized_root: None,
                has_vendor_root: false,
                has_contents: false,
                has_pdb: false,
                has_edb: false,
                can_initialize: false,
                status_text: String::new(),
                init_hint_text: String::new(),
                warnings,
            }));
        }

        let normalized = normalize_usb_root_path(canonicalize_or_self(candidate));
        let mount_conn = self.db.connect()?;
        crate::service::usb::identity::upsert_usb_device(&mount_conn, &normalized, true, &now())?;
        // Before staging below: a drive carrying another drive's name must
        // not stage into that drive's cache.
        self.check_drive_identity(&mount_conn, &normalized)?;
        drop(mount_conn);
        let has_vendor_root = normalized.join(USB_VENDOR_ROOT_DIR).is_dir();
        let has_contents = normalized.join(USB_CONTENTS_DIR).is_dir();

        if !has_vendor_root {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.missing-vendor-root",
                &tr::usb_warning_missing_vendor_root(),
            ));
        }
        if !has_contents {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.missing-contents",
                &tr::usb_warning_missing_contents(),
            ));
        }

        // Stage the local HDD working copy of the PDB/eDB exactly once, here
        // at connect time, so every read for the rest of this connected
        // session (diagnostics, playlist fetch, track inspect, export, ...)
        // can trust the local copy and skip its own USB stat() -- see
        // `usb::staging::stage_file_with_root`'s verified-this-connect fast
        // path. A database the USB lacks is dropped from staging. Best-effort:
        // a staging hiccup shouldn't fail validation.
        let databases = UsbDatabases::on_connect(&normalized);
        let has_pdb = databases.pdb().is_some();
        let has_edb = databases.edb().is_some();

        let has_write_access = has_write_access(&normalized);
        if !has_write_access {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-import",
                "usb.import.read-only",
                &tr::usb_warning_read_only(),
            ));
        }

        Ok(with_usb_selection_text(ValidateUsbRootData {
            valid: has_vendor_root && has_contents,
            has_write_access,
            normalized_root: Some(normalized.to_string_lossy().to_string()),
            has_vendor_root,
            has_contents,
            has_pdb,
            has_edb,
            can_initialize: false,
            status_text: String::new(),
            init_hint_text: String::new(),
            warnings,
        }))
    }

    /// The selected USB (`selected_usb_root`), if any.
    pub(crate) fn selected_usb_root(&self) -> Option<std::path::PathBuf> {
        self.selected_usb_root
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn set_selected_usb_root(&self, root: Option<std::path::PathBuf>) {
        *self
            .selected_usb_root
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = root;
    }

    /// No USB selected any more: the window cleared its choice.
    /// Selects the USB at `req.path` (none when empty): checks it, makes it the selected USB
    /// when valid, saves it as the folder the app reopens with, and answers with what the USB
    /// page shows for it.
    pub fn select_usb(
        &self,
        req: crate::models::SelectUsbRequest,
    ) -> BackendResult<crate::models::SelectUsbData> {
        let path = req.path.trim();
        // Another USB's order edit ends here (refused while it's being saved).
        let new_root = (!path.is_empty())
            .then(|| resolve_usb_root(Some(path)).ok())
            .flatten();
        self.switch_usb_order_edit_root(new_root.as_deref())?;
        let usb = if path.is_empty() {
            self.set_selected_usb_root(None);
            ValidateUsbRootData {
                valid: false,
                normalized_root: None,
                has_write_access: false,
                has_vendor_root: false,
                has_contents: false,
                has_pdb: false,
                has_edb: false,
                can_initialize: false,
                status_text: tr::usb_root_cleared().render(),
                init_hint_text: String::new(),
                warnings: Vec::new(),
            }
        } else {
            self.validate_usb_root(ValidateUsbRootRequest {
                path: path.to_string(),
            })?
        };
        let saved_root = usb
            .normalized_root
            .clone()
            .or_else(|| (!path.is_empty()).then(|| path.to_string()));
        self.save_usb_root_setting(saved_root.as_deref())?;

        let valid_root = usb.normalized_root.clone().filter(|_| usb.valid);
        let audio_formats = valid_root.as_ref().and_then(|root| {
            self.get_usb_audio_formats(crate::models::GetUsbAudioFormatsRequest {
                usb_root: root.clone(),
            })
            .map_err(|err| {
                logging::emit_text(Level::Warn, "usb", &tr::usb_audio_formats_read_failed(&err));
            })
            .ok()
        });
        let device_name = valid_root.as_ref().and_then(|root| {
            self.get_usb_device_name(crate::models::GetUsbDeviceNameRequest {
                usb_root: root.clone(),
            })
            .map_err(|err| {
                logging::emit_text(Level::Warn, "usb", &tr::usb_name_check_failed(&err));
            })
            .ok()
        });
        let devices = self.list_usb_devices()?.items;
        Ok(crate::models::SelectUsbData {
            usb,
            audio_formats,
            device_name,
            devices,
        })
    }
}

#[cfg(test)]
mod tests;
