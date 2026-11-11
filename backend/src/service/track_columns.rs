//! The track table's columns: which exist, which the user may hide or move, and the saved
//! layout. One layout covers every track list (library, playlist, USB playlist, USB
//! history). The frontend draws the columns; what a layout may hold is decided here.

use rusqlite::{OptionalExtension, params};

use super::{BackendService, SETTING_UI_TRACK_COLUMNS, now};
use crate::error::BackendResult;
use crate::models::{
    GetTrackColumnsData, SetTrackColumnsData, SetTrackColumnsRequest, TrackColumnInfo,
    TrackColumnLayout,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackColumn {
    /// The drag-to-reorder handle; only the app and USB playlist grids have it.
    Drag,
    Select,
    Cover,
    Waveform,
    /// Artist and title.
    Track,
    Album,
    Year,
    Bpm,
    Key,
    Length,
    Format,
    Action,
}

impl TrackColumn {
    /// Every column, in the default order.
    pub(crate) const ALL: [Self; 12] = [
        Self::Drag,
        Self::Select,
        Self::Cover,
        Self::Waveform,
        Self::Track,
        Self::Album,
        Self::Year,
        Self::Bpm,
        Self::Key,
        Self::Length,
        Self::Format,
        Self::Action,
    ];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Drag => "drag",
            Self::Select => "select",
            Self::Cover => "cover",
            Self::Waveform => "waveform",
            Self::Track => "track",
            Self::Album => "album",
            Self::Year => "year",
            Self::Bpm => "bpm",
            Self::Key => "key",
            Self::Length => "length",
            Self::Format => "format",
            Self::Action => "action",
        }
    }

    fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|column| column.id() == id)
    }

    /// The `sortBy` key `TrackSort` sorts this column by (the Track header's first one).
    pub(crate) fn sort_key(self) -> Option<&'static str> {
        match self {
            Self::Track => Some("artist"),
            Self::Album => Some("album"),
            Self::Year => Some("year"),
            Self::Bpm => Some("bpm"),
            Self::Key => Some("key"),
            Self::Length => Some("durationMs"),
            Self::Format => Some("format"),
            Self::Drag | Self::Select | Self::Cover | Self::Waveform | Self::Action => None,
        }
    }

    fn label_key(self) -> Option<&'static str> {
        match self {
            Self::Track => Some("ui-track-table-track"),
            Self::Album => Some("ui-track-table-album"),
            Self::Year => Some("ui-track-table-year"),
            Self::Bpm => Some("ui-track-table-bpm"),
            Self::Key => Some("ui-track-table-key"),
            Self::Length => Some("ui-track-table-length"),
            Self::Format => Some("ui-track-table-fmt"),
            Self::Drag | Self::Select | Self::Cover | Self::Waveform | Self::Action => None,
        }
    }

    /// Always shown, in a fixed place: drag, select, cover, waveform and track lead,
    /// action is last. Only the other columns are hidden or moved, between track and
    /// action.
    pub(crate) fn locked(self) -> bool {
        matches!(
            self,
            Self::Drag | Self::Select | Self::Cover | Self::Waveform | Self::Track | Self::Action
        )
    }

    /// Hidden until the user shows it: what a column added later starts as.
    fn hidden_by_default(self) -> bool {
        false
    }

    fn optional() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|column| !column.locked())
    }
}

fn parse_optional(ids: &[String]) -> Vec<TrackColumn> {
    let mut columns: Vec<TrackColumn> = Vec::new();
    for column in ids.iter().filter_map(|id| TrackColumn::parse(id.trim())) {
        if !column.locked() && !columns.contains(&column) {
            columns.push(column);
        }
    }
    columns
}

fn ids(columns: impl IntoIterator<Item = TrackColumn>) -> Vec<String> {
    columns
        .into_iter()
        .map(|column| column.id().to_string())
        .collect()
}

/// The layout with every optional column in its default place and default visibility.
pub(crate) fn default_layout() -> TrackColumnLayout {
    TrackColumnLayout {
        order: ids(TrackColumn::optional()),
        hidden: ids(TrackColumn::optional().filter(|column| column.hidden_by_default())),
    }
}

