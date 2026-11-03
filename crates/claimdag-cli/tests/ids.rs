//! A malformed id is a bad id. A hex prefix of one node names that node
//! and is not used as the id.

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

fn upsert(dir: &Path) -> String {
    let out = claimdag(dir, &["upsert", "--summary", "one"]);
    assert_eq!(out.status.code(), Some(0), "upsert: {}", stderr(&out));
    stdout(&out)
}

#[test]
fn a_short_id_is_bad_not_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let _kept = upsert(dir.path());
    let out = claimdag(dir.path(), &["get", "zz"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.starts_with("bad id zz"), "{err}");
    assert!(!err.contains("unknown"), "{err}");
    assert_eq!(err, "bad id zz: an id is 32 hex characters, this is 2\n");
    assert!(
        out.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn a_unique_prefix_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let id = upsert(dir.path());
    // Eight characters is the assignee prefix `list` prints. A seat that
    // copies that width from a node id gets a hint, not the node.
    let prefix = &id[..8];
    let out = claimdag(dir.path(), &["get", prefix]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        format!("bad id {prefix}: an id is 32 hex characters, this is 8. did you mean {id}\n"),
        "stderr"
    );
    assert!(
        out.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let full = claimdag(dir.path(), &["get", &id]);
    assert_eq!(full.status.code(), Some(0), "{}", stderr(&full));
    assert!(stdout(&full).starts_with(&id), "{}", stdout(&full));
}
