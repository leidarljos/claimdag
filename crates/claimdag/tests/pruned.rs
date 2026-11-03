//! Prune drops finished nodes to stay under the cap. A seat coming back to
//! one later is told it finished, not that it never existed.

use std::path::PathBuf;

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};

/// No cap inside the budget in measurements/node-cap.org comes near this
/// many nodes, so a graph that has pruned nothing by then has no cap.
const NO_CAP: usize = 1 << 16;

fn seat() -> WorkId {
    WorkId { hi: 7, lo: 7 }
}

fn fields(summary: &str) -> WorkFields<'_> {
    WorkFields {
        kind: WorkKind::Task,
        status: WorkStatus::Ready,
        role: WorkRole::Unset,
        parent: WorkId::ZERO,
        actor: seat(),
        summary,
    }
}

fn empty_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("claimdag-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Finish one node as `status`, then take on live work until prune drops
/// it, which leaves the graph at the cap. Returns the dropped node.
fn finished_then_pruned(graph: &mut WorkGraph, status: WorkStatus) -> WorkId {
    let first = graph
        .upsert(WorkId::ZERO, fields("finished first"))
        .expect("upsert");
    graph
        .complete(first, status, "", seat(), None)
        .expect("complete");
    let mut made = 1;
    while graph.get(first).is_some() {
        assert!(made < NO_CAP, "nothing was pruned past {made} nodes");
        graph
            .upsert(WorkId::ZERO, fields(&format!("live {made}")))
            .expect("upsert");
        made += 1;
    }
    first
}

#[test]
fn a_pruned_node_is_answered_for_after_a_reload_until_it_is_taken_on_again() {
    let dir = empty_dir("pruned");
    let mut graph = WorkGraph::default();
    let first = finished_then_pruned(&mut graph, WorkStatus::Cancelled);
    graph.save_dir(&dir).expect("save");

    let mut graph = WorkGraph::load_dir(&dir);
    assert_eq!(graph.pruned(first), Some(WorkStatus::Cancelled));
    graph
        .complete(first, WorkStatus::Done, "again", seat(), Some(9))
        .expect("completing a pruned node");
    assert_eq!(graph.release(first, seat()), Ok(0));
    assert_eq!(
        graph.claim(first, seat(), None),
        Err("claim: status cancelled, pruned".to_string())
    );
    assert!(graph.get(first).is_none(), "an answer brought it back");

    let never = WorkId { hi: 1, lo: 2 };
    assert_eq!(graph.pruned(never), None);
    assert!(graph
        .release(never, seat())
        .is_err_and(|why| why.starts_with("release: unknown id")));

    graph
        .upsert(first, fields("taken on again"))
        .expect("upsert");
    assert_eq!(graph.pruned(first), None);
    graph
        .claim(first, seat(), None)
        .expect("claim the live node");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compacting_the_list_keeps_the_newest_pruned_nodes() {
    let dir = empty_dir("pruned-compact");
    let mut graph = WorkGraph::default();
    finished_then_pruned(&mut graph, WorkStatus::Done);
    let cap = graph.list().len() as u64;
    graph.save_dir(&dir).expect("save");
    // pruned.log at twice the cap, oldest first, so the next drop rewrites it.
    let named = |n: u64| WorkId { hi: 1, lo: n };
    let lines: String = (1..=2 * cap)
        .map(|n| format!("{} done\n", named(n).to_hex()))
        .collect();
    std::fs::write(dir.join("pruned.log"), lines).unwrap();

    let mut graph = WorkGraph::load_dir(&dir);
    let last = graph
        .upsert(WorkId::ZERO, fields("finished last"))
        .expect("upsert");
    graph
        .complete(last, WorkStatus::Failed, "", seat(), None)
        .expect("complete");
    graph
        .upsert(WorkId::ZERO, fields("one more"))
        .expect("upsert");
    assert!(
        graph.get(last).is_none(),
        "prune kept the only finished node"
    );
    graph.save_dir(&dir).expect("save");

    let graph = WorkGraph::load_dir(&dir);
    assert_eq!(graph.pruned(last), Some(WorkStatus::Failed));
    assert_eq!(graph.pruned(named(2 * cap)), Some(WorkStatus::Done));
    assert_eq!(
        graph.pruned(named(1)),
        None,
        "the list grew past twice the cap"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
