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

use super::order_edit::{apply_pending_order, move_before};
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

/// Edit order's pending changes to the tree. Every id is the stick's
/// ([`UsbTreeNode::id`]), for the whole edit: a moved folder keeps its id, so
/// what it holds moves with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PendingTree {
    /// Per folder ([`TOP`] for the top level) whose items changed, its items'
    /// ids (playlists and folders) in the new order. A folder back in the
    /// stick's order has none.
    pub(crate) order: HashMap<String, Vec<String>>,
    /// Per item moved to another folder, that folder's id ([`TOP`]: the top
    /// level). An item back in its stick folder has none.
    pub(crate) parents: HashMap<String, String>,
}

impl PendingTree {
    pub(crate) fn is_empty(&self) -> bool {
        self.order.is_empty() && self.parents.is_empty()
    }

    /// The folder `node` is in: its pending one, else the stick's.
    pub(crate) fn parent_of_node<'a>(&'a self, node: &'a UsbTreeNode) -> &'a str {
        self.parents
            .get(&node.id)
            .map_or(node.parent.as_str(), String::as_str)
    }

    /// This tree with item `id` moved to `placement`, possibly into another
    /// folder. The folder it lands in, and the one it left, get their whole
    /// order, so the save numbers each one from 0 with no gaps. Refused for a
    /// folder into itself or what it holds, and for a name its new folder
    /// already has (a playlist or a folder: both databases pair items by
    /// their path).
    pub(crate) fn moved(
        &self,
        nodes: &[UsbTreeNode],
        id: &str,
        placement: Placement<'_>,
    ) -> Result<PendingTree, MoveRefusal> {
        let node = |id: &str| {
            nodes
                .iter()
                .find(|node| node.id == id)
                .ok_or_else(|| MoveRefusal::NotFound(id.to_string()))
        };
        let item = node(id)?;
        let from = self.parent_of_node(item).to_string();
        let to = match placement {
            Placement::Before(target) | Placement::After(target) => {
                self.parent_of_node(node(target)?).to_string()
            }
            Placement::Inside(folder) => {
                let folder = node(folder)?;
                if folder.kind != PlaylistTreeKind::Folder {
                    return Err(MoveRefusal::NotAFolder);
                }
                folder.id.clone()
            }
            Placement::End => from.clone(),
        };
        if to != from {
            if is_in_folder(nodes, &to, id, self) {
                return Err(MoveRefusal::IntoItself);
            }
            let name = canonicalize_playlist_name(&item.name);
            if let Some(taken) = children_in_order(nodes, &to, self)
                .into_iter()
                .find(|node| canonicalize_playlist_name(&node.name) == name)
            {
                return Err(MoveRefusal::NameTaken(taken.name.clone()));
            }
        }

        let mut tree = self.clone();
        if item.parent == to {
            tree.parents.remove(id);
        } else {
            tree.parents.insert(id.to_string(), to.clone());
        }
        let ids = |folder: &str, tree: &PendingTree| -> Vec<String> {
            children_in_order(nodes, folder, tree)
                .into_iter()
                .map(|node| node.id.clone())
                .collect()
        };
        let mut order = ids(&to, &tree);
        let position = |order: &[String], id: &str| {
            order
                .iter()
                .position(|other| other == id)
                .ok_or_else(|| MoveRefusal::NotFound(id.to_string()))
        };
        let at = position(&order, id)?;
        let before = match placement {
            Placement::Before(target) => Some(position(&order, target)?),
            Placement::After(target) => {
                let next = position(&order, target)? + 1;
                (next < order.len()).then_some(next)
            }
            Placement::Inside(_) | Placement::End => None,
        };
        move_before(&mut order, at, before);
        tree.order.insert(to.clone(), order);
        if from != to {
            let rest = ids(&from, &tree);
            tree.order.insert(from, rest);
        }
        tree.drop_unchanged_orders(nodes);
        Ok(tree)
    }

