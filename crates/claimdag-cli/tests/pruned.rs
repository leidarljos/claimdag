//! A node the graph pruned after it finished is still finished: completing or
//! releasing it again succeeds and says so.

use std::path::Path;
use std::process::{Command, Output};

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};

const SEAT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// No cap inside the budget in measurements/node-cap.org comes near this
/// many nodes, so a graph that has pruned nothing by then has no cap.
const NO_CAP: usize = 1 << 16;

fn claimdag(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_claimdag"))
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .expect("run claimdag")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn add(graph: &mut WorkGraph, summary: &str) -> WorkId {
    graph
        .upsert(
            WorkId::ZERO,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: WorkId::ZERO,
                summary,
            },
        )
        .expect("upsert")
}

/// A graph in `dir` that finished one node, then took on live work until
/// prune dropped it. Returns the dropped node.
fn finished_then_pruned(dir: &Path) -> String {
    let seat = WorkId::from_hex(SEAT).unwrap();
    let mut graph = WorkGraph::default();
    let first = add(&mut graph, "finished first");
    graph.claim(first, seat, None).expect("claim");
    graph
        .complete(first, WorkStatus::Done, "", seat, None)
        .expect("complete");
    let mut made = 1;
    while graph.get(first).is_some() {
        assert!(made < NO_CAP, "nothing was pruned past {made} nodes");
        add(&mut graph, &format!("live {made}"));
        made += 1;
    }
    graph.save_dir(dir).expect("save");
    first.to_hex()
}

#[test]
fn completing_or_releasing_a_pruned_node_succeeds_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let id = finished_then_pruned(dir.path());
    let said = format!("{id}  done  already terminal and pruned; nothing changed");

    let out = claimdag(
        dir.path(),
        &["complete", &id, "--actor", SEAT, "--gen", "2"],
    );
    assert!(out.status.success(), "complete: {}", stderr(&out));
    assert_eq!(stdout(&out), said);

    let out = claimdag(dir.path(), &["release", &id, "--actor", SEAT]);
    assert!(out.status.success(), "release: {}", stderr(&out));
    assert_eq!(stdout(&out), said);

    let out = claimdag(dir.path(), &["claim", &id, "--assignee", SEAT]);
    assert!(!out.status.success(), "claimed finished work");
    assert!(
        stderr(&out).contains("claim: status done, pruned"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn an_id_the_graph_never_had_is_still_not_found() {
    let dir = tempfile::tempdir().unwrap();
    finished_then_pruned(dir.path());
    let never = "cccccccccccccccccccccccccccccccc";
    for verb in ["complete", "release"] {
        let out = claimdag(dir.path(), &[verb, never, "--actor", SEAT]);
        assert!(
            !out.status.success(),
            "{verb} of a node never seen succeeded"
        );
        assert!(
            stderr(&out).contains(&format!("{verb}: unknown id {never}")),
            "{}",
            stderr(&out)
        );
    }
}
