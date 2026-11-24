//! The library's playlist folders and the sidebar tree they make with the
//! playlists.
//!
//! Folders (`playlist_folders.parent_id`) and playlists (`playlists.folder_id`)
//! share one order per parent (`sort_order`, the same column on both tables).
//! Items never ordered by hand have none and come after the ordered ones,
//! newest first, as playlists did before folders. The tree is the library's
//! own: it never changes anything on the USB.

use super::*;
use crate::i18n::tr;
use crate::models::{
    CreatePlaylistFolderData, CreatePlaylistFolderRequest, DeletePlaylistFolderData,
    DeletePlaylistFolderRequest, MovePlaylistTreeItemData, MovePlaylistTreeItemRequest, Playlist,
    PlaylistFolder, PlaylistListRow, PlaylistSidebarRow, PlaylistTreeItem, PlaylistTreeKind,
    PlaylistTreePlacement, RenamePlaylistFolderData, RenamePlaylistFolderRequest,
    SetPlaylistFolderCollapsedData, SetPlaylistFolderCollapsedRequest,
};
use std::collections::{HashMap, HashSet};

/// One folder or playlist as the tree orders it.
#[derive(Debug, Clone)]
struct Node {
    item: PlaylistTreeItem,
    parent: Option<String>,
    sort_order: Option<i64>,
    created_at: String,
}

