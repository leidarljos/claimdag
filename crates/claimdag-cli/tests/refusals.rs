//! A refusal from claim, upsert, link, unlink or archive names the node and
//! the command that resolves it. The process status is 1.

use std::path::Path;
use std::process::{Command, Output};

const HOLDER: &str = "00000000000000000000000000000007";
const OTHER: &str = "00000000000000000000000000000009";
const MISSING: &str = "00000000000000000000000000000005";

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

fn assert_refused(out: &Output, line: &str) {
    assert_eq!(out.status.code(), Some(1), "{}", stderr(out));
    assert_eq!(stderr(out), format!("{line}\n"), "stderr");
    assert!(
        out.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

fn upsert(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["upsert"];
    full.extend_from_slice(args);
    let out = claimdag(dir, &full);
    assert_eq!(out.status.code(), Some(0), "upsert: {}", stderr(&out));
    stdout(&out)
}

fn ready(dir: &Path, summary: &str) -> String {
    upsert(dir, &["--status", "ready", "--summary", summary])
}

fn claim(dir: &Path, id: &str, assignee: &str) {
    let out = claimdag(dir, &["claim", id, "--assignee", assignee]);
    assert_eq!(out.status.code(), Some(0), "claim: {}", stderr(&out));
}

fn updated_unix(dir: &Path, id: &str) -> u64 {
    let out = claimdag(dir, &["list", "--json", "--all"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    let marker = format!("\"id\":\"{id}\"");
    let at = text
        .find(&marker)
        .unwrap_or_else(|| panic!("list --json has no {id}: {text}"));
    let rest = &text[at..];
    let key = "\"updated_unix\":";
    let rel = rest.find(key).expect("updated_unix");
    rest[rel + key.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .expect("updated_unix")
}

fn field(dir: &Path, id: &str, key: &str) -> String {
    let out = claimdag(dir, &["get", id]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let prefix = format!("{key}=");
    stdout(&out)
        .split_whitespace()
        .find_map(|w| w.strip_prefix(&prefix).map(str::to_string))
        .unwrap_or_else(|| panic!("no {key} in {}", stdout(&out)))
}

fn status_of(dir: &Path, id: &str) -> String {
    let out = claimdag(dir, &["get", id]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    stdout(&out)
        .split_whitespace()
        .nth(1)
        .expect("status")
        .to_string()
}

fn summary_of(dir: &Path, id: &str) -> String {
    let out = claimdag(dir, &["get", id]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    stdout(&out)
        .lines()
        .next()
        .expect("get")
        .split_whitespace()
        .last()
        .expect("summary")
        .to_string()
}

fn deps_of(dir: &Path, id: &str) -> String {
    let out = claimdag(dir, &["get", id]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    stdout(&out)
        .lines()
        .nth(1)
        .expect("deps")
        .strip_prefix("deps")
        .expect("deps prefix")
        .trim()
        .to_string()
}

#[test]
fn claim_of_a_held_node_names_the_holder_and_release() {
    let dir = tempfile::tempdir().unwrap();
    let id = ready(dir.path(), "held");
    claim(dir.path(), &id, HOLDER);
    let since = updated_unix(dir.path(), &id);
    let out = claimdag(dir.path(), &["claim", &id, "--assignee", OTHER]);
    let line = format!(
        "claim: status claimed. node {id} is held by {HOLDER} since unix {since}. claimdag release {id} --actor {HOLDER}"
    );
    assert_refused(&out, &line);
    assert_eq!(field(dir.path(), &id, "assignee"), HOLDER);

    let fixed = claimdag(dir.path(), &["release", &id, "--actor", HOLDER]);
    assert_eq!(fixed.status.code(), Some(0), "{}", stderr(&fixed));
    assert_eq!(field(dir.path(), &id, "assignee"), "0".repeat(32));
}

#[test]
fn claim_with_unfinished_deps_lists_them() {
    let dir = tempfile::tempdir().unwrap();
    let first = ready(dir.path(), "first");
    let second = ready(dir.path(), "second");
    let child = ready(dir.path(), "child");
    let linked = claimdag(dir.path(), &["link", &first, &child]);
    assert_eq!(linked.status.code(), Some(0), "{}", stderr(&linked));
    let linked = claimdag(dir.path(), &["link", &second, &child]);
    assert_eq!(linked.status.code(), Some(0), "{}", stderr(&linked));

    let out = claimdag(dir.path(), &["claim", &child, "--assignee", OTHER]);
    let line = format!("claim: deps unsatisfied {first} (ready) {second} (ready)");
    assert_refused(&out, &line);
    assert_eq!(status_of(dir.path(), &child), "todo");

    let done = claimdag(
        dir.path(),
        &["complete", &first, "--status", "done", "--force"],
    );
    assert_eq!(done.status.code(), Some(0), "{}", stderr(&done));
    let done = claimdag(
        dir.path(),
        &["complete", &second, "--status", "done", "--force"],
    );
    assert_eq!(done.status.code(), Some(0), "{}", stderr(&done));
    claim(dir.path(), &child, OTHER);
    assert_eq!(field(dir.path(), &child, "assignee"), OTHER);
}

#[test]
fn claim_when_the_assignee_is_busy_names_release_or_complete() {
    let dir = tempfile::tempdir().unwrap();
    let held = ready(dir.path(), "held");
    let next = ready(dir.path(), "next");
    claim(dir.path(), &held, HOLDER);
    let out = claimdag(dir.path(), &["claim", &next, "--assignee", HOLDER]);
    let line = format!(
        "claim: assignee busy {held}. claimdag release {held} --actor {HOLDER} or claimdag complete {held} --actor {HOLDER}"
    );
    assert_refused(&out, &line);
    assert_eq!(status_of(dir.path(), &next), "ready");
    assert_eq!(field(dir.path(), &next, "assignee"), "0".repeat(32));

    let fixed = claimdag(dir.path(), &["release", &held, "--actor", HOLDER]);
    assert_eq!(fixed.status.code(), Some(0), "{}", stderr(&fixed));
    claim(dir.path(), &next, HOLDER);
    assert_eq!(field(dir.path(), &next, "assignee"), HOLDER);
}

#[test]
fn claim_with_the_wrong_generation_names_the_current_one() {
    let dir = tempfile::tempdir().unwrap();
    let id = ready(dir.path(), "fenced");
    let generation = field(dir.path(), &id, "gen");
    let out = claimdag(
        dir.path(),
        &["claim", &id, "--assignee", OTHER, "--gen", "5"],
    );
    let line = format!("claim: gen mismatch. node {id} is generation {generation}");
    assert_refused(&out, &line);
    assert_eq!(field(dir.path(), &id, "gen"), generation);

    let fixed = claimdag(
        dir.path(),
        &["claim", &id, "--assignee", OTHER, "--gen", &generation],
    );
    assert_eq!(fixed.status.code(), Some(0), "{}", stderr(&fixed));
    assert_eq!(field(dir.path(), &id, "assignee"), OTHER);
}

#[test]
fn upsert_of_a_terminal_node_names_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let id = ready(dir.path(), "finished");
    claim(dir.path(), &id, HOLDER);
    let done = claimdag(dir.path(), &["complete", &id, "--actor", HOLDER]);
    assert_eq!(done.status.code(), Some(0), "{}", stderr(&done));

    let out = claimdag(dir.path(), &["upsert", "--id", &id, "--summary", "x"]);
    let line = format!("upsert: node is terminal. node {id} is done. claimdag reopen {id}");
    assert_refused(&out, &line);
    assert_eq!(status_of(dir.path(), &id), "done");
    assert_eq!(field(dir.path(), &id, "assignee"), HOLDER);

    let fixed = claimdag(dir.path(), &["reopen", &id]);
    assert_eq!(fixed.status.code(), Some(0), "{}", stderr(&fixed));
    assert_eq!(status_of(dir.path(), &id), "ready");
}

#[test]
fn upsert_of_a_claimed_node_names_complete_or_release() {
    let dir = tempfile::tempdir().unwrap();
    let id = ready(dir.path(), "held");
    claim(dir.path(), &id, HOLDER);
    let since = updated_unix(dir.path(), &id);
    // Omitting --status repeats the status the node has, which is a summary
    // edit. An explicit different status is what this refusal is for.
    let out = claimdag(
        dir.path(),
        &["upsert", "--id", &id, "--status", "todo", "--summary", "x"],
    );
    let line = format!(
        "upsert: node is claimed. node {id} is held by {HOLDER} since unix {since}. claimdag complete {id} --actor {HOLDER} or claimdag release {id} --actor {HOLDER}"
    );
    assert_refused(&out, &line);
    assert_eq!(status_of(dir.path(), &id), "claimed");
    assert_eq!(summary_of(dir.path(), &id), "held");

    let fixed = claimdag(dir.path(), &["release", &id, "--actor", HOLDER]);
    assert_eq!(fixed.status.code(), Some(0), "{}", stderr(&fixed));
    assert_eq!(status_of(dir.path(), &id), "ready");
}

#[test]
fn upsert_of_a_lifecycle_status_names_claim_or_complete() {
    let dir = tempfile::tempdir().unwrap();
    let id = ready(dir.path(), "open");
    let done = claimdag(
        dir.path(),
        &["upsert", "--id", &id, "--status", "done", "--summary", "x"],
    );
    assert_refused(
        &done,
        &format!("upsert: status claim/run/terminal requires claimdag complete {id} --status done"),
    );
    let claimed = claimdag(dir.path(), &["upsert", "--id", &id, "--status", "claimed"]);
    assert_refused(
        &claimed,
        &format!("upsert: status claim/run/terminal requires claimdag claim {id} --assignee"),
    );
    assert_eq!(status_of(dir.path(), &id), "ready");
    assert_eq!(summary_of(dir.path(), &id), "open");
}

#[test]
fn link_names_a_missing_parent() {
    let dir = tempfile::tempdir().unwrap();
    let child = ready(dir.path(), "child");
    let out = claimdag(dir.path(), &["link", MISSING, &child]);
    let line = format!("link: parent missing. unknown id {MISSING}. claimdag list --all");
    assert_refused(&out, &line);
    assert_eq!(deps_of(dir.path(), &child), "-");

    let listed = claimdag(dir.path(), &["list", "--all"]);
    assert_eq!(listed.status.code(), Some(0), "{}", stderr(&listed));
    assert!(stdout(&listed).contains(&child), "{}", stdout(&listed));
    assert!(!stdout(&listed).contains(MISSING), "{}", stdout(&listed));
}

#[test]
fn link_names_a_missing_child() {
    let dir = tempfile::tempdir().unwrap();
    let parent = ready(dir.path(), "parent");
    let out = claimdag(dir.path(), &["link", &parent, MISSING]);
    let line = format!("link: child missing. unknown id {MISSING}. claimdag list --all");
    assert_refused(&out, &line);
    assert_eq!(status_of(dir.path(), &parent), "ready");
}

#[test]
fn unlink_names_a_missing_child() {
    let dir = tempfile::tempdir().unwrap();
    let parent = ready(dir.path(), "parent");
    let out = claimdag(dir.path(), &["unlink", &parent, MISSING]);
    let line = format!("unlink: child missing. unknown id {MISSING}. claimdag list --all");
    assert_refused(&out, &line);
    assert_eq!(status_of(dir.path(), &parent), "ready");
}

#[test]
fn archive_of_a_live_node_names_the_node() {
    let dir = tempfile::tempdir().unwrap();
    let id = ready(dir.path(), "open");
    let out = claimdag(dir.path(), &["archive", &id]);
    let line = format!(
        "archive: not terminal. node {id} is ready. claimdag complete {id} --force. claimdag list --all"
    );
    assert_refused(&out, &line);
    assert_eq!(status_of(dir.path(), &id), "ready");

    let finished = claimdag(dir.path(), &["complete", &id, "--force"]);
    assert_eq!(finished.status.code(), Some(0), "{}", stderr(&finished));
    let archived = claimdag(dir.path(), &["archive", &id]);
    assert_eq!(archived.status.code(), Some(0), "{}", stderr(&archived));

    let held = ready(dir.path(), "held");
    claim(dir.path(), &held, HOLDER);
    let since = updated_unix(dir.path(), &held);
    let out = claimdag(dir.path(), &["archive", &held]);
    let line = format!(
        "archive: not terminal. node {held} is claimed and held by {HOLDER} since unix {since}. claimdag complete {held} --actor {HOLDER}. claimdag list --all"
    );
    assert_refused(&out, &line);
    assert_eq!(status_of(dir.path(), &held), "claimed");

    let finished = claimdag(dir.path(), &["complete", &held, "--actor", HOLDER]);
    assert_eq!(finished.status.code(), Some(0), "{}", stderr(&finished));
    let archived = claimdag(dir.path(), &["archive", &held]);
    assert_eq!(archived.status.code(), Some(0), "{}", stderr(&archived));
}
