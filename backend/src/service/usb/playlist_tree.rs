//! The USB view's playlist tree: the stick's folders and playlists, each
//! folder's items (folders and playlists together) in one order, as rekordbox
//! numbers them in both databases. Built once per read of the stick
//! ([`build_usb_tree`], kept with the resolved playlists); the view's list,
//! Edit order's moves and the save all order a folder's items with
//! [`children_in_order`], so what the view shows is what gets written.
//!
//! Items are matched across the databases by path, as the playlists are: a
//! folder's id is `usb-f-` and its path's canonical segments, so the PDB's
//! and the eDB's rows of one folder are one item.

use super::order_edit::apply_pending_order;
use super::text::canonicalize_playlist_name;
use crate::models::{PlaylistTreeKind, UsbPlaylistTreeRow};
use crate::pdb_reader::PlaylistPath;
use std::collections::{HashMap, HashSet};

/// The parent of the items at the top level.
pub(crate) const TOP: &str = "";

/// A folder or a playlist of the stick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsbTreeNode {
    pub(crate) kind: PlaylistTreeKind,
    /// A playlist's `UsbPlaylist::id`, or [`folder_id`].
    pub(crate) id: String,
    /// The folder it is in, or [`TOP`].
    pub(crate) parent: String,
    /// Its own name.
    pub(crate) name: String,
    /// Its place among its folder's items on the stick.
    pub(crate) sort_order: u32,
}

/// What every folder id starts with.
pub(crate) const FOLDER_PREFIX: &str = "usb-f-";

/// The id of the folder at `segments` (its parent folders, then itself).
pub(crate) fn folder_id(segments: &[String]) -> String {
    let canonical: Vec<String> = segments
        .iter()
        .map(|segment| canonicalize_playlist_name(segment))
        .collect();
    format!("{FOLDER_PREFIX}{}", canonical.join("/"))
}

/// The id of the folder a playlist or folder at `parents` is in.
fn parent_id(parents: &[String]) -> String {
    if parents.is_empty() {
        TOP.to_string()
    } else {
        folder_id(parents)
    }
}

/// The tree of `folders` (each one's path and place, the PDB's first: the
/// first of one path wins) and `playlists` (`UsbPlaylist::id`, path, place).
/// A folder a playlist's path names that neither database lists is added,
/// after its folder's other items.
pub(crate) fn build_usb_tree(
    folders: impl IntoIterator<Item = (PlaylistPath, u32)>,
    playlists: impl IntoIterator<Item = (String, PlaylistPath, u32)>,
) -> Vec<UsbTreeNode> {
    let mut nodes = Vec::new();
    let mut known = HashSet::new();
    let mut add_folder = |nodes: &mut Vec<UsbTreeNode>, segments: &[String], sort_order: u32| {
        let Some((name, parents)) = segments.split_last() else {
            return;
        };
        let id = folder_id(segments);
        if known.insert(id.clone()) {
            nodes.push(UsbTreeNode {
                kind: PlaylistTreeKind::Folder,
                id,
                parent: parent_id(parents),
                name: name.clone(),
                sort_order,
            });
        }
    };
    let listed: Vec<Vec<String>> = folders
        .into_iter()
        .map(|(path, sort_order)| {
            let mut segments = path.folders;
            segments.push(path.name);
            add_folder(&mut nodes, &segments, sort_order);
            segments
        })
        .collect();
    // A listed folder's parents, when only it is listed.
    for segments in &listed {
        for depth in 1..segments.len() {
            add_folder(&mut nodes, &segments[..depth], u32::MAX);
        }
    }
    for (id, path, sort_order) in playlists {
        for depth in 1..=path.folders.len() {
            add_folder(&mut nodes, &path.folders[..depth], u32::MAX);
        }
        nodes.push(UsbTreeNode {
            kind: PlaylistTreeKind::Playlist,
            id,
            parent: parent_id(&path.folders),
            name: path.name,
            sort_order,
        });
    }
    nodes
}

