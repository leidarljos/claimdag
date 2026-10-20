//! Writes a work graph with claimdag 0.7.1 and capnp 0.20, and one line per
//! node beside it. The graph has every kind, status and role, a parent, a
//! dependency chain, a live claim, a running node and an archived one.

use std::path::{Path, PathBuf};

use claimdag::{
    WorkFields, WorkGraph, WorkId, WorkKind, WorkNode, WorkRole, WorkStatus, SNAP_FILE,
};

fn seat(n: u64) -> WorkId {
    WorkId { hi: n, lo: n }
}

fn add(
    graph: &mut WorkGraph,
    kind: WorkKind,
    status: WorkStatus,
    role: WorkRole,
    parent: WorkId,
    summary: &str,
) -> WorkId {
    graph
        .upsert(
            WorkId::ZERO,
            WorkFields {
                kind,
                status,
                role,
                parent,
                actor: seat(1),
                summary,
            },
        )
        .expect("upsert")
}

/// The line tests/compat_0_7_1.rs builds for the same node. Keep the two
/// the same.
fn describe(n: &WorkNode) -> String {
    let deps: Vec<String> = n.deps.iter().map(|d| d.to_hex()).collect();
    format!(
        "{} {} {} {} {} {} [{}] {} {} {} {} {} {:?}",
        n.id.to_hex(),
        n.kind.as_str(),
        n.status.as_str(),
        n.role.as_str(),
        n.assignee.to_hex(),
        n.parent.to_hex(),
        deps.join(","),
        n.cas_gen,
        n.created_unix,
        n.updated_unix,
        n.finished_unix,
        n.archived,
        n.summary,
    )
}

fn lines(graph: &WorkGraph) -> Vec<String> {
    graph.list().into_iter().map(describe).collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let bin = PathBuf::from(args.next().expect("path for work.bin"));
    let txt = PathBuf::from(args.next().expect("path for the node lines"));

    let mut graph = WorkGraph::default();
    let goal = add(
        &mut graph,
        WorkKind::Goal,
        WorkStatus::Ready,
        WorkRole::Orchestrator,
        WorkId::ZERO,
        "ship the release",
    );
    let design = add(
        &mut graph,
        WorkKind::Step,
        WorkStatus::Ready,
        WorkRole::Architect,
        goal,
        "design the change",
    );
    let build = add(
        &mut graph,
        WorkKind::Task,
        WorkStatus::Ready,
        WorkRole::Implementor,
        goal,
        "build it",
    );
    let check = add(
        &mut graph,
        WorkKind::Task,
        WorkStatus::Ready,
        WorkRole::Verifier,
        goal,
        "check it",
    );
    graph
        .link_dep(design, build, seat(1))
        .expect("build waits on design");
    graph
        .link_dep(build, check, seat(1))
        .expect("check waits on build");

    // design: claimed, run and done, which makes build ready.
    graph.claim(design, seat(3), None).expect("claim design");
    graph.set_running(design, seat(3)).expect("run design");
    graph
        .complete(design, WorkStatus::Done, "designed", seat(3), None)
        .expect("finish design");
    // build: a live claim. check still waits on it.
    graph.claim(build, seat(4), None).expect("claim build");

    let bench = add(
        &mut graph,
        WorkKind::Task,
        WorkStatus::Ready,
        WorkRole::Implementor,
        WorkId::ZERO,
        "run the bench",
    );
    graph.claim(bench, seat(5), None).expect("claim bench");
    graph.set_running(bench, seat(5)).expect("run bench");

    let survey = add(
        &mut graph,
        WorkKind::Molecule,
        WorkStatus::Ready,
        WorkRole::Explore,
        WorkId::ZERO,
        "survey the old path",
    );
    graph.claim(survey, seat(6), None).expect("claim survey");
    graph
        .complete(
            survey,
            WorkStatus::Failed,
            "no old path left",
            seat(6),
            None,
        )
        .expect("fail survey");
    graph.archive(survey, seat(6)).expect("archive survey");

    let shim = add(
        &mut graph,
        WorkKind::Task,
        WorkStatus::Ready,
        WorkRole::General,
        WorkId::ZERO,
        "drop the shim",
    );
    graph
        .complete(shim, WorkStatus::Cancelled, "kept the shim", seat(1), None)
        .expect("cancel shim");

    // A node with no open dependency is made ready, so blocked takes a
    // dependency first and the status after.
    let reply = add(
        &mut graph,
        WorkKind::Task,
        WorkStatus::Todo,
        WorkRole::General,
        WorkId::ZERO,
        "wait for a reply",
    );
    graph
        .link_dep(check, reply, seat(1))
        .expect("reply waits on check");
    graph
        .upsert(
            reply,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Blocked,
                role: WorkRole::General,
                parent: WorkId::ZERO,
                actor: seat(1),
                summary: "",
            },
        )
        .expect("block reply");
    add(
        &mut graph,
        WorkKind::Unset,
        WorkStatus::Ready,
        WorkRole::Unset,
        WorkId::ZERO,
        "unsorted",
    );

    let written = lines(&graph);
    for want in [
        "todo",
        "ready",
        "claimed",
        "running",
        "blocked",
        "done",
        "failed",
        "cancelled",
    ] {
        assert!(
            graph.list().iter().any(|n| n.status.as_str() == want),
            "no {want} node: {written:#?}"
        );
    }
    for want in ["unset", "goal", "step", "task", "molecule"] {
        assert!(
            graph.list().iter().any(|n| n.kind.as_str() == want),
            "no {want} kind: {written:#?}"
        );
    }
    for want in [
        "unset",
        "explore",
        "architect",
        "implementor",
        "verifier",
        "orchestrator",
        "general",
    ] {
        assert!(
            graph.list().iter().any(|n| n.role.as_str() == want),
            "no {want} role: {written:#?}"
        );
    }

    let dir = std::env::temp_dir().join(format!("claimdag-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    graph.save_dir(&dir).expect("save");
    let reread = WorkGraph::open_dir(&dir).expect("0.7.1 reads its own snapshot");
    assert_eq!(lines(&reread), written, "0.7.1 read back other nodes");
    copy(&dir.join(SNAP_FILE), &bin);
    std::fs::write(&txt, written.join("\n") + "\n").expect("write the node lines");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

fn copy(from: &Path, to: &Path) {
    std::fs::copy(from, to).expect("copy work.bin");
}
