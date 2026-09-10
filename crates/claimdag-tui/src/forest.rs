//! The parent tree, and the one line each node draws as.
//!
//! The graph is a DAG over dependencies and a forest over parents. The pane
//! shows the forest, because that is the shape a person navigates, and the
//! dependency count rides along on the label.

use std::collections::{BTreeMap, HashSet};

use claimdag::{WorkId, WorkNode, WorkStatus};

use crate::handles::Handles;

/// One row of the drawn forest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The node this row stands for.
    pub id: WorkId,
    /// How deep under a root it sits.
    pub depth: usize,
    /// The text drawn after the indent.
    pub label: String,
    /// Whether this row is a repeat of an ancestor rather than a node.
    pub cycle: bool,
}

/// The forest: children by parent, and the roots, both in a stable order.
///
/// A node whose parent is not in the set is a root, so filtering the set never
/// hides a node behind a parent that is no longer drawn.
#[must_use]
pub fn parent_tree(nodes: &[&WorkNode]) -> (Vec<WorkId>, BTreeMap<WorkId, Vec<WorkId>>) {
    let present: HashSet<WorkId> = nodes.iter().map(|n| n.id).collect();
    let mut children: BTreeMap<WorkId, Vec<WorkId>> = BTreeMap::new();
    let mut roots: Vec<WorkId> = Vec::new();
    for node in nodes {
        if !node.parent.is_zero() && node.parent != node.id && present.contains(&node.parent) {
            children.entry(node.parent).or_default().push(node.id);
        } else {
            roots.push(node.id);
        }
    }
    for kids in children.values_mut() {
        kids.sort();
    }
    roots.sort();
    (roots, children)
}

/// The label one node draws: status, who holds it, its role, and its summary.
#[must_use]
pub fn label(node: &WorkNode, handles: &Handles) -> String {
    let mut parts: Vec<String> = vec![node.status.as_str().to_string()];
    if !node.assignee.is_zero() {
        parts.push(handles.name_for(node.assignee));
    }
    if node.role.as_str() != "unset" {
        parts.push(node.role.as_str().to_string());
    }
    let summary = node.summary.trim();
    parts.push(if summary.is_empty() {
        node.id.to_hex()[..8].to_string()
    } else {
        summary.to_string()
    });
    let mut line = parts.join("  ");
    if !node.deps.is_empty() {
        line.push_str(&format!("  deps={}", node.deps.len()));
    }
    if node.archived {
        line.push_str("  archived");
    }
    // How long a held node has been quiet. The decision to hand a claim back
    // is a judgement about whether somebody is still on it, and an operator
    // cannot make it from a status that says `claimed` either way.
    if let Some(quiet) = quiet_for(node) {
        line.push_str(&format!("  quiet {quiet}"));
    }
    line
}

/// How long since a held node was touched, in the coarsest unit that is still
/// true, or nothing when the node is not held.
#[must_use]
pub fn quiet_for(node: &WorkNode) -> Option<String> {
    if !matches!(node.status, WorkStatus::Claimed | WorkStatus::Running) {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let seconds = now.saturating_sub(node.updated_unix);
    Some(match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86400),
    })
}

/// Walk the forest into drawable rows, depth first.
///
/// A parent cycle would otherwise recurse forever. It cannot be reached
/// through the graph's own writes, which reject one, but the pane reads a file
/// and a file can say anything.
#[must_use]
pub fn rows(nodes: &[&WorkNode], handles: &Handles) -> Vec<Row> {
    let by_id: BTreeMap<WorkId, &WorkNode> = nodes.iter().map(|n| (n.id, *n)).collect();
    let (roots, children) = parent_tree(nodes);
    let mut out: Vec<Row> = Vec::with_capacity(nodes.len());
    let mut seen: Vec<WorkId> = Vec::new();
    for root in roots {
        walk(root, 0, &by_id, &children, handles, &mut seen, &mut out);
    }
    out
}

