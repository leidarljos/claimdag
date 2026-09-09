//! What the graph promises, probed from outside the crate.
//!
//! The rules in the README are claims about what cannot happen: a claim is
//! CAS, terminal is sticky, a dependency is a hard one. These ask whether the
//! paths that do not enforce a rule can be used to get around the paths that
//! do.

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};

fn actor(n: u64) -> WorkId {
    WorkId { hi: n, lo: n }
}

/// A node with no dependencies, ready to claim.
fn ready(graph: &mut WorkGraph, summary: &str) -> WorkId {
    graph
        .upsert(
            WorkId::ZERO,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Implementor,
                parent: WorkId::ZERO,
                actor: actor(1),
                summary,
            },
        )
        .expect("upsert")
}

#[test]
fn a_live_claim_survives_an_upsert() {
    // upsert owns metadata and the three open statuses; claim owns the live
    // ones. If upsert can put a claimed node back to Todo, one agent can take
    // work another is holding without the claim ever failing.
    let mut graph = WorkGraph::default();
    let node = ready(&mut graph, "the work");
    let holder = actor(7);
    graph.claim(node, holder, None).expect("claim");

    let demoted = graph.upsert(
        node,
        WorkFields {
            kind: WorkKind::Task,
            status: WorkStatus::Todo,
            role: WorkRole::Implementor,
            parent: WorkId::ZERO,
            actor: actor(1),
            summary: "",
        },
    );
    assert!(
        demoted.is_err(),
        "upsert put a claimed node back to Todo: {:?}",
        graph.get(node).map(|n| (n.status, n.assignee))
    );
}

#[test]
fn only_the_assignee_starts_the_work() {
    // complete refuses a stranger. set_running is the same access question and
    // should answer the same way, or the ledger records the wrong agent as
    // having started somebody else's node.
    let mut graph = WorkGraph::default();
    let node = ready(&mut graph, "the work");
    let holder = actor(7);
    let stranger = actor(9);
    graph.claim(node, holder, None).expect("claim");

    assert!(
        graph.set_running(node, stranger).is_err(),
        "a stranger marked another agent's claim as running"
    );
    graph.set_running(node, holder).expect("the holder may");
}

#[test]
fn a_terminal_summary_is_not_rewritable_by_a_stranger() {
    // complete returns early for a terminal node and rewrites the summary
    // before it ever reaches the assignee check.
    let mut graph = WorkGraph::default();
    let node = ready(&mut graph, "the work");
    let holder = actor(7);
    graph.claim(node, holder, None).expect("claim");
    graph
        .complete(node, WorkStatus::Done, "what happened", holder)
        .expect("complete");

    let rewritten = graph.complete(node, WorkStatus::Done, "something else", actor(9));
    let summary = graph
        .get(node)
        .map(|n| n.summary.clone())
        .unwrap_or_default();
    assert!(
        rewritten.is_err() || summary == "what happened",
        "a stranger rewrote a finished node's summary to {summary:?}"
    );
}

#[test]
fn ready_means_the_dependencies_are_done() {
    // Status is what a reader trusts to decide what is workable. Setting it
    // directly past an unsatisfied dependency makes it a lie, and claim
    // rejecting the claim later does not help the reader who believed it.
    let mut graph = WorkGraph::default();
    let blocker = ready(&mut graph, "first");
    let blocked = ready(&mut graph, "second");
    graph.link_dep(blocker, blocked, actor(1)).expect("link");
    assert_eq!(
        graph.get(blocked).map(|n| n.status),
        Some(WorkStatus::Todo),
        "linking an unfinished dependency should take it out of Ready"
    );

    let forged = graph.upsert(
        blocked,
        WorkFields {
            kind: WorkKind::Task,
            status: WorkStatus::Ready,
            role: WorkRole::Implementor,
            parent: WorkId::ZERO,
            actor: actor(1),
            summary: "",
        },
    );
    let status = graph.get(blocked).map(|n| n.status);
    assert!(
        forged.is_err() || status != Some(WorkStatus::Ready),
        "a node with an unfinished dependency reports {status:?}"
    );
}

