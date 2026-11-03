//! Golden text of `claimdag-tui --dump`.
//!
//! The binary is what a seat runs. Each test writes a graph, dumps it, and
//! matches the whole stdout against the forest a person should read: one
//! line per node, two spaces of indent per parent that is still in the
//! graph, siblings in id order.

use std::path::Path;
use std::process::{Command, Output};

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};

/// Spaces one level sits in from its parent. The pane uses the same two
/// spaces, so a dumped line and a drawn row start the summary in the same
/// column.
const INDENT_STEP: usize = 2;

/// How deep the chain fixture goes. Fifty is enough that the last line's
/// indent is obvious (an off-by-one or a depth cutoff lands there) and
/// still a single screen of text to diff.
const CHAIN_LEVELS: usize = 50;

/// How many children the fan fixture hangs under one root. Five hundred is
/// enough that a dropped tail is not a rounding error in a short list, and
/// that a summary sort cannot pass by agreeing with a handful of rows.
const FAN_CHILDREN: usize = 500;

/// Gap before a second write. `updated_unix` is a whole second, so a gap
/// under one second can share a timestamp with the writes just made.
/// 1100ms always falls in a later second.
const LATER_EDIT: std::time::Duration = std::time::Duration::from_millis(1100);

fn nid(n: u64) -> WorkId {
    WorkId { hi: n, lo: 0 }
}

fn put(graph: &mut WorkGraph, id: WorkId, parent: WorkId, summary: &str) {
    graph
        .upsert(
            id,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent,
                actor: WorkId::ZERO,
                summary,
            },
        )
        .unwrap_or_else(|err| panic!("upsert {summary}: {err}"));
}

fn save(graph: &WorkGraph, dir: &Path) {
    graph
        .save_dir(dir)
        .unwrap_or_else(|err| panic!("save {}: {err}", dir.display()));
}

fn run_dump(dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_claimdag-tui"))
        .arg("--dir")
        .arg(dir)
        .arg("--dump")
        .output()
        .expect("run claimdag-tui --dump")
}

fn stdout_text(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("dump stdout is utf-8")
}

fn stderr_text(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("dump stderr is utf-8")
}

/// A node with no open dependency is drawn `ready`, then two spaces, then
/// its summary. That is the whole label when nobody holds it and it has no
/// role and no dependency count.
fn ready_line(depth: usize, summary: &str) -> String {
    format!("{}ready  {summary}\n", " ".repeat(depth * INDENT_STEP))
}

fn assert_golden(out: &Output, expected: &str) {
    assert!(
        out.status.success(),
        "dump exited {:?}\n{}",
        out.status.code(),
        stderr_text(out)
    );
    assert_eq!(stderr_text(out), "", "dump wrote to stderr");
    let got = stdout_text(out);
    if got == expected {
        return;
    }
    let got_lines: Vec<&str> = got.lines().collect();
    let expected_lines: Vec<&str> = expected.lines().collect();
    let rows = got_lines.len().max(expected_lines.len());
    for index in 0..rows {
        if got_lines.get(index) != expected_lines.get(index) {
            panic!(
                "dump differs at line {} of {} (expected {} lines)\n got: {:?}\n exp: {:?}",
                index + 1,
                got_lines.len(),
                expected_lines.len(),
                got_lines.get(index),
                expected_lines.get(index)
            );
        }
    }
    panic!(
        "dump matched every line but not the bytes (trailing newline or a blank line)\n got: {got:?}\n exp: {expected:?}"
    );
}

#[test]
fn an_empty_graph_dumps_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    save(&WorkGraph::default(), dir.path());
    let out = run_dump(dir.path());
    assert_golden(&out, "(empty)\n");
    let again = run_dump(dir.path());
    assert_eq!(stdout_text(&out), stdout_text(&again));
}

#[test]
fn a_chain_fifty_deep_indents_one_level_per_parent() {
    let dir = tempfile::tempdir().unwrap();
    let mut graph = WorkGraph::default();
    // Ids run backwards along the chain, so a dump sorted by id would
    // reverse the parents. The forest has to follow parent links.
    let mut parent = WorkId::ZERO;
    for depth in 0..CHAIN_LEVELS {
        let id = nid((CHAIN_LEVELS - depth) as u64);
        put(&mut graph, id, parent, &format!("level {depth:02}"));
        parent = id;
    }
    save(&graph, dir.path());

    let mut expected = String::new();
    for depth in 0..CHAIN_LEVELS {
        expected.push_str(&ready_line(depth, &format!("level {depth:02}")));
    }
    assert_golden(&run_dump(dir.path()), &expected);
}