/// `parent`'s items in order: the stick's (place, then name), or the pending
/// order's for that folder (`pending`, keyed by folder id).
pub(crate) fn children_in_order<'a>(
    nodes: &'a [UsbTreeNode],
    parent: &str,
    pending: &HashMap<String, Vec<String>>,
) -> Vec<&'a UsbTreeNode> {
    let mut children: Vec<&UsbTreeNode> = nodes.iter().filter(|n| n.parent == parent).collect();
    children.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    if let Some(order) = pending.get(parent) {
        apply_pending_order(&mut children, order, |node| node.id.clone());
    }
    children
}

/// The whole tree, top first, each folder's items after it, with each one's
/// depth.
pub(crate) fn walk<'a>(
    nodes: &'a [UsbTreeNode],
    pending: &HashMap<String, Vec<String>>,
) -> Vec<(&'a UsbTreeNode, usize)> {
    fn visit<'a>(
        nodes: &'a [UsbTreeNode],
        parent: &str,
        depth: usize,
        pending: &HashMap<String, Vec<String>>,
        seen: &mut HashSet<&'a str>,
        out: &mut Vec<(&'a UsbTreeNode, usize)>,
    ) {
        for node in children_in_order(nodes, parent, pending) {
            if !seen.insert(node.id.as_str()) {
                continue;
            }
            out.push((node, depth));
            if node.kind == PlaylistTreeKind::Folder {
                visit(nodes, &node.id, depth + 1, pending, seen, out);
            }
        }
    }
    let mut out = Vec::with_capacity(nodes.len());
    visit(nodes, TOP, 0, pending, &mut HashSet::new(), &mut out);
    out
}

/// The view's rows for [`walk`]'s order.
pub(crate) fn tree_rows(walked: &[(&UsbTreeNode, usize)]) -> Vec<UsbPlaylistTreeRow> {
    walked
        .iter()
        .map(|(node, depth)| UsbPlaylistTreeRow {
            kind: node.kind,
            id: node.id.clone(),
            parent_id: node.parent.clone(),
            depth: *depth,
            name: node.name.clone(),
        })
        .collect()
}

/// The folder item `id` is in, when the tree has it.
pub(crate) fn parent_of<'a>(nodes: &'a [UsbTreeNode], id: &str) -> Option<&'a str> {
    nodes
        .iter()
        .find(|node| node.id == id)
        .map(|node| node.parent.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(folders: &[&str], name: &str) -> PlaylistPath {
        PlaylistPath {
            folders: folders.iter().map(|f| f.to_string()).collect(),
            name: name.to_string(),
        }
    }

    fn lines(nodes: &[UsbTreeNode], pending: &HashMap<String, Vec<String>>) -> Vec<String> {
        walk(nodes, pending)
            .into_iter()
            .map(|(node, depth)| format!("{}{}", "  ".repeat(depth), node.name))
            .collect()
    }

    #[test]
    fn folders_and_playlists_share_one_order_per_folder() {
        let nodes = build_usb_tree(
            [(path(&[], "Sets"), 1), (path(&["Sets"], "Late"), 0)],
            [
                ("p-top".to_string(), path(&[], "Top"), 0),
                ("p-warm".to_string(), path(&["Sets"], "Warm"), 1),
                ("p-deep".to_string(), path(&["Sets", "Late"], "Deep"), 0),
                // Its folder is listed by neither database: added last.
                ("p-odd".to_string(), path(&["Loose"], "Odd"), 0),
            ],
        );
        let none = HashMap::new();
        assert_eq!(
            lines(&nodes, &none),
            [
                "Top", "Sets", "  Late", "    Deep", "  Warm", "Loose", "  Odd"
            ]
        );
        // A folder's pending order moves its items only.
        let pending =
            HashMap::from([(folder_id(&["Sets".to_string()]), vec!["p-warm".to_string()])]);
        assert_eq!(
            lines(&nodes, &pending),
            [
                "Top", "Sets", "  Warm", "  Late", "    Deep", "Loose", "  Odd"
            ]
        );
        assert_eq!(parent_of(&nodes, "p-deep"), Some("usb-f-sets/late"));
        assert_eq!(parent_of(&nodes, "usb-f-sets"), Some(TOP));
    }
}