fn load_nodes(conn: &rusqlite::Connection) -> BackendResult<Vec<Node>> {
    let mut nodes = Vec::new();
    for (kind, sql) in [
        (
            PlaylistTreeKind::Folder,
            "SELECT id, parent_id, sort_order, created_at FROM playlist_folders",
        ),
        (
            PlaylistTreeKind::Playlist,
            "SELECT id, folder_id, sort_order, created_at FROM playlists",
        ),
    ] {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(Node {
                item: PlaylistTreeItem {
                    kind,
                    id: row.get(0)?,
                },
                parent: row.get(1)?,
                sort_order: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        for row in rows {
            nodes.push(row?);
        }
    }
    Ok(nodes)
}

/// The tree: each parent's children in order. A child whose folder is gone
/// sits at the top level.
struct Tree {
    children: HashMap<Option<String>, Vec<PlaylistTreeItem>>,
    parent: HashMap<PlaylistTreeItem, Option<String>>,
}

impl Tree {
    fn load(conn: &rusqlite::Connection) -> BackendResult<Self> {
        let mut nodes = load_nodes(conn)?;
        let folders: HashSet<String> = nodes
            .iter()
            .filter(|node| node.item.kind == PlaylistTreeKind::Folder)
            .map(|node| node.item.id.clone())
            .collect();
        for node in &mut nodes {
            if node.parent.as_ref().is_some_and(|p| !folders.contains(p)) {
                node.parent = None;
            }
        }
        // Ordered first; then newest first, as the sidebar always listed new
        // playlists on top.
        nodes.sort_by(|a, b| {
            (a.sort_order.is_none(), a.sort_order)
                .cmp(&(b.sort_order.is_none(), b.sort_order))
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| b.item.id.cmp(&a.item.id))
        });
        let mut children: HashMap<Option<String>, Vec<PlaylistTreeItem>> = HashMap::new();
        let mut parent = HashMap::new();
        for node in nodes {
            parent.insert(node.item.clone(), node.parent.clone());
            children.entry(node.parent).or_default().push(node.item);
        }
        Ok(Self { children, parent })
    }

    fn children(&self, parent: Option<&str>) -> &[PlaylistTreeItem] {
        self.children
            .get(&parent.map(str::to_string))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Every item depth-first, top first, with its depth; `descend` says
    /// whether a folder's children are visited. A folder loop is cut where it
    /// repeats.
    fn walk(&self, descend: impl Fn(&str) -> bool) -> Vec<(PlaylistTreeItem, usize)> {
        fn visit(
            tree: &Tree,
            parent: Option<&str>,
            depth: usize,
            descend: &dyn Fn(&str) -> bool,
            seen: &mut HashSet<String>,
            out: &mut Vec<(PlaylistTreeItem, usize)>,
        ) {
            for item in tree.children(parent) {
                out.push((item.clone(), depth));
                if item.kind == PlaylistTreeKind::Folder
                    && seen.insert(item.id.clone())
                    && descend(&item.id)
                {
                    visit(tree, Some(&item.id), depth + 1, descend, seen, out);
                }
            }
        }
        let mut out = Vec::new();
        visit(self, None, 0, &descend, &mut HashSet::new(), &mut out);
        out
    }

    /// Whether `folder` is `ancestor` or inside it.
    fn is_within(&self, folder: &str, ancestor: &str) -> bool {
        let mut current = Some(folder.to_string());
        let mut seen = HashSet::new();
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            if !seen.insert(id.clone()) {
                return false;
            }
            current = self
                .parent
                .get(&PlaylistTreeItem {
                    kind: PlaylistTreeKind::Folder,
                    id,
                })
                .cloned()
                .flatten();
        }
        false
    }
}

/// Writes `items` as `parent`'s children, in this order.
fn write_children(
    conn: &rusqlite::Connection,
    parent: Option<&str>,
    items: &[PlaylistTreeItem],
) -> BackendResult<()> {
    for (index, item) in items.iter().enumerate() {
        let sql = match item.kind {
            PlaylistTreeKind::Folder => {
                "UPDATE playlist_folders SET parent_id = ?1, sort_order = ?2 WHERE id = ?3"
            }
            PlaylistTreeKind::Playlist => {
                "UPDATE playlists SET folder_id = ?1, sort_order = ?2 WHERE id = ?3"
            }
        };
        conn.execute(sql, params![parent, index as i64, item.id])?;
    }
    Ok(())
}

/// The `sort_order` that puts a new item first among `parent`'s children.
/// A new folder `name` first in `parent` (none: the top level), which opens.
/// Returns its id.
fn insert_folder(
    conn: &rusqlite::Connection,
    name: &str,
    parent: Option<&str>,
) -> BackendResult<String> {
    let id = Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO playlist_folders (id, name, parent_id, sort_order, collapsed, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
        params![id, name, parent, first_sort_order(conn, parent)?, now()],
    )?;
    if let Some(parent) = parent {
        expand_folder(conn, parent)?;
    }
    Ok(id)
}

/// The library folder at `names` (outermost first), each found by name in
/// its parent (compared as playlist names are; the first in its order), else
/// made first there. Returns the innermost folder's id; none for no names.
pub(crate) fn ensure_folder_path(
    conn: &rusqlite::Connection,
    names: &[String],
) -> BackendResult<Option<String>> {
    let canonical = crate::service::usb::text::canonicalize_playlist_name;
    let mut parent: Option<String> = None;
    for name in names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
    {
        let existing = {
            let mut stmt = conn.prepare(
                "SELECT id, name FROM playlist_folders WHERE parent_id IS ?1
                 ORDER BY sort_order IS NULL, sort_order, created_at DESC",
            )?;
            let rows = stmt
                .query_map(params![parent], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter()
                .find(|(_, existing)| canonical(existing) == canonical(name))
                .map(|(id, _)| id)
        };
        parent = Some(match existing {
            Some(id) => id,
            None => insert_folder(conn, name, parent.as_deref())?,
        });
    }
    Ok(parent)
}

pub(crate) fn first_sort_order(
    conn: &rusqlite::Connection,
    parent: Option<&str>,
) -> BackendResult<i64> {
    let lowest: Option<i64> = conn.query_row(
        "SELECT MIN(sort_order) FROM (
           SELECT sort_order FROM playlist_folders WHERE parent_id IS ?1
           UNION ALL SELECT sort_order FROM playlists WHERE folder_id IS ?1)",
        params![parent],
        |row| row.get(0),
    )?;
    Ok(lowest.unwrap_or(0) - 1)
}

/// The library folders above `playlist_id`, outermost first, as folder rows
/// for the export writers: names only (id 0: a new folder takes the next free
/// id). Empty for a top-level playlist.
pub(crate) fn playlist_folder_rows(
    conn: &rusqlite::Connection,
    playlist_id: &str,
) -> BackendResult<Vec<crate::pdb_reader::PdbPlaylistTreeRow>> {
    let mut current: Option<String> = conn
        .query_row(
            "SELECT folder_id FROM playlists WHERE id = ?1",
            params![playlist_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    while let Some(id) = current {
        if !seen.insert(id.clone()) {
            break;
        }
        let Some((name, parent)) = conn
            .query_row(
                "SELECT name, parent_id FROM playlist_folders WHERE id = ?1",
                params![id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?
        else {
            break;
        };
        rows.push(crate::pdb_reader::PdbPlaylistTreeRow {
            id: 0,
            parent_id: 0,
            sort_order: 0,
            is_folder: true,
            name,
        });
        current = parent;
    }
    rows.reverse();
    Ok(rows)
}

pub(crate) fn ensure_folder_exists(
    conn: &rusqlite::Connection,
    folder_id: &str,
) -> BackendResult<()> {
    let found: Option<String> = conn
        .query_row(
            "SELECT id FROM playlist_folders WHERE id = ?1",
            params![folder_id],
            |row| row.get(0),
        )
        .optional()?;
    match found {
        Some(_) => Ok(()),
        None => Err(BackendError::NotFound(tr::error_playlist_folder_not_found(
            folder_id,
        ))),
    }
}

/// The folders, the sidebar rows, and `playlists` put in tree order (top
/// first, depth-first, including those in collapsed folders) with their
/// folder paths.
/// Every playlist in `ordered` (the order `arrange_playlists` gives) under
/// its folders, every folder open: see [`PlaylistListRow`]. A playlist the
/// tree walk misses (a folder loop) comes last, at the top level.
pub(crate) fn playlist_list_rows(
    conn: &rusqlite::Connection,
    ordered: &[&str],
) -> BackendResult<Vec<PlaylistListRow>> {
    let tree = Tree::load(conn)?;
    let names: HashMap<String, String> = conn
        .prepare("SELECT id, name FROM playlist_folders")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let wanted: HashSet<&str> = ordered.iter().copied().collect();
    let walked = tree.walk(|_| true);
    let mut rows: Vec<PlaylistListRow> = Vec::with_capacity(walked.len());
    // Open folders, innermost last: (row index, depth).
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut listed = HashSet::new();
    for (item, depth) in walked {
        while open.last().is_some_and(|(_, d)| *d >= depth) {
            open.pop();
        }
        match item.kind {
            PlaylistTreeKind::Folder => {
                open.push((rows.len(), depth));
                rows.push(PlaylistListRow {
                    kind: PlaylistTreeKind::Folder,
                    name: names.get(&item.id).cloned().unwrap_or_default(),
                    id: item.id,
                    depth,
                    playlist_ids: Vec::new(),
                });
            }
            PlaylistTreeKind::Playlist => {
                if !wanted.contains(item.id.as_str()) || !listed.insert(item.id.clone()) {
                    continue;
                }
                for (index, _) in &open {
                    rows[*index].playlist_ids.push(item.id.clone());
                }
                rows.push(PlaylistListRow {
                    kind: PlaylistTreeKind::Playlist,
                    id: item.id,
                    depth,
                    name: String::new(),
                    playlist_ids: Vec::new(),
                });
            }
        }
    }
    rows.retain(|row| row.kind == PlaylistTreeKind::Playlist || !row.playlist_ids.is_empty());
    for id in ordered {
        if !listed.contains(*id) {
            rows.push(PlaylistListRow {
                kind: PlaylistTreeKind::Playlist,
                id: id.to_string(),
                depth: 0,
                name: String::new(),
                playlist_ids: Vec::new(),
            });
        }
    }
    Ok(rows)
}

pub(crate) fn arrange_playlists(
    conn: &rusqlite::Connection,
    playlists: Vec<Playlist>,
) -> BackendResult<(Vec<Playlist>, Vec<PlaylistFolder>, Vec<PlaylistSidebarRow>)> {
    let tree = Tree::load(conn)?;
    let mut stmt = conn.prepare("SELECT id, name, parent_id, collapsed FROM playlist_folders")?;
    let folder_rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let folder_names: HashMap<&str, &str> = folder_rows
        .iter()
        .map(|(id, name, _, _)| (id.as_str(), name.as_str()))
        .collect();
    let collapsed: HashSet<&str> = folder_rows
        .iter()
        .filter(|(_, _, _, collapsed)| *collapsed)
        .map(|(id, _, _, _)| id.as_str())
        .collect();

    let folder_path = |item: &PlaylistTreeItem| -> Option<String> {
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        let mut current = tree.parent.get(item).cloned().flatten();
        while let Some(id) = current {
            if !seen.insert(id.clone()) {
                break;
            }
            names.push(folder_names.get(id.as_str()).copied().unwrap_or_default());
            current = tree
                .parent
                .get(&PlaylistTreeItem {
                    kind: PlaylistTreeKind::Folder,
                    id,
                })
                .cloned()
                .flatten();
        }
        names.reverse();
        (!names.is_empty()).then(|| names.join(" / "))
    };

    let mut by_id: HashMap<String, Playlist> = playlists
        .into_iter()
        .map(|playlist| (playlist.id.clone(), playlist))
        .collect();
    let mut ordered = Vec::with_capacity(by_id.len());
    for (item, _) in tree.walk(|_| true) {
        if item.kind != PlaylistTreeKind::Playlist {
            continue;
        }
        if let Some(mut playlist) = by_id.remove(&item.id) {
            playlist.folder_id = tree.parent.get(&item).cloned().flatten();
            playlist.folder_path = folder_path(&item);
            ordered.push(playlist);
        }
    }
    // A playlist the walk missed (a folder loop) still shows, at the end.
    let mut rest: Vec<Playlist> = by_id.into_values().collect();
    rest.sort_by(|a, b| a.id.cmp(&b.id));
    ordered.extend(rest);

    let sidebar = tree
        .walk(|folder| !collapsed.contains(folder))
        .into_iter()
        .map(|(item, depth)| PlaylistSidebarRow {
            kind: item.kind,
            id: item.id,
            depth,
        })
        .collect();

    let folders = folder_rows
        .iter()
        .map(|(id, name, _, collapsed)| {
            let holds_items = !tree.children(Some(id)).is_empty();
            PlaylistFolder {
                id: id.clone(),
                name: name.clone(),
                parent_id: tree
                    .parent
                    .get(&PlaylistTreeItem {
                        kind: PlaylistTreeKind::Folder,
                        id: id.clone(),
                    })
                    .cloned()
                    .flatten(),
                collapsed: *collapsed,
                delete_confirm_text: if holds_items {
                    tr::playlist_folder_delete_confirm_items(name)
                } else {
                    tr::playlist_folder_delete_confirm(name)
                }
                .render(),
                toggle_tooltip: if *collapsed {
                    tr::playlist_folder_expand()
                } else {
                    tr::playlist_folder_collapse()
                }
                .render(),
            }
        })
        .collect();
    Ok((ordered, folders, sidebar))
}

fn folder_name_arg(name: &str) -> BackendResult<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(BackendError::Validation(
            tr::playlist_folder_error_name_empty(),
        ));
    }
    Ok(name)
}

impl BackendService {
    pub fn create_playlist_folder(
        &self,
        req: CreatePlaylistFolderRequest,
    ) -> BackendResult<CreatePlaylistFolderData> {
        let name = folder_name_arg(&req.name)?;
        let conn = self.db.connect()?;
        let parent = req.parent_id.as_deref();
        if let Some(parent) = parent {
            ensure_folder_exists(&conn, parent)?;
        }
        let id = insert_folder(&conn, name, parent)?;
        Ok(CreatePlaylistFolderData {
            folder_id: id,
            name: name.to_string(),
            status_text: tr::playlist_folder_created(name).render(),
        })
    }

    pub fn rename_playlist_folder(
        &self,
        req: RenamePlaylistFolderRequest,
    ) -> BackendResult<RenamePlaylistFolderData> {
        let name = folder_name_arg(&req.name)?;
        let conn = self.db.connect()?;
        ensure_folder_exists(&conn, &req.folder_id)?;
        conn.execute(
            "UPDATE playlist_folders SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now(), req.folder_id],
        )?;
        Ok(RenamePlaylistFolderData {
            folder_id: req.folder_id,
            name: name.to_string(),
        })
    }

    /// Deletes the folder only: what it holds moves up one level, in its
    /// place and order. Playlists are never deleted with a folder.
    pub fn delete_playlist_folder(
        &self,
        req: DeletePlaylistFolderRequest,
    ) -> BackendResult<DeletePlaylistFolderData> {
        let mut conn = self.db.connect()?;
        ensure_folder_exists(&conn, &req.folder_id)?;
        let name: String = conn.query_row(
            "SELECT name FROM playlist_folders WHERE id = ?1",
            params![req.folder_id],
            |row| row.get(0),
        )?;
        let tx = conn.transaction()?;
        let tree = Tree::load(&tx)?;
        let folder = PlaylistTreeItem {
            kind: PlaylistTreeKind::Folder,
            id: req.folder_id.clone(),
        };
        let parent = tree.parent.get(&folder).cloned().flatten();
        let mut siblings = Vec::new();
        for item in tree.children(parent.as_deref()) {
            if *item == folder {
                siblings.extend(tree.children(Some(&req.folder_id)).iter().cloned());
            } else {
                siblings.push(item.clone());
            }
        }
        write_children(&tx, parent.as_deref(), &siblings)?;
        let deleted = tx.execute(
            "DELETE FROM playlist_folders WHERE id = ?1",
            params![req.folder_id],
        )?;
        tx.commit()?;
        Ok(DeletePlaylistFolderData {
            folder_id: req.folder_id,
            deleted: deleted > 0,
            status_text: tr::playlist_folder_deleted(name).render(),
        })
    }

    pub fn set_playlist_folder_collapsed(
        &self,
        req: SetPlaylistFolderCollapsedRequest,
    ) -> BackendResult<SetPlaylistFolderCollapsedData> {
        let conn = self.db.connect()?;
        ensure_folder_exists(&conn, &req.folder_id)?;
        conn.execute(
            "UPDATE playlist_folders SET collapsed = ?1 WHERE id = ?2",
            params![req.collapsed, req.folder_id],
        )?;
        Ok(SetPlaylistFolderCollapsedData {
            folder_id: req.folder_id,
            collapsed: req.collapsed,
        })
    }

    /// Moves a folder (with what it holds) or a playlist before or after
    /// `target`, or to the end of the folder `target`, which then opens.
    pub fn move_playlist_tree_item(
        &self,
        req: MovePlaylistTreeItemRequest,
    ) -> BackendResult<MovePlaylistTreeItemData> {
        let mut conn = self.db.connect()?;
        let tx = conn.transaction()?;
        let tree = Tree::load(&tx)?;
        for item in [&req.item, &req.target] {
            if !tree.parent.contains_key(item) {
                return Err(BackendError::NotFound(match item.kind {
                    PlaylistTreeKind::Folder => tr::error_playlist_folder_not_found(&item.id),
                    PlaylistTreeKind::Playlist => tr::error_playlist_not_found(&item.id),
                }));
            }
        }
        // Next to itself it stays where it is; inside itself is refused below.
        if req.item == req.target && req.placement != PlaylistTreePlacement::Inside {
            return Ok(MovePlaylistTreeItemData { item: req.item });
        }
        let parent = match req.placement {
            PlaylistTreePlacement::Inside => {
                if req.target.kind != PlaylistTreeKind::Folder {
                    return Err(BackendError::Validation(
                        tr::playlist_folder_error_move_into_playlist(),
                    ));
                }
                Some(req.target.id.clone())
            }
            _ => tree.parent.get(&req.target).cloned().flatten(),
        };
        if req.item.kind == PlaylistTreeKind::Folder
            && parent
                .as_deref()
                .is_some_and(|parent| tree.is_within(parent, &req.item.id))
        {
            return Err(BackendError::Validation(
                tr::playlist_folder_error_move_into_itself(),
            ));
        }
        let mut siblings: Vec<PlaylistTreeItem> = tree
            .children(parent.as_deref())
            .iter()
            .filter(|item| **item != req.item)
            .cloned()
            .collect();
        let at = match req.placement {
            PlaylistTreePlacement::Inside => siblings.len(),
            placement => {
                let target = siblings
                    .iter()
                    .position(|item| *item == req.target)
                    .unwrap_or(siblings.len());
                if placement == PlaylistTreePlacement::After {
                    target + 1
                } else {
                    target
                }
            }
        };
        siblings.insert(at.min(siblings.len()), req.item.clone());
        write_children(&tx, parent.as_deref(), &siblings)?;
        if req.placement == PlaylistTreePlacement::Inside {
            expand_folder(&tx, &req.target.id)?;
        }
        tx.commit()?;
        Ok(MovePlaylistTreeItemData { item: req.item })
    }
}

/// Opens a folder something was just put in, so it shows.
fn expand_folder(conn: &rusqlite::Connection, folder_id: &str) -> BackendResult<()> {
    conn.execute(
        "UPDATE playlist_folders SET collapsed = 0 WHERE id = ?1",
        params![folder_id],
    )?;
    Ok(())
}
