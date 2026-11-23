//! The USB drive's partition layout and filesystem (`usb_layout`) as a report
//! section. Only a warning: the app doesn't format drives, so it names the
//! difference and how to reformat.

use super::*;
use crate::i18n::{Text, tr};
use crate::service::usb_layout::{layout_problems, read_drive_layout};

/// `None` when `usb_root` isn't the mount point of a USB device: a folder
/// used as a USB has no drive layout of its own to check.
pub(super) fn drive_layout_section(
    usb_root: &std::path::Path,
    warnings: &mut Vec<WarningEntry>,
) -> Option<DiagSection> {
    let layout = read_drive_layout(usb_root)?;
    let problems = layout_problems(&layout);
    let check = if problems.is_empty() {
        DiagCheck::new(
            tr::diagnostics_check_drive_layout(),
            DiagStatus::Pass,
            tr::diagnostics_check_drive_layout_ok(),
        )
    } else {
        for problem in &problems {
            warnings.push(logging::log_text(
                Level::Warn,
                "usb-diagnostics",
                "usb.diagnostics.drive-layout",
                problem,
            ));
        }
        let detail = problems
            .iter()
            .chain([&tr::diagnostics_check_drive_layout_reformat()])
            .map(Text::render)
            .collect::<Vec<_>>()
            .join(" ");
        DiagCheck::new(
            tr::diagnostics_check_drive_layout(),
            DiagStatus::Warn,
            detail,
        )
    };
    Some(DiagSection {
        key: "",
        title: "USB drive".into(),
        status: check.status.clone(),
        checks: vec![check],
        counts: None,
    })
}
