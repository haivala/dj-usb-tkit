//! Shared metadata cleanup used before writing player database text fields.

use std::borrow::Cow;

/// Strip null bytes and truncate to 255 Unicode characters.
///
/// Applied to exported metadata strings (titles, artist names, album names),
/// not media paths, analysis paths, or key/tonality values.
pub(crate) fn sanitize_metadata(s: &str) -> Cow<'_, str> {
    if !s.contains('\0') && s.chars().count() <= 255 {
        return Cow::Borrowed(s);
    }
    let stripped: String = s.chars().filter(|&c| c != '\0').take(255).collect();
    Cow::Owned(stripped)
}
