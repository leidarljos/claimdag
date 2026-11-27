//! A blocking edge that would close a cycle is refused, and the graph is
//! left as it was. An advisory edge is recorded and does not block.

use std::path::Path;
use std::process::{Command, Output};

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

fn upsert(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["upsert", "--status", "ready"];
    full.extend_from_slice(args);
    let out = claimdag(dir, &full);
    assert_eq!(out.status.code(), Some(0), "upsert: {}", stderr(&out));
    stdout(&out)
}

fn snapshot(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join("work.bin")).expect("work.bin")
}

/// P14. Linking the reverse of a blocking edge is a cycle. The process
/// status is 3 and the snapshot is the one from before the call.
#[test]
fn a_blocking_cycle_is_refused_and_the_graph_stays() {
    let dir = tempfile::tempdir().unwrap();
    let first = upsert(dir.path(), &["--summary", "first"]);
    let second = upsert(dir.path(), &["--summary", "second"]);
    let linked = claimdag(dir.path(), &["link", &first, &second]);
    assert_eq!(linked.status.code(), Some(0), "{}", stderr(&linked));
    let before = snapshot(dir.path());

    let out = claimdag(dir.path(), &["link", "--kind", "hard", &second, &first]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(stderr(&out), format!("cycle involving {first}\n"), "stderr");
    assert!(out.stdout.is_empty(), "stdout: {}", stdout(&out));
    assert_eq!(snapshot(dir.path()), before, "snapshot changed");

    let got = claimdag(dir.path(), &["get", &first]);
    assert_eq!(got.status.code(), Some(0), "{}", stderr(&got));
    let body = stdout(&got);
    let deps = body.lines().nth(1).expect("deps line");
    assert_eq!(deps, "deps  -");
}

/// P15. An advisory edge between siblings is listed, and both stay ready.
#[test]
fn an_advisory_edge_is_listed_and_both_siblings_stay_ready() {
    let dir = tempfile::tempdir().unwrap();
    let parent = upsert(dir.path(), &["--summary", "parent"]);
    let left = upsert(dir.path(), &["--summary", "left", "--parent", &parent]);
    let right = upsert(dir.path(), &["--summary", "right", "--parent", &parent]);
    let linked = claimdag(dir.path(), &["link", "--kind", "advisory", &left, &right]);
    assert_eq!(linked.status.code(), Some(0), "{}", stderr(&linked));

    let got = claimdag(dir.path(), &["get", &right]);
    assert_eq!(got.status.code(), Some(0), "{}", stderr(&got));
    let body = stdout(&got);
    let deps = body.lines().nth(1).expect("deps line");
    assert_eq!(deps, format!("deps  {left} advisory"));

    let ready = claimdag(dir.path(), &["ready"]);
    assert_eq!(ready.status.code(), Some(0), "{}", stderr(&ready));
    let text = stdout(&ready);
    assert!(text.contains(&left), "left missing from ready: {text}");
    assert!(text.contains(&right), "right missing from ready: {text}");
}
