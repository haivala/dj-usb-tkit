//! App settings: blocked actions, UI texts and language, frontend settings,
//! track columns, and data migrations.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What's blocked now (`job_conflicts::blocked_actions`): the groups, each with its
/// reason. `get_blocked_actions` answers with it; `jobs:blocked` sends the same map.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockedActionsData {
    pub blocked_actions: std::collections::BTreeMap<crate::job_conflicts::ActionGroup, String>,
}

/// `get_ui_texts`: every `ui-` text of the catalog, in the current language, for the text
/// JS and `index.html` show. Sent once at startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUiTextsData {
    /// The language code, for `Intl.PluralRules` in `tCount`.
    pub language: String,
    /// The language's decimal separator (`format-decimal-separator`), for the numbers JS
    /// formats itself (BPM, cue times), so they match the ones the backend renders.
    pub decimal_separator: String,
    pub texts: std::collections::BTreeMap<String, UiText>,
    /// Every language the app has, English first: the settings' language list.
    pub languages: Vec<UiLanguage>,
    /// Whether there's a language to choose: more than one in `languages`. The language
    /// lists (Settings, Quick Start) show only then.
    pub language_choice: bool,
}

/// One language the app has: its code and its name in itself ("English").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiLanguage {
    pub code: String,
    pub name: String,
}

/// One `ui-` text: a string with `{name}` placeholders, or a counted text's forms by plural
/// category (`{ "one": "{count} selected", "other": "{count} selected" }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UiText {
    Text(String),
    Counted(std::collections::BTreeMap<String, String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrontendSettingsData {
    pub values: HashMap<String, String>,
    /// The track table's saved column layout, cleaned (`track_columns`). Not in `values`,
    /// which holds only what `set_frontend_setting` writes.
    pub track_columns: TrackColumnLayout,
    pub node_available: bool,
    pub essentia_installed: bool,
}

/// The track table's optional columns (`service::track_columns`): their order between the
/// Track and action columns, and which are hidden. Column ids are strings here: a saved or
/// sent layout may hold ids this build doesn't know, which cleaning drops.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackColumnLayout {
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub hidden: Vec<String>,
}

/// One column of the track table, for the column editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackColumnInfo {
    pub id: String,
    /// The `ui-` text key of its header; none for the unlabeled columns.
    pub label_key: Option<String>,
    pub sortable: bool,
    /// Always shown, in a fixed place: not in the editor or a layout.
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTrackColumnsData {
    pub columns: Vec<TrackColumnInfo>,
    pub layout: TrackColumnLayout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTrackColumnsRequest {
    pub layout: TrackColumnLayout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTrackColumnsData {
    /// What was saved: the sent layout, cleaned.
    pub layout: TrackColumnLayout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFrontendSettingRequest {
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetLanguageRequest {
    /// A catalog's language code, as `get_ui_texts` lists them.
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetLanguageData {
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFrontendSettingData {
    pub saved: bool,
}

/// What a `run_data_migrations` pass did: the ids it completed, and those
/// that will run again on the next launch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDataMigrationsData {
    pub ran: Vec<String>,
    pub retry_later: Vec<String>,
}
