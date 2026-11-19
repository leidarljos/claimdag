//! Omitting `--status` keeps the status an existing node already has.
//! A new node still starts at todo, which readiness turns into ready when
//! nothing blocks it. An explicit status on held work is still refused.

use std::path::Path;
use std::process::{Command, Output};

const SEAT: &str = "00000000000000000000000000000007";

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
    let mut full = vec!["upsert"];
    full.extend_from_slice(args);
    let out = claimdag(dir, &full);
    assert_eq!(out.status.code(), Some(0), "upsert: {}", stderr(&out));
    stdout(&out)
}

/// The first line `get` prints for `id`.
fn get_line(dir: &Path, id: &str) -> String {
    let out = claimdag(dir, &["get", id]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    stdout(&out).lines().next().expect("get").to_string()
}

fn status_of(line: &str) -> &str {
    line.split_whitespace().nth(1).expect("status")
}

fn kind_of(line: &str) -> &str {
    line.split_whitespace().nth(2).expect("kind")
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key}=");
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("no {key} in {line}"))
}

fn summary_of(line: &str) -> &str {
    let parent = line
        .split_whitespace()
        .find(|w| w.starts_with("parent="))
        .expect("parent");
    let at = line.find(parent).expect("parent") + parent.len();
    line[at..].trim()
}

/// Claim `id` for `SEAT` and return the `get` line while it is held.
fn claim(dir: &Path, id: &str) -> String {
    let out = claimdag(dir, &["claim", id, "--assignee", SEAT]);
    assert_eq!(out.status.code(), Some(0), "claim: {}", stderr(&out));
    let line = get_line(dir, id);
    assert_eq!(status_of(&line), "claimed", "{line}");
    assert_eq!(field(&line, "assignee"), SEAT, "{line}");
    assert_eq!(
        format!("gen={}", field(&line, "gen")),
        stdout(&out),
        "{line}"
    );
    line
}

#[test]
fn a_holder_can_rewrite_the_summary_of_held_work() {
    let dir = tempfile::tempdir().unwrap();
    let id = upsert(dir.path(), &["--summary", "parse header"]);
    let before = claim(dir.path(), &id);
    let again = claimdag(
        dir.path(),
        &["upsert", "--id", &id, "--summary", "parse header v2"],
    );
    assert_eq!(again.status.code(), Some(0), "{}", stderr(&again));
    assert_eq!(stdout(&again), id);
    let after = get_line(dir.path(), &id);
    assert_eq!(status_of(&after), "claimed", "{after}");
    assert_eq!(
        field(&after, "assignee"),
        field(&before, "assignee"),
        "{after}"
    );
    assert_eq!(field(&after, "gen"), field(&before, "gen"), "{after}");
    assert_eq!(summary_of(&after), "parse header v2", "{after}");
}

#[test]
fn a_new_node_without_status_is_todo_or_ready() {
    let dir = tempfile::tempdir().unwrap();
    let id = upsert(dir.path(), &["--summary", "x"]);
    let line = get_line(dir.path(), &id);
    assert_eq!(status_of(&line), "ready", "{line}");
    assert_eq!(summary_of(&line), "x", "{line}");
}

#[test]
fn an_explicit_status_on_held_work_is_still_refused() {
    let dir = tempfile::tempdir().unwrap();
    let id = upsert(dir.path(), &["--summary", "parse header"]);
    let before = claim(dir.path(), &id);
    let out = claimdag(dir.path(), &["upsert", "--id", &id, "--status", "todo"]);
    assert_eq!(out.status.code(), Some(1), "stdout {}", stdout(&out));
    assert!(stdout(&out).is_empty(), "stdout {}", stdout(&out));
    let after = get_line(dir.path(), &id);
    assert_eq!(after, before, "a refused upsert changed the node");
}

#[test]
fn omitting_kind_on_upsert_keeps_the_existing_nodes_kind() {
    let dir = tempfile::tempdir().unwrap();
    let id = upsert(dir.path(), &["--kind", "goal", "--summary", "initial goal"]);
    assert_eq!(kind_of(&get_line(dir.path(), &id)), "goal");

    let updated = upsert(dir.path(), &["--id", &id, "--summary", "revised goal"]);
    assert_eq!(updated, id);
    let line = get_line(dir.path(), &id);
    assert_eq!(kind_of(&line), "goal", "{line}");
    assert_eq!(summary_of(&line), "revised goal", "{line}");

    let minted = upsert(dir.path(), &["--summary", "default kind"]);
    assert_eq!(kind_of(&get_line(dir.path(), &minted)), "task");
}

#[test]
fn list_and_get_show_the_id_prefix_when_summary_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let id = upsert(dir.path(), &[]);

    let listed = claimdag(dir.path(), &["list"]);
    assert_eq!(listed.status.code(), Some(0), "{}", stderr(&listed));
    assert!(stdout(&listed).contains(&id[..8]), "{}", stdout(&listed));

    let line = get_line(dir.path(), &id);
    assert_eq!(summary_of(&line), &id[..8], "{line}");
}