fn walk(
    id: WorkId,
    depth: usize,
    by_id: &BTreeMap<WorkId, &WorkNode>,
    children: &BTreeMap<WorkId, Vec<WorkId>>,
    handles: &Handles,
    seen: &mut Vec<WorkId>,
    out: &mut Vec<Row>,
) {
    if seen.contains(&id) {
        out.push(Row {
            id,
            depth,
            label: format!("{}  (cycle)", &id.to_hex()[..8]),
            cycle: true,
        });
        return;
    }
    let Some(node) = by_id.get(&id) else { return };
    out.push(Row {
        id,
        depth,
        label: label(node, handles),
        cycle: false,
    });
    seen.push(id);
    for kid in children.get(&id).map(Vec::as_slice).unwrap_or_default() {
        walk(*kid, depth + 1, by_id, children, handles, seen, out);
    }
    seen.pop();
}

/// The forest as plain text, for `--dump`.
#[must_use]
pub fn dump(nodes: &[&WorkNode], handles: &Handles) -> String {
    let drawn = rows(nodes, handles);
    if drawn.is_empty() {
        return "(empty)\n".to_string();
    }
    let mut out = String::new();
    for row in drawn {
        out.push_str(&"  ".repeat(row.depth));
        out.push_str(&row.label);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use claimdag::{WorkGraph, WorkKind};

    fn fixture() -> WorkGraph {
        let mut graph = WorkGraph::default();
        let root = graph.mint_id(WorkKind::Task, WorkId::ZERO, "land the adapter");
        graph
            .upsert(
                root,
                claimdag::WorkFields {
                    kind: WorkKind::Task,
                    status: claimdag::WorkStatus::Todo,
                    role: claimdag::WorkRole::Unset,
                    parent: WorkId::ZERO,
                    actor: WorkId::ZERO,
                    summary: "land the adapter",
                },
            )
            .unwrap();
        for summary in ["write the tree", "verify the tree"] {
            let kid = graph.mint_id(WorkKind::Task, root, summary);
            graph
                .upsert(
                    kid,
                    claimdag::WorkFields {
                        kind: WorkKind::Task,
                        status: claimdag::WorkStatus::Todo,
                        role: claimdag::WorkRole::Unset,
                        parent: root,
                        actor: WorkId::ZERO,
                        summary,
                    },
                )
                .unwrap();
        }
        graph
    }

    #[test]
    fn children_hang_under_their_parent() {
        let graph = fixture();
        let nodes = graph.list();
        let (roots, children) = parent_tree(&nodes);
        assert_eq!(roots.len(), 1);
        assert_eq!(children[&roots[0]].len(), 2);
    }

    #[test]
    fn the_dump_indents_by_depth() {
        let graph = fixture();
        let text = dump(&graph.list(), &Handles::default());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].starts_with(' '));
        assert!(lines[1].starts_with("  "));
        assert!(lines[2].starts_with("  "));
    }

    #[test]
    fn an_empty_graph_says_so() {
        assert_eq!(dump(&[], &Handles::default()), "(empty)\n");
    }

    /// A node drawn without its parent is a root, not a node that vanishes.
    #[test]
    fn filtering_out_a_parent_promotes_its_children() {
        let graph = fixture();
        let all = graph.list();
        let root = parent_tree(&all).0[0];
        let without_root: Vec<&WorkNode> = all.into_iter().filter(|n| n.id != root).collect();
        let (roots, _) = parent_tree(&without_root);
        assert_eq!(roots.len(), 2);
        assert_eq!(rows(&without_root, &Handles::default()).len(), 2);
    }

    #[test]
    fn the_label_carries_status_summary_and_the_dependency_count() {
        let mut graph = fixture();
        let nodes: Vec<WorkId> = graph.list().iter().map(|n| n.id).collect();
        let (roots, children) = parent_tree(&graph.list());
        let kids = children[&roots[0]].clone();
        graph.link_dep(kids[0], kids[1], WorkId::ZERO).unwrap();
        assert_eq!(nodes.len(), 3);
        let node = graph.get(kids[1]).unwrap();
        let line = label(node, &Handles::default());
        assert!(line.contains("deps=1"), "{line}");
        assert!(line.contains(&node.summary), "{line}");
        assert!(line.starts_with(node.status.as_str()), "{line}");
    }
}