#[test]
fn a_cycle_cannot_be_built_by_any_route() {
    // The direct case has a test. This is the long way round.
    let mut graph = WorkGraph::default();
    let a = ready(&mut graph, "a");
    let b = ready(&mut graph, "b");
    let c = ready(&mut graph, "c");
    graph.link_dep(a, b, actor(1)).expect("b depends on a");
    graph.link_dep(b, c, actor(1)).expect("c depends on b");
    assert!(
        graph.link_dep(c, a, actor(1)).is_err(),
        "closed a three-node loop"
    );
    graph.verify().expect("the graph still verifies");
}

#[test]
fn completing_a_dependency_readies_what_waited_on_it() {
    let mut graph = WorkGraph::default();
    let blocker = ready(&mut graph, "first");
    let blocked = ready(&mut graph, "second");
    graph.link_dep(blocker, blocked, actor(1)).expect("link");
    let holder = actor(7);
    graph.claim(blocker, holder, None).expect("claim");
    graph
        .complete(blocker, WorkStatus::Done, "", holder)
        .expect("complete");
    assert_eq!(
        graph.get(blocked).map(|n| n.status),
        Some(WorkStatus::Ready),
        "the dependent was not promoted"
    );
}

#[test]
fn a_failed_dependency_does_not_ready_what_waited_on_it() {
    // Done is the only terminal that satisfies a dependency: work that waited
    // on something which failed is not workable, it is stuck.
    let mut graph = WorkGraph::default();
    let blocker = ready(&mut graph, "first");
    let blocked = ready(&mut graph, "second");
    graph.link_dep(blocker, blocked, actor(1)).expect("link");
    let holder = actor(7);
    graph.claim(blocker, holder, None).expect("claim");
    graph
        .complete(blocker, WorkStatus::Failed, "", holder)
        .expect("complete");
    assert_eq!(
        graph.get(blocked).map(|n| n.status),
        Some(WorkStatus::Todo),
        "a failed dependency readied its dependent"
    );
    assert!(graph.claim(blocked, actor(8), None).is_err());
}

/// The parent chain is a forest. A dependency edge is refused when it would
/// close a loop, and the parent edge has to be refused for the same reason:
/// every reader that draws the tree, or walks up from a node to find its root,
/// runs forever on a chain that comes back to where it started.
#[test]
fn a_node_cannot_become_its_own_ancestor() {
    let mut graph = WorkGraph::default();
    let root = ready(&mut graph, "land the adapter");
    let kid = graph
        .upsert(
            WorkId::ZERO,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent: root,
                actor: actor(1),
                summary: "write the tree",
            },
        )
        .expect("child");

    let point_at = |graph: &mut WorkGraph, id: WorkId, parent: WorkId| {
        graph.upsert(
            id,
            WorkFields {
                kind: WorkKind::Unset,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent,
                actor: actor(1),
                summary: "",
            },
        )
    };

    assert!(point_at(&mut graph, root, root).is_err(), "own parent");
    assert!(point_at(&mut graph, root, kid).is_err(), "two-node loop");
    assert_eq!(graph.get(root).expect("root").parent, WorkId::ZERO);
    assert_eq!(graph.get(kid).expect("kid").parent, root);
    graph.verify().expect("still a forest");
}

/// Reparenting sideways is ordinary and must keep working: the check refuses a
/// loop, not a move.
#[test]
fn a_node_can_be_moved_under_a_different_parent() {
    let mut graph = WorkGraph::default();
    let first = ready(&mut graph, "land the adapter");
    let second = ready(&mut graph, "write the tree");
    let leaf = ready(&mut graph, "verify the tree");

    let reparent = |graph: &mut WorkGraph, id: WorkId, parent: WorkId| {
        graph
            .upsert(
                id,
                WorkFields {
                    kind: WorkKind::Unset,
                    status: WorkStatus::Todo,
                    role: WorkRole::Unset,
                    parent,
                    actor: actor(1),
                    summary: "",
                },
            )
            .expect("reparent");
    };

    reparent(&mut graph, leaf, first);
    assert_eq!(graph.get(leaf).expect("leaf").parent, first);
    reparent(&mut graph, leaf, second);
    assert_eq!(graph.get(leaf).expect("leaf").parent, second);
    graph.verify().expect("still a forest");
}