    /// Drops each folder's order that is the stick's again, unless an item
    /// moved into or out of that folder: those keep theirs, so the save
    /// renumbers them.
    fn drop_unchanged_orders(&mut self, nodes: &[UsbTreeNode]) {
        let touched: HashSet<&str> = self
            .parents
            .iter()
            .flat_map(|(id, to)| {
                let from = nodes
                    .iter()
                    .find(|node| node.id == *id)
                    .map(|node| node.parent.as_str());
                [Some(to.as_str()), from].into_iter().flatten()
            })
            .collect();
        let stick_order = PendingTree {
            order: HashMap::new(),
            parents: self.parents.clone(),
        };
        let unchanged: Vec<String> = self
            .order
            .iter()
            .filter(|(folder, order)| {
                !touched.contains(folder.as_str())
                    && children_in_order(nodes, folder, &stick_order)
                        .iter()
                        .map(|node| &node.id)
                        .eq(order.iter())
            })
            .map(|(folder, _)| folder.clone())
            .collect();
        for folder in unchanged {
            self.order.remove(&folder);
        }
    }
}

/// Where [`PendingTree::moved`] puts an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement<'a> {
    /// Just before this item, in its folder.
    Before(&'a str),
    /// Just after this item, in its folder.
    After(&'a str),
    /// At the end of this folder.
    Inside(&'a str),
    /// At the end of its own folder.
    End,
}

/// Why [`PendingTree::moved`] refused a move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MoveRefusal {
    /// The tree has no item with this id.
    NotFound(String),
    /// "Inside" a playlist.
    NotAFolder,
    /// A folder into itself or something it holds.
    IntoItself,
    /// The new folder already has an item of this name.
    NameTaken(String),
}

/// Whether `id` is `folder` or something it holds, in the pending tree.
fn is_in_folder(nodes: &[UsbTreeNode], id: &str, folder: &str, pending: &PendingTree) -> bool {
    let mut current = id;
    // One step per node at most: a damaged stick tree could loop.
    for _ in 0..=nodes.len() {
        if current == folder {
            return true;
        }
        match parent_of(nodes, current, pending) {
            Some(parent) if parent != TOP => current = parent,
            _ => return false,
        }
    }
    false
}

/// `parent`'s items in order: the stick's (place, then name), or the pending
/// order's for that folder. An item is `parent`'s when it is there in the
/// pending tree.
pub(crate) fn children_in_order<'a>(
    nodes: &'a [UsbTreeNode],
    parent: &str,
    pending: &PendingTree,
) -> Vec<&'a UsbTreeNode> {
    let mut children: Vec<&UsbTreeNode> = nodes
        .iter()
        .filter(|node| pending.parent_of_node(node) == parent)
        .collect();
    children.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    if let Some(order) = pending.order.get(parent) {
        apply_pending_order(&mut children, order, |node| node.id.clone());
    }
    children
}

