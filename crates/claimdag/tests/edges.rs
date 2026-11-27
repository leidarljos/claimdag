//! Typed edges.
//!
//! A blocking edge that would close a cycle is refused with
//! [`Fault::Cycle`], and the graph is unchanged.
//! An advisory edge between siblings is stored, and both stay ready.
//! A snapshot the base wrote has no kind field, so every edge loads as hard.

use std::path::PathBuf;

use claimdag::{
    EdgeKind, Fault, WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus, SNAP_BIN,
};

fn actor() -> WorkId {
    WorkId { hi: 9, lo: 9 }
}

fn fields(summary: &str, parent: WorkId) -> WorkFields<'_> {
    WorkFields {
        kind: WorkKind::Task,
        status: WorkStatus::Ready,
        role: WorkRole::Unset,
        parent,
        actor: actor(),
        summary,
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "claimdag-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// P14. Each blocking kind is refused, and the child gains no edge.
#[test]
fn a_blocking_edge_that_would_close_a_cycle_is_refused() {
    let mut graph = WorkGraph::default();
    let first = graph
        .upsert(WorkId::ZERO, fields("first", WorkId::ZERO))
        .unwrap();
    let second = graph
        .upsert(WorkId::ZERO, fields("second", WorkId::ZERO))
        .unwrap();
    graph
        .link_edge(first, second, EdgeKind::Hard, actor())
        .unwrap();
    for kind in [
        EdgeKind::Hard,
        EdgeKind::Needs,
        EdgeKind::Pin,
        EdgeKind::History,
    ] {
        let err = graph.link_edge(second, first, kind, actor()).unwrap_err();
        assert_eq!(err, Fault::Cycle { node: first }, "{kind:?}");
        assert_eq!(
            err.to_string(),
            format!("cycle involving {}", first.to_hex())
        );
        let node = graph.get(first).unwrap();
        assert!(node.deps.is_empty(), "{kind:?} wrote {:?}", node.deps);
        assert!(node.dep_kinds.is_empty());
    }
    let kept = graph.get(second).unwrap();
    assert_eq!(kept.deps, vec![first]);
    assert_eq!(kept.dep_kinds, vec![EdgeKind::Hard]);
    graph.verify().unwrap();
}

/// P15. Siblings with an advisory edge are both still ready, and the kind
/// survives a save.
#[test]
fn an_advisory_edge_between_siblings_is_listed_and_both_stay_ready() {
    let mut graph = WorkGraph::default();
    let parent = graph
        .upsert(WorkId::ZERO, fields("parent", WorkId::ZERO))
        .unwrap();
    let left = graph.upsert(WorkId::ZERO, fields("left", parent)).unwrap();
    let right = graph.upsert(WorkId::ZERO, fields("right", parent)).unwrap();
    graph
        .link_edge(left, right, EdgeKind::Advisory, actor())
        .unwrap();
    let node = graph.get(right).unwrap();
    assert_eq!(node.deps, vec![left]);
    assert_eq!(node.dep_kinds, vec![EdgeKind::Advisory]);
    let ready: Vec<WorkId> = graph.ready_view().iter().map(|n| n.id).collect();
    assert!(ready.contains(&left), "{ready:?}");
    assert!(ready.contains(&right), "{ready:?}");

    let dir = scratch("advisory");
    graph.save_dir(&dir).unwrap();
    let loaded = WorkGraph::open_dir(&dir).unwrap();
    let again = loaded.get(right).unwrap();
    assert_eq!(again.deps, vec![left]);
    assert_eq!(again.dep_kinds, vec![EdgeKind::Advisory]);
    let ready: Vec<WorkId> = loaded.ready_view().iter().map(|n| n.id).collect();
    assert!(ready.contains(&left) && ready.contains(&right), "{ready:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The base wrote this file with no `depKinds` field. Every edge is hard,
/// and saving it again writes those same bytes.
#[test]
fn a_snapshot_written_by_the_base_loads_edges_as_hard() {
    const SNAPSHOT: &[u8] = include_bytes!("fixtures/work-base.bin");
    let dir = scratch("base-in");
    std::fs::write(dir.join(SNAP_BIN), SNAPSHOT).unwrap();
    let graph = WorkGraph::open_dir(&dir).expect("open the base snapshot");
    let mut edges = 0;
    for node in graph.list() {
        assert_eq!(node.dep_kinds.len(), node.deps.len());
        for kind in &node.dep_kinds {
            assert_eq!(*kind, EdgeKind::Hard);
        }
        edges += node.deps.len();
    }
    assert!(edges >= 1, "the base snapshot has no edge");
    let to = scratch("base-out");
    graph.save_dir(&to).unwrap();
    let saved = std::fs::read(to.join(SNAP_BIN)).unwrap();
    assert_eq!(saved, SNAPSHOT);
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&to).unwrap();
}
