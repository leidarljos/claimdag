//! release and complete name who is acting, or say plainly that nobody is.

use std::path::Path;
use std::process::{Command, Output};

const SEAT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SEAT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const NOBODY: &str = "00000000000000000000000000000000";

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

/// A ready node held by seat A.
fn held(dir: &Path) -> String {
    let out = claimdag(dir, &["upsert", "--status", "ready", "--summary", "held"]);
    assert!(out.status.success(), "upsert: {}", stderr(&out));
    let id = stdout(&out);
    let out = claimdag(dir, &["claim", &id, "--assignee", SEAT_A]);
    assert!(out.status.success(), "claim: {}", stderr(&out));
    id
}

fn assignee(dir: &Path, id: &str) -> String {
    let out = claimdag(dir, &["get", id]);
    let line = stdout(&out);
    line.split_whitespace()
        .find_map(|w| w.strip_prefix("assignee=").map(str::to_string))
        .expect("get prints the assignee")
}

#[test]
fn release_and_complete_without_an_actor_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let id = held(dir.path());
    for args in [vec!["release", &id], vec!["complete", &id]] {
        let out = claimdag(dir.path(), &args);
        assert!(!out.status.success(), "{args:?} went through with no actor");
        assert!(
            stderr(&out).contains("--actor"),
            "{args:?} did not say what is missing: {}",
            stderr(&out)
        );
    }
    for verb in ["release", "complete"] {
        let out = claimdag(dir.path(), &[verb, &id, "--actor", NOBODY]);
        assert!(!out.status.success(), "{verb} took the zero actor");
        assert!(stderr(&out).contains("no actor"), "{}", stderr(&out));
    }
    assert_eq!(assignee(dir.path(), &id), SEAT_A, "the claim moved");
    assert!(!dir.path().join("forced.log").exists());
}

#[test]
fn another_seat_still_cannot_release() {
    let dir = tempfile::tempdir().unwrap();
    let id = held(dir.path());
    let out = claimdag(dir.path(), &["release", &id, "--actor", SEAT_B]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not assignee"), "{}", stderr(&out));
    let out = claimdag(dir.path(), &["release", &id, "--actor", SEAT_A]);
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn force_acts_for_nobody_and_is_logged() {
    let dir = tempfile::tempdir().unwrap();
    let id = held(dir.path());
    let out = claimdag(dir.path(), &["release", &id, "--force"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("forced"), "no line on stderr");
    assert_eq!(assignee(dir.path(), &id), NOBODY);

    let out = claimdag(dir.path(), &["claim", &id, "--assignee", SEAT_B]);
    assert!(out.status.success(), "{}", stderr(&out));
    let out = claimdag(
        dir.path(),
        &["complete", &id, "--status", "cancelled", "--force"],
    );
    assert!(out.status.success(), "{}", stderr(&out));

    let log = std::fs::read_to_string(dir.path().join("forced.log")).expect("forced.log");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2, "{log}");
    assert!(lines[0].contains("release --force") && lines[0].contains(&format!("holder={SEAT_A}")));
    assert!(
        lines[1].contains("complete --force") && lines[1].contains(&format!("holder={SEAT_B}"))
    );
    assert!(lines.iter().all(|l| l.contains(&id)));
}

#[test]
fn force_and_actor_together_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let id = held(dir.path());
    let out = claimdag(dir.path(), &["release", &id, "--actor", SEAT_A, "--force"]);
    assert!(!out.status.success());
}
