//! `claimdag where` names the graph a seat uses and every other location that
//! holds one, and a seat that finds two says so once.

use std::path::Path;
use std::process::{Command, Output};

/// A seat whose home and state directory are under `root`, with no
/// `CLAIMDAG_DIR` and no runtime directory, so the defaults decide.
fn seat(root: &Path, state_home: bool, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_claimdag"));
    cmd.env_remove("CLAIMDAG_DIR")
        .env_remove("XDG_RUNTIME_DIR")
        .env("HOME", root.join("home"));
    if state_home {
        cmd.env("XDG_STATE_HOME", root.join("state"));
    } else {
        cmd.env_remove("XDG_STATE_HOME");
    }
    cmd.args(args).output().expect("run claimdag")
}

/// A graph with one node in `dir`, written the way a seat that named it would.
fn graph_in(dir: &Path, summary: &str) {
    let out = Command::new(env!("CARGO_BIN_EXE_claimdag"))
        .arg("--dir")
        .arg(dir)
        .args(["upsert", "--summary", summary])
        .output()
        .expect("run claimdag");
    assert!(out.status.success(), "{}", stderr(&out));
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn where_lists_the_graph_in_use_and_the_others_that_hold_work() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("state/claimdag");
    let home = root.path().join("home/.local/state/claimdag");
    graph_in(&first, "under XDG_STATE_HOME");
    graph_in(&home, "under HOME");
    let out = seat(root.path(), true, &["where"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).starts_with(&format!(
            "{}  in use  XDG_STATE_HOME\n{}  holds work  HOME\n",
            first.display(),
            home.display()
        )),
        "{}",
        stdout(&out)
    );
    assert_eq!(stderr(&out), "");
}

#[test]
fn a_second_graph_is_named_once() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("state/claimdag");
    let home = root.path().join("home/.local/state/claimdag");
    graph_in(&first, "under XDG_STATE_HOME");
    graph_in(&home, "under HOME");
    let line = format!(
        "claimdag: using {}; also holding work: {}",
        first.display(),
        home.display()
    );
    let out = seat(root.path(), true, &["list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("under XDG_STATE_HOME"));
    assert!(stderr(&out).contains(&line), "{}", stderr(&out));
    let out = seat(root.path(), true, &["list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out), "");
}

#[test]
fn a_seat_with_its_own_state_home_reads_the_graph_and_move_puts_it_there() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("state/claimdag");
    let home = root.path().join("home/.local/state/claimdag");
    graph_in(&home, "started without XDG_STATE_HOME");

    let out = seat(root.path(), true, &["list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("started without XDG_STATE_HOME"));
    let out = seat(root.path(), true, &["where"]);
    let shown = stdout(&out);
    assert!(
        shown.starts_with(&format!(
            "{}  no graph  XDG_STATE_HOME\n{}  in use  HOME\n",
            first.display(),
            home.display()
        )),
        "{shown}"
    );
    assert!(shown.ends_with(&format!(
        "claimdag where --move moves the graph in use to {}\n",
        first.display()
    )));

    let out = seat(root.path(), true, &["where", "--move"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).starts_with(&format!(
            "moved {} to {}\n{}  in use  XDG_STATE_HOME\n{}  no graph  HOME\n",
            home.display(),
            first.display(),
            first.display(),
            home.display()
        )),
        "{}",
        stdout(&out)
    );
    let out = seat(root.path(), true, &["list"]);
    assert!(stdout(&out).contains("started without XDG_STATE_HOME"));
    assert_eq!(stderr(&out), "");

    let out = seat(root.path(), true, &["where", "--move"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!stdout(&out).contains("moved"), "{}", stdout(&out));
}