/// `layout` cleaned: unknown, locked and repeated ids dropped, and each optional column it
/// lacks put in after the column it follows by default (first if none of those is there),
/// hidden if it starts hidden. `hidden` follows `order`.
pub(crate) fn normalize(layout: &TrackColumnLayout) -> TrackColumnLayout {
    let mut order = parse_optional(&layout.order);
    let mut hidden = parse_optional(&layout.hidden);
    let defaults: Vec<TrackColumn> = TrackColumn::optional().collect();
    for (index, &column) in defaults.iter().enumerate() {
        if order.contains(&column) {
            continue;
        }
        let at = defaults[..index]
            .iter()
            .rev()
            .find_map(|before| order.iter().position(|placed| placed == before))
            .map_or(0, |position| position + 1);
        order.insert(at, column);
        if column.hidden_by_default() && !hidden.contains(&column) {
            hidden.push(column);
        }
    }
    hidden.sort_by_key(|column| order.iter().position(|placed| placed == column));
    TrackColumnLayout {
        order: ids(order),
        hidden: ids(hidden),
    }
}

/// The saved layout, cleaned; the default one when none is saved or it doesn't parse.
pub(crate) fn track_columns_setting(
    conn: &rusqlite::Connection,
) -> BackendResult<TrackColumnLayout> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTING_UI_TRACK_COLUMNS],
            |row| row.get(0),
        )
        .optional()?;
    let saved = raw.and_then(
        |raw| match serde_json::from_str::<TrackColumnLayout>(&raw) {
            Ok(layout) => Some(layout),
            Err(err) => {
                crate::logging::debug(
                    "settings",
                    format!("Saved track column layout unreadable, using the default: {err}"),
                );
                None
            }
        },
    );
    Ok(saved.as_ref().map_or_else(default_layout, normalize))
}

impl BackendService {
    pub fn get_track_columns(&self) -> BackendResult<GetTrackColumnsData> {
        let layout = track_columns_setting(&*self.db.connect()?)?;
        let columns = TrackColumn::ALL
            .into_iter()
            .map(|column| TrackColumnInfo {
                id: column.id().to_string(),
                label_key: column.label_key().map(str::to_string),
                sortable: column.sort_key().is_some(),
                locked: column.locked(),
            })
            .collect();
        Ok(GetTrackColumnsData { columns, layout })
    }

    /// Saves `req.layout` cleaned, and answers with what was saved.
    pub fn set_track_columns(
        &self,
        req: SetTrackColumnsRequest,
    ) -> BackendResult<SetTrackColumnsData> {
        let layout = normalize(&req.layout);
        let value = serde_json::to_string(&layout)
            .map_err(|err| crate::error::BackendError::Internal(err.to_string()))?;
        self.db.connect()?.execute(
            r#"
            INSERT INTO app_settings (key, value, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
            "#,
            params![SETTING_UI_TRACK_COLUMNS, value, now()],
        )?;
        Ok(SetTrackColumnsData { layout })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(order: &[&str], hidden: &[&str]) -> TrackColumnLayout {
        let strings = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect();
        TrackColumnLayout {
            order: strings(order),
            hidden: strings(hidden),
        }
    }

    #[test]
    fn every_sort_key_is_one_track_sort_sorts_by() {
        for column in TrackColumn::ALL {
            if let Some(key) = column.sort_key() {
                assert!(
                    super::super::track_list::is_sort_key(key),
                    "{} sorts by {key}, which TrackSort doesn't know",
                    column.id()
                );
            }
        }
    }

    #[test]
    fn normalize_drops_unknown_locked_and_repeated_ids() {
        let cleaned = normalize(&layout(
            &["bpm", "nope", "waveform", "year", "bpm"],
            &["album", "track", "album", "nope"],
        ));
        assert_eq!(
            cleaned,
            layout(
                &["album", "bpm", "key", "length", "format", "year"],
                &["album"],
            )
        );
    }

    #[test]
    fn normalize_puts_a_missing_column_after_its_default_predecessor() {
        let cleaned = normalize(&layout(&["format", "key", "album"], &[]));
        assert_eq!(
            cleaned.order,
            ["format", "key", "length", "album", "year", "bpm"]
        );
        assert_eq!(normalize(&TrackColumnLayout::default()), default_layout());
    }

    #[test]
    fn normalize_keeps_hidden_in_column_order() {
        let cleaned = normalize(&layout(
            &["format", "album", "year", "bpm", "key", "length"],
            &["album", "format"],
        ));
        assert_eq!(cleaned.hidden, ["format", "album"]);
    }
}
