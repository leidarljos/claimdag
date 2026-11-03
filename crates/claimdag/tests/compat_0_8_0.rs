//! A work.bin that claimdag 0.8.0 wrote, read by this build.
//!
//! fixtures/gen-0.8.0 wrote fixtures/work-0.8.0.bin with claimdag pinned
//! to the v0.8.0 tag, and wrote one line per node to
//! fixtures/work-0.8.0.txt. This build has to read the same nodes back,
//! and saving them has to write the bytes 0.8.0 wrote. A ready node in
//! the file can be claimed, completed and written back.

use std::path::PathBuf;

use claimdag::{WorkGraph, WorkId, WorkNode, WorkStatus, SNAP_BIN};

const SNAPSHOT: &[u8] = include_bytes!("fixtures/work-0.8.0.bin");
const NODES: &str = include_str!("fixtures/work-0.8.0.txt");

/// The line fixtures/gen-0.8.0 wrote for the same node. Keep the two the
/// same.
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

/// A directory holding only the 0.8.0 snapshot.
fn with_snapshot(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("claimdag-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(SNAP_BIN), SNAPSHOT).unwrap();
    dir
}

#[test]
fn a_0_8_0_snapshot_reads_back_node_for_node() {
    let dir = with_snapshot("compat-0-8-0-read");
    let graph = WorkGraph::open_dir(&dir).expect("open the 0.8.0 snapshot");
    let got: Vec<String> = graph.list().into_iter().map(describe).collect();
    let want: Vec<&str> = NODES.lines().collect();
    assert_eq!(got, want);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn saving_a_0_8_0_snapshot_writes_the_bytes_0_8_0_wrote() {
    let from = with_snapshot("compat-0-8-0-from");
    let to = std::env::temp_dir().join(format!("claimdag-compat-0-8-0-to-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&to);
    WorkGraph::open_dir(&from)
        .expect("open the 0.8.0 snapshot")
        .save_dir(&to)
        .expect("save");
    let saved = std::fs::read(to.join(SNAP_BIN)).unwrap();
    assert!(
        saved == SNAPSHOT,
        "saved {} bytes, 0.8.0 wrote {}",
        saved.len(),
        SNAPSHOT.len()
    );
    std::fs::remove_dir_all(&from).unwrap();
    std::fs::remove_dir_all(&to).unwrap();
}

#[test]
fn a_ready_node_in_a_0_8_0_snapshot_can_be_claimed_completed_and_written_back() {
    let dir = with_snapshot("compat-0-8-0-claim");
    // Seat 9 holds no live claim in the fixture. Seats 4 and 5 already
    // do, and one seat can hold only one claimed or running node.
    let seat = WorkId { hi: 9, lo: 9 };
    let node = {
        let _lock = claimdag::lock_dir(&dir).expect("lock");
        let mut graph = WorkGraph::load_for_write(&dir).expect("load for write");
        let node = graph
            .list()
            .into_iter()
            .find(|n| n.status == WorkStatus::Ready && n.deps.is_empty())
            .map(|n| n.id)
            .expect("a ready node with no dependencies");
        let generation = graph.claim(node, seat, None).expect("claim");
        graph
            .complete(node, WorkStatus::Done, "finished", seat, Some(generation))
            .expect("complete");
        graph.save_dir(&dir).expect("write the snapshot back");
        node
    };
    let graph = WorkGraph::open_dir(&dir).expect("reopen the written snapshot");
    let saved = graph.get(node).expect("the completed node");
    assert_eq!(saved.status, WorkStatus::Done);
    assert_eq!(saved.assignee, seat);
    assert_eq!(saved.summary, "finished");
    assert_ne!(saved.finished_unix, 0);
    let others: Vec<&str> = NODES
        .lines()
        .filter(|line| !line.starts_with(&node.to_hex()))
        .collect();
    let got_others: Vec<String> = graph
        .list()
        .into_iter()
        .filter(|n| n.id != node)
        .map(describe)
        .collect();
    assert_eq!(got_others, others);
    std::fs::remove_dir_all(&dir).unwrap();
}