/// The whole tree, top first, each folder's items after it, with each one's
/// depth.
pub(crate) fn walk<'a>(
    nodes: &'a [UsbTreeNode],
    pending: &PendingTree,
) -> Vec<(&'a UsbTreeNode, usize)> {
    fn visit<'a>(
        nodes: &'a [UsbTreeNode],
        parent: &str,
        depth: usize,
        pending: &PendingTree,
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

/// The view's rows for [`walk`]'s order, each with its folder in the
/// pending tree.
pub(crate) fn tree_rows(
    walked: &[(&UsbTreeNode, usize)],
    pending: &PendingTree,
) -> Vec<UsbPlaylistTreeRow> {
    walked
        .iter()
        .map(|(node, depth)| UsbPlaylistTreeRow {
            kind: node.kind,
            id: node.id.clone(),
            parent_id: pending.parent_of_node(node).to_string(),
            depth: *depth,
            name: node.name.clone(),
        })
        .collect()
}

/// The folder item `id` is in, in the pending tree, when the tree has it.
pub(crate) fn parent_of<'a>(
    nodes: &'a [UsbTreeNode],
    id: &str,
    pending: &'a PendingTree,
) -> Option<&'a str> {
    nodes
        .iter()
        .find(|node| node.id == id)
        .map(|node| pending.parent_of_node(node))
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

    fn lines(nodes: &[UsbTreeNode], pending: &PendingTree) -> Vec<String> {
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
        let none = PendingTree::default();
        assert_eq!(
            lines(&nodes, &none),
            [
                "Top", "Sets", "  Late", "    Deep", "  Warm", "Loose", "  Odd"
            ]
        );
        // A folder's pending order moves its items only.
        let pending = PendingTree {
            order: HashMap::from([(folder_id(&["Sets".to_string()]), vec!["p-warm".to_string()])]),
            ..PendingTree::default()
        };
        assert_eq!(
            lines(&nodes, &pending),
            [
                "Top", "Sets", "  Warm", "  Late", "    Deep", "Loose", "  Odd"
            ]
        );
        assert_eq!(parent_of(&nodes, "p-deep", &none), Some("usb-f-sets/late"));
        assert_eq!(parent_of(&nodes, "usb-f-sets", &none), Some(TOP));
    }

    #[test]
    fn a_moved_folder_takes_what_it_holds_along() {
        let nodes = build_usb_tree(
            [
                (path(&[], "Sets"), 1),
                (path(&["Sets"], "Late"), 0),
                (path(&[], "Old"), 2),
            ],
            [
                ("p-top".to_string(), path(&[], "Top"), 0),
                ("p-deep".to_string(), path(&["Sets", "Late"], "Deep"), 0),
                ("p-warm".to_string(), path(&["Sets"], "Warm"), 1),
            ],
        );
        let late = folder_id(&["Sets".to_string(), "Late".to_string()]);
        let old = folder_id(&["Old".to_string()]);
        // "Late" goes into "Old"; "Top" into "Sets", first.
        let sets = folder_id(&["Sets".to_string()]);
        let pending = PendingTree {
            order: HashMap::from([
                (
                    sets.clone(),
                    vec!["p-top".to_string(), "p-warm".to_string()],
                ),
                (TOP.to_string(), vec![sets.clone(), old.clone()]),
                (old.clone(), vec![late.clone()]),
            ]),
            parents: HashMap::from([
                (late.clone(), old.clone()),
                ("p-top".to_string(), sets.clone()),
            ]),
        };
        assert_eq!(
            lines(&nodes, &pending),
            ["Sets", "  Top", "  Warm", "Old", "  Late", "    Deep"]
        );
        // Ids stay the stick's; the folder an item is in is the pending one.
        assert_eq!(parent_of(&nodes, &late, &pending), Some(old.as_str()));
        assert_eq!(parent_of(&nodes, "p-deep", &pending), Some(late.as_str()));
        let rows = tree_rows(&walk(&nodes, &pending), &pending);
        let top = rows.iter().find(|row| row.id == "p-top").expect("Top row");
        assert_eq!((top.parent_id.as_str(), top.depth), (sets.as_str(), 1));
    }

    /// Top-level "Top" and "Sets" (holding "Warm" and "Late", which holds
    /// "Deep"), then "Old" (holding "Gold").
    fn move_tree() -> Vec<UsbTreeNode> {
        build_usb_tree(
            [
                (path(&[], "Sets"), 1),
                (path(&["Sets"], "Late"), 1),
                (path(&[], "Old"), 2),
            ],
            [
                ("p-top".to_string(), path(&[], "Top"), 0),
                ("p-warm".to_string(), path(&["Sets"], "Warm"), 0),
                ("p-deep".to_string(), path(&["Sets", "Late"], "Deep"), 0),
                ("p-gold".to_string(), path(&["Old"], "Gold"), 0),
            ],
        )
    }

    fn id(segments: &[&str]) -> String {
        folder_id(&segments.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_move_into_another_folder_renumbers_both_folders() {
        let nodes = move_tree();
        let tree = PendingTree::default()
            .moved(&nodes, "p-top", Placement::Inside(&id(&["Old"])))
            .expect("into Old");
        assert_eq!(
            lines(&nodes, &tree),
            [
                "Sets", "  Warm", "  Late", "    Deep", "Old", "  Gold", "  Top"
            ]
        );
        // Both folders keep their whole order, though the top level's is the
        // stick's relative order: the save numbers each from 0.
        assert_eq!(tree.order[TOP], [id(&["Sets"]), id(&["Old"])]);
        assert_eq!(tree.order[&id(&["Old"])], ["p-gold", "p-top"]);
        assert_eq!(tree.parents["p-top"], id(&["Old"]));

        // Before an item of another folder: into that folder, at that place.
        let tree = tree
            .moved(&nodes, "p-warm", Placement::Before("p-gold"))
            .expect("before Gold");
        assert_eq!(
            lines(&nodes, &tree),
            [
                "Sets", "  Late", "    Deep", "Old", "  Warm", "  Gold", "  Top"
            ]
        );
    }

    #[test]
    fn a_folder_moves_with_what_it_holds() {
        let nodes = move_tree();
        let tree = PendingTree::default()
            .moved(&nodes, &id(&["Sets", "Late"]), Placement::After("p-gold"))
            .expect("Late after Gold");
        assert_eq!(
            lines(&nodes, &tree),
            [
                "Top", "Sets", "  Warm", "Old", "  Gold", "  Late", "    Deep"
            ]
        );
        // Only the folder itself has a pending parent.
        assert_eq!(tree.parents.len(), 1);
    }

    #[test]
    fn moving_back_leaves_nothing_pending() {
        let nodes = move_tree();
        let tree = PendingTree::default()
            .moved(&nodes, "p-top", Placement::Inside(&id(&["Old"])))
            .expect("into Old");
        let back = tree
            .moved(&nodes, "p-top", Placement::Before(&id(&["Sets"])))
            .expect("back to the top");
        assert!(back.is_empty(), "{back:?}");
    }

    #[test]
    fn refused_moves() {
        let nodes = move_tree();
        let none = PendingTree::default();
        let sets = id(&["Sets"]);
        let late = id(&["Sets", "Late"]);
        // Into itself, or into a folder it holds.
        for into in [&sets, &late] {
            assert_eq!(
                none.moved(&nodes, &sets, Placement::Inside(into)),
                Err(MoveRefusal::IntoItself)
            );
        }
        assert_eq!(
            none.moved(&nodes, &sets, Placement::Before("p-deep")),
            Err(MoveRefusal::IntoItself)
        );
        assert_eq!(
            none.moved(&nodes, "p-top", Placement::Inside("p-warm")),
            Err(MoveRefusal::NotAFolder)
        );
        assert_eq!(
            none.moved(&nodes, "p-nope", Placement::End),
            Err(MoveRefusal::NotFound("p-nope".to_string()))
        );

        // A name the new folder has, as playlists compare names, whether a
        // playlist or a folder has it.
        let clash = build_usb_tree(
            [(path(&[], "A"), 0), (path(&["A"], "Warm"), 0)],
            [
                ("p-gold".to_string(), path(&["A"], "Gold"), 1),
                ("p-gold2".to_string(), path(&[], "gold!"), 1),
                ("p-warm".to_string(), path(&[], "WARM"), 2),
            ],
        );
        for item in ["p-gold2", "p-warm"] {
            assert!(matches!(
                none.moved(&clash, item, Placement::Inside(&id(&["A"]))),
                Err(MoveRefusal::NameTaken(_))
            ));
        }
        // Within its own folder a name is no clash.
        assert!(none.moved(&clash, "p-warm", Placement::End).is_ok());
    }
}
