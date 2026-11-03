//! `claim-next` takes the same node for the same graph and the same worker,
//! whichever process asks.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

/// Ready nodes of one depth and one role, so every one is a candidate and
/// only the worker's seed tells them apart. Twelve runs on a graph of eight
/// picked five different nodes before the pick was made from a fixed order.
const NODES: usize = 8;

/// One run per node is enough to see a second pick: those twelve runs never
/// went eight in a row on one node.
const RUNS: usize = NODES;

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

fn worker(n: u64) -> String {
    format!("{n:032x}")
}

/// The node `claim-next` gives `assignee` on a fresh copy of the graph in
/// `seed`, so every run starts from the same snapshot.
fn pick(seed: &Path, assignee: &str) -> String {
    let copy = tempfile::tempdir().unwrap();
    std::fs::copy(
        seed.join(claimdag::SNAP_BIN),
        copy.path().join(claimdag::SNAP_BIN),
    )
    .unwrap();
    let out = claimdag(copy.path(), &["claim-next", "--assignee", assignee]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line = stdout(&out);
    line.split_whitespace().next().unwrap().to_string()
}

fn seeded() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..NODES {
        let out = claimdag(dir.path(), &["upsert", "--summary", &format!("node {n}")]);
        assert!(out.status.success());
    }
    dir
}

#[test]
fn the_same_worker_takes_the_same_node_in_every_process() {
    let graph = seeded();
    let me = worker(7);
    let picks: BTreeSet<String> = (0..RUNS).map(|_| pick(graph.path(), &me)).collect();
    assert_eq!(picks.len(), 1, "{picks:?}");
}

/// The seed is the worker, so two workers asking at once do not both go
/// for one node.
#[test]
fn different_workers_spread_over_the_ready_nodes() {
    let graph = seeded();
    let picks: BTreeSet<String> = (1..=RUNS as u64)
        .map(|n| pick(graph.path(), &worker(n)))
        .collect();
    assert!(picks.len() > 1, "{picks:?}");
}
