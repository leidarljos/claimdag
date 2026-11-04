//! A refusal names the node, the holder and since when those exist, and the
//! command that resolves it. The process status is 1.

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

fn claimdag_bare(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_claimdag"))
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

/// A ready node held by seat A.
fn held(dir: &Path) -> String {
    let id = upsert(dir, &["--status", "ready", "--summary", "held"]);
    let out = claimdag(dir, &["claim", &id, "--assignee", SEAT_A]);
    assert_eq!(out.status.code(), Some(0), "claim: {}", stderr(&out));
    id
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

#[test]
fn not_assignee_runs_and_names_the_holder() {
    for verb in ["release", "renew", "complete"] {
        let dir = tempfile::tempdir().unwrap();
        let id = held(dir.path());
        let since = updated_unix(dir.path(), &id);
        let out = claimdag(dir.path(), &[verb, &id, "--actor", SEAT_B]);
        let line = format!(
            "allow\twarn\tnot-assignee\t{verb} {id} actor {SEAT_B} holder {SEAT_A} since {since}"
        );
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(stderr(&out), format!("{line}\n"));
        let log = std::fs::read_to_string(dir.path().join("warn.log")).unwrap();
        assert_eq!(log, format!("{line}\n"));
        match verb {
            "release" => assert_eq!(field(dir.path(), &id, "assignee"), NOBODY),
            "renew" => {
                assert_eq!(field(dir.path(), &id, "assignee"), SEAT_A);
                assert_eq!(status_of(dir.path(), &id), "claimed");
            }
            "complete" => assert_eq!(stdout(&out), format!("{id}  done")),
            _ => unreachable!(),
        }
    }
}

#[test]
fn no_actor_runs_and_names_the_holder() {
    for (verb, args_tail) in [
        ("release", None),
        ("complete", None),
        ("release", Some(NOBODY)),
        ("complete", Some(NOBODY)),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let id = held(dir.path());
        let mut args = vec![verb, id.as_str()];
        if let Some(actor) = args_tail {
            args.extend(["--actor", actor]);
        }
        let out = claimdag(dir.path(), &args);
        let line = format!("allow\twarn\tno-actor\t{verb} {id} holder {SEAT_A}");
        assert_eq!(out.status.code(), Some(0), "{args:?} {}", stderr(&out));
        assert_eq!(stderr(&out), format!("{line}\n"), "{args:?}");
        assert!(!dir.path().join("forced.log").exists());
    }
}

#[test]
fn release_of_an_unheld_node_names_the_status() {
    let dir = tempfile::tempdir().unwrap();
    let id = upsert(dir.path(), &["--status", "ready", "--summary", "open"]);
    assert_refused(
        &claimdag(dir.path(), &["release", &id]),
        "release: status ready",
    );
    assert_eq!(status_of(dir.path(), &id), "ready");
    assert_eq!(field(dir.path(), &id, "assignee"), NOBODY);
    assert!(!dir.path().join("warn.log").exists());
}

#[test]
fn complete_with_no_holder_runs_and_is_labeled() {
    let dir = tempfile::tempdir().unwrap();
    let id = upsert(dir.path(), &["--status", "ready", "--summary", "open"]);
    let out = claimdag(dir.path(), &["complete", &id]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let line = format!("allow\twarn\tno-actor\tcomplete {id} holder {NOBODY}");
    assert_eq!(stderr(&out), format!("{line}\n"));
    assert_eq!(stdout(&out), format!("{id}  done"));
    let other = upsert(dir.path(), &["--status", "ready", "--summary", "forced"]);
    let forced = claimdag(dir.path(), &["complete", &other, "--force"]);
    assert_eq!(forced.status.code(), Some(0), "{}", stderr(&forced));
    assert_eq!(stdout(&forced), format!("{other}  done"));
    let log = std::fs::read_to_string(dir.path().join("forced.log")).unwrap();
    assert!(log.contains("complete --force"), "{log}");
    assert!(log.contains(&other), "{log}");
}

#[test]
fn unknown_id_names_the_id_and_list() {
    let dir = tempfile::tempdir().unwrap();
    let real = upsert(dir.path(), &["--status", "ready", "--summary", "real"]);
    let missing = "dddddddddddddddddddddddddddddddd";
    let listed = format!("{missing}. claimdag list --all");

    assert_refused(
        &claimdag(dir.path(), &["get", missing]),
        &format!("get: unknown id {listed}"),
    );
    assert_refused(
        &claimdag(dir.path(), &["claim", missing, "--assignee", SEAT_A]),
        &format!("claim: unknown id {listed}"),
    );
    assert_refused(
        &claimdag(dir.path(), &["release", missing, "--actor", SEAT_A]),
        &format!("release: unknown id {listed}"),
    );
    assert_refused(
        &claimdag(dir.path(), &["get", "zz"]),
        "bad id zz: an id is 32 hex characters, this is 2",
    );

    let out = claimdag(dir.path(), &["list", "--all"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains(&real), "{text}");
    assert!(!text.contains(missing), "{text}");
}

#[test]
fn parent_cycle_names_a_command_that_keeps_the_stored_node() {
    let dir = tempfile::tempdir().unwrap();
    let a = upsert(dir.path(), &["--status", "ready", "--summary", "alpha"]);
    let b = upsert(
        dir.path(),
        &["--status", "ready", "--summary", "beta", "--parent", &a],
    );
    // The request asks for a different kind and status. The refusal repeats
    // the stored ones: the write was rejected, and the command line's
    // defaults would otherwise demote a ready node to todo.
    let out = claimdag(
        dir.path(),
        &[
            "upsert", "--id", &a, "--parent", &b, "--kind", "step", "--status", "todo",
        ],
    );
    let line = format!(
        "upsert: parent cycle. node {a} would have parent {b}. claimdag upsert --id {a} --kind task --status ready"
    );
    assert_refused(&out, &line);
    assert_eq!(field(dir.path(), &a, "parent"), NOBODY);

    let fixed = claimdag(
        dir.path(),
        &["upsert", "--id", &a, "--kind", "task", "--status", "ready"],
    );
    assert_eq!(fixed.status.code(), Some(0), "{}", stderr(&fixed));
    assert_eq!(field(dir.path(), &a, "parent"), NOBODY);
    assert_eq!(status_of(dir.path(), &a), "ready");
}

#[test]
fn parent_cycle_on_a_held_node_names_the_holder() {
    let dir = tempfile::tempdir().unwrap();
    let a = upsert(dir.path(), &["--status", "ready", "--summary", "alpha"]);
    let b = upsert(
        dir.path(),
        &["--status", "ready", "--summary", "beta", "--parent", &a],
    );
    let claimed = claimdag(dir.path(), &["claim", &a, "--assignee", SEAT_A]);
    assert_eq!(claimed.status.code(), Some(0), "{}", stderr(&claimed));
    let since = updated_unix(dir.path(), &a);
    let out = claimdag(
        dir.path(),
        &["upsert", "--id", &a, "--parent", &b, "--status", "ready"],
    );
    let line = format!(
        "upsert: parent cycle. node {a} is held by {SEAT_A} since unix {since} and would have parent {b}. claimdag release {a} --actor {SEAT_A}"
    );
    assert_refused(&out, &line);
    assert_eq!(field(dir.path(), &a, "assignee"), SEAT_A);
    assert_eq!(field(dir.path(), &a, "parent"), NOBODY);

    let fixed = claimdag(dir.path(), &["release", &a, "--actor", SEAT_A]);
    assert_eq!(fixed.status.code(), Some(0), "{}", stderr(&fixed));
    assert_eq!(field(dir.path(), &a, "assignee"), NOBODY);
    assert_eq!(field(dir.path(), &a, "parent"), NOBODY);
}

#[test]
fn parent_cycle_on_a_finished_node_names_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let a = upsert(dir.path(), &["--status", "ready", "--summary", "alpha"]);
    let b = upsert(
        dir.path(),
        &["--status", "ready", "--summary", "beta", "--parent", &a],
    );
    let claimed = claimdag(dir.path(), &["claim", &a, "--assignee", SEAT_A]);
    assert_eq!(claimed.status.code(), Some(0), "{}", stderr(&claimed));
    let done = claimdag(dir.path(), &["complete", &a, "--actor", SEAT_A]);
    assert_eq!(done.status.code(), Some(0), "{}", stderr(&done));
    let out = claimdag(
        dir.path(),
        &["upsert", "--id", &a, "--parent", &b, "--status", "ready"],
    );
    let line = format!("upsert: parent cycle. node {a} would have parent {b}. claimdag reopen {a}");
    assert_refused(&out, &line);

    let fixed = claimdag(dir.path(), &["reopen", &a]);
    assert_eq!(fixed.status.code(), Some(0), "{}", stderr(&fixed));
    assert_eq!(status_of(dir.path(), &a), "ready");
    assert_eq!(field(dir.path(), &a, "parent"), NOBODY);
}

#[test]
fn a_new_node_cannot_be_its_own_parent() {
    let dir = tempfile::tempdir().unwrap();
    let id = "cccccccccccccccccccccccccccccccc";
    let out = claimdag(
        dir.path(),
        &[
            "upsert", "--id", id, "--parent", id, "--kind", "step", "--status", "ready",
        ],
    );
    let line = format!(
        "upsert: parent cycle. node {id} would have parent {id}. claimdag upsert --id {id} --kind step --status ready"
    );
    assert_refused(&out, &line);
    let fixed = claimdag(
        dir.path(),
        &["upsert", "--id", id, "--kind", "step", "--status", "ready"],
    );
    assert_eq!(fixed.status.code(), Some(0), "{}", stderr(&fixed));
    assert_eq!(stdout(&fixed), id);
    assert_eq!(field(dir.path(), id, "parent"), NOBODY);
}

#[test]
fn release_of_an_unknown_id_with_no_actor_names_list() {
    let dir = tempfile::tempdir().unwrap();
    let _real = upsert(dir.path(), &["--status", "ready", "--summary", "real"]);
    let missing = "dddddddddddddddddddddddddddddddd";
    let line = format!("release: unknown id {missing}. claimdag list --all");
    assert_refused(&claimdag(dir.path(), &["release", missing]), &line);
}

#[test]
fn help_documents_the_exit_statuses() {
    let out = claimdag_bare(&["--help"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let flat = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        flat.contains(
            "Exit status is 0 when the verb ran, 1 when it was refused, and 2 when the arguments could not be read."
        ),
        "{flat}"
    );
    assert!(flat.contains("who holds it and the unix second"), "{flat}");
}

#[test]
fn unreadable_arguments_exit_2() {
    let out = claimdag_bare(&[]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let out = claimdag_bare(&["--not-a-flag"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}
