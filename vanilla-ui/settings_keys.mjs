// Settings persistence keys and bindings.
// Shared between settings hydration, persist helpers, and init.

export const STORAGE_KEY_THEME = "djusbtkit.theme";
export const STORAGE_KEY_ACCENT_HUE = "djusbtkit.accentHue";
export const STORAGE_KEY_SOURCE_ROOTS = "djusbtkit.sourceRoots";
export const STORAGE_KEY_SOURCE_ROOT_ENABLED = "djusbtkit.sourceRootEnabled";
export const STORAGE_KEY_USB_ROOT = "djusbtkit.usbRoot";
export const STORAGE_KEY_USB_RECENT_ROOTS = "djusbtkit.usbRecentRoots";
export const STORAGE_KEY_EXPORT_PRUNE_STALE = "djusbtkit.exportPruneStale";
export const STORAGE_KEY_EXPORT_BACKUP = "djusbtkit.exportBackup";
export const STORAGE_KEY_ANALYSIS_BPM_RANGE = "djusbtkit.analysisBpmRange";
export const STORAGE_KEY_ANALYSIS_ENGINE = "djusbtkit.analysisEngine";
export const STORAGE_KEY_SIDEBAR_COLLAPSED = "djusbtkit.sidebarCollapsed";
export const STORAGE_KEY_HELP_SEEN = "djusbtkit.helpSeen";
export const STORAGE_KEY_MASTER_DB_ENABLED = "djusbtkit.masterDbEnabled";
export const STORAGE_KEY_SOURCES_EVER_CONFIGURED = "djusbtkit.sourcesEverConfigured";
export const STORAGE_KEY_UPDATE_DISMISSED = "djusbtkit.updateDismissedVersion";

export const FRONTEND_DB_KEY_THEME = "ui_theme_v1";
export const FRONTEND_DB_KEY_ACCENT_HUE = "ui_accent_hue_v1";
export const FRONTEND_DB_KEY_SOURCE_ROOTS = "ui_source_roots_v1";
export const FRONTEND_DB_KEY_SOURCE_ROOT_ENABLED = "ui_source_root_enabled_v1";
export const FRONTEND_DB_KEY_USB_ROOT = "ui_usb_root_v1";
export const FRONTEND_DB_KEY_USB_RECENT_ROOTS = "ui_usb_recent_roots_v1";
export const FRONTEND_DB_KEY_EXPORT_PRUNE_STALE = "ui_export_prune_stale_v1";
export const FRONTEND_DB_KEY_EXPORT_BACKUP = "ui_export_backup_v1";
export const FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE = "ui_analysis_bpm_range_v1";
export const FRONTEND_DB_KEY_ANALYSIS_ENGINE = "ui_analysis_engine_v1";
export const FRONTEND_DB_KEY_SIDEBAR_COLLAPSED = "ui_sidebar_collapsed_v1";
export const FRONTEND_DB_KEY_HELP_SEEN = "ui_help_seen_v1";
export const FRONTEND_DB_KEY_MASTER_DB_ENABLED = "ui_master_db_enabled_v1";
export const FRONTEND_DB_KEY_SOURCES_EVER_CONFIGURED = "ui_sources_ever_configured_v1";

export const FRONTEND_SETTING_BINDINGS = [
  { storageKey: STORAGE_KEY_THEME, dbKey: FRONTEND_DB_KEY_THEME },
  { storageKey: STORAGE_KEY_ACCENT_HUE, dbKey: FRONTEND_DB_KEY_ACCENT_HUE },
  { storageKey: STORAGE_KEY_SOURCE_ROOTS, dbKey: FRONTEND_DB_KEY_SOURCE_ROOTS },
  { storageKey: STORAGE_KEY_SOURCE_ROOT_ENABLED, dbKey: FRONTEND_DB_KEY_SOURCE_ROOT_ENABLED },
  { storageKey: STORAGE_KEY_USB_ROOT, dbKey: FRONTEND_DB_KEY_USB_ROOT },
  { storageKey: STORAGE_KEY_USB_RECENT_ROOTS, dbKey: FRONTEND_DB_KEY_USB_RECENT_ROOTS },
  { storageKey: STORAGE_KEY_EXPORT_PRUNE_STALE, dbKey: FRONTEND_DB_KEY_EXPORT_PRUNE_STALE },
  { storageKey: STORAGE_KEY_EXPORT_BACKUP, dbKey: FRONTEND_DB_KEY_EXPORT_BACKUP },
  { storageKey: STORAGE_KEY_ANALYSIS_BPM_RANGE, dbKey: FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE },
  { storageKey: STORAGE_KEY_ANALYSIS_ENGINE, dbKey: FRONTEND_DB_KEY_ANALYSIS_ENGINE },
  { storageKey: STORAGE_KEY_SIDEBAR_COLLAPSED, dbKey: FRONTEND_DB_KEY_SIDEBAR_COLLAPSED },
  { storageKey: STORAGE_KEY_HELP_SEEN, dbKey: FRONTEND_DB_KEY_HELP_SEEN },
  { storageKey: STORAGE_KEY_MASTER_DB_ENABLED, dbKey: FRONTEND_DB_KEY_MASTER_DB_ENABLED },
  { storageKey: STORAGE_KEY_SOURCES_EVER_CONFIGURED, dbKey: FRONTEND_DB_KEY_SOURCES_EVER_CONFIGURED }
];