/// A later edit must not pull a sibling above the ones with smaller ids.
#[test]
fn siblings_stay_in_id_order_when_a_later_edit_is_newer() {
    let dir = tempfile::tempdir().unwrap();
    let mut graph = WorkGraph::default();
    let root = nid(1);
    put(&mut graph, root, WorkId::ZERO, "root");
    put(&mut graph, nid(10), root, "low");
    put(&mut graph, nid(20), root, "mid");
    put(&mut graph, nid(30), root, "high");
    std::thread::sleep(LATER_EDIT);
    put(&mut graph, nid(30), root, "high");
    save(&graph, dir.path());

    let expected = [
        ready_line(0, "root"),
        ready_line(1, "low"),
        ready_line(1, "mid"),
        ready_line(1, "high"),
    ]
    .concat();
    assert_golden(&run_dump(dir.path()), &expected);
}

#[test]
fn a_fan_five_hundred_wide_lists_every_child_in_id_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut graph = WorkGraph::default();
    let root = nid(1);
    put(&mut graph, root, WorkId::ZERO, "fan");
    // Insert high ids first. Id order is the opposite direction, and it is
    // the order the dump has to keep: stable for a given graph, and not the
    // order the nodes were written.
    for index in (0..FAN_CHILDREN).rev() {
        put(
            &mut graph,
            nid(2 + index as u64),
            root,
            &format!("n{index:03}"),
        );
    }
    save(&graph, dir.path());

    let mut expected = ready_line(0, "fan");
    for index in 0..FAN_CHILDREN {
        expected.push_str(&ready_line(1, &format!("n{index:03}")));
    }
    let out = run_dump(dir.path());
    assert_golden(&out, &expected);
    let again = run_dump(dir.path());
    assert_eq!(
        stdout_text(&out),
        stdout_text(&again),
        "a second dump moved"
    );
}

#[test]
fn summaries_keep_cjk_emoji_and_combining_marks() {
    // Stored as scalars, not as source glyphs, so a dump that normalises or
    // drops a combiner cannot accidentally match the fixture.
    let thumbs = "\u{1F44D}\u{1F3FD}";
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let cjk = "\u{6E2C}\u{5B9A}";
    let two_marks = "a\u{0308}\u{0301}";
    let cafe = "cafe\u{0301}";
    let heart = "\u{2764}\u{FE0F}";

    let dir = tempfile::tempdir().unwrap();
    let mut graph = WorkGraph::default();
    let emoji = nid(10);
    let measured = nid(20);
    let coffee = nid(30);
    // Written in an order that is neither id order nor summary order.
    put(&mut graph, coffee, WorkId::ZERO, cafe);
    put(&mut graph, nid(31), coffee, heart);
    put(&mut graph, measured, WorkId::ZERO, cjk);
    put(&mut graph, nid(21), measured, two_marks);
    put(&mut graph, emoji, WorkId::ZERO, thumbs);
    put(&mut graph, nid(11), emoji, family);
    save(&graph, dir.path());

    let expected = [
        ready_line(0, thumbs),
        ready_line(1, family),
        ready_line(0, cjk),
        ready_line(1, two_marks),
        ready_line(0, cafe),
        ready_line(1, heart),
    ]
    .concat();
    assert_golden(&run_dump(dir.path()), &expected);
}

#[test]
fn a_node_whose_parent_was_pruned_dumps_as_a_root() {
    // Prune removes a finished node and leaves the parent id on its child.
    // The child is still in the graph, so the dump has to show it, at the
    // root, because the parent it names is no longer there to hang under.
    let dir = tempfile::tempdir().unwrap();
    let mut graph = WorkGraph::default();
    let gone = nid(9);
    let cut = nid(2);
    let top = nid(3);
    put(&mut graph, top, WorkId::ZERO, "top");
    put(&mut graph, nid(4), top, "nested");
    put(&mut graph, cut, gone, "cut loose");
    put(&mut graph, nid(5), cut, "still under the orphan");
    save(&graph, dir.path());

    let expected = [
        ready_line(0, "cut loose"),
        ready_line(1, "still under the orphan"),
        ready_line(0, "top"),
        ready_line(1, "nested"),
    ]
    .concat();
    assert_golden(&run_dump(dir.path()), &expected);
}
