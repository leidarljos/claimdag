//! `ready` says how much unfinished work waits below each row.
//! `list` does not grow a field for it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::{Command, Output};

/// Fields `list --json` prints. `ready --json` is this set plus `waiting_below`.
const LIST_JSON_KEYS: &[&str] = &[
    "archived",
    "assignee",
    "cas_gen",
    "created_unix",
    "deps",
    "finished_unix",
    "id",
    "kind",
    "parent",
    "role",
    "status",
    "summary",
    "updated_unix",
];

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

fn add(dir: &Path, summary: &str) -> String {
    let out = claimdag(dir, &["upsert", "--status", "ready", "--summary", summary]);
    assert!(out.status.success(), "upsert: {}", stderr(&out));
    stdout(&out)
}

fn link(dir: &Path, parent: &str, child: &str) {
    let out = claimdag(dir, &["link", parent, child]);
    assert!(out.status.success(), "link: {}", stderr(&out));
}

fn list_keys() -> BTreeSet<String> {
    LIST_JSON_KEYS.iter().map(|k| (*k).to_string()).collect()
}

/// A chain A -> B -> C, plus a lone D. A is first and two long; D is zero.
#[test]
fn ready_rows_say_what_waits_below() {
    let dir = tempfile::tempdir().unwrap();
    let a = add(dir.path(), "a");
    let b = add(dir.path(), "b");
    let c = add(dir.path(), "c");
    let d = add(dir.path(), "d");
    // A finishes before B, and B before C. D depends on nothing.
    link(dir.path(), &a, &b);
    link(dir.path(), &b, &c);

    let out = claimdag(dir.path(), &["ready", "--json"]);
    assert!(out.status.success(), "ready --json: {}", stderr(&out));
    let ready = rows(&stdout(&out));
    assert_eq!(
        ready
            .iter()
            .map(|row| string(row, "id"))
            .collect::<Vec<_>>(),
        vec![a.clone(), d.clone()],
        "ready order: {}",
        stdout(&out)
    );
    // The chain under A is B then C, two long. Nothing waits under D.
    assert_eq!(raw(&ready[0], "waiting_below"), "2");
    assert_eq!(raw(&ready[1], "waiting_below"), "0");
    let mut ready_keys = list_keys();
    ready_keys.insert("waiting_below".to_string());
    for row in &ready {
        assert_eq!(row.keys().cloned().collect::<BTreeSet<_>>(), ready_keys);
    }

    let out = claimdag(dir.path(), &["ready"]);
    assert!(out.status.success(), "ready: {}", stderr(&out));
    let text = stdout(&out);
    let line_a = text
        .lines()
        .find(|line| line.starts_with(&a))
        .unwrap_or_else(|| panic!("no text row for A in {text}"));
    assert!(
        line_a.contains("below=2"),
        "A should say what waits below it: {line_a}"
    );
    let line_d = text
        .lines()
        .find(|line| line.starts_with(&d))
        .unwrap_or_else(|| panic!("no text row for D in {text}"));
    assert!(
        line_d.contains("below=0"),
        "D should say nothing waits below it: {line_d}"
    );

    let out = claimdag(dir.path(), &["list", "--json"]);
    assert!(out.status.success(), "list --json: {}", stderr(&out));
    let listed = rows(&stdout(&out));
    // A, B, C and D are all live, so each of the four rows keeps the old keys.
    let keys = list_keys();
    assert_eq!(listed.len(), 4, "list: {}", stdout(&out));
    for row in &listed {
        assert_eq!(row.keys().cloned().collect::<BTreeSet<_>>(), keys);
    }
}

/// One JSON value, as the seat sees it: a decoded string, or the raw token.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Json {
    String(String),
    Raw(String),
}

fn string(row: &BTreeMap<String, Json>, key: &str) -> String {
    match row.get(key) {
        Some(Json::String(s)) => s.clone(),
        other => panic!("{key} is not a string: {other:?}"),
    }
}

fn raw<'a>(row: &'a BTreeMap<String, Json>, key: &str) -> &'a str {
    match row.get(key) {
        Some(Json::Raw(s)) => s,
        other => panic!("{key} is not a raw token: {other:?}"),
    }
}

/// Top-level objects of the compact array `ready --json` and `list --json` print.
fn rows(json: &str) -> Vec<BTreeMap<String, Json>> {
    let b = json.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    skip(b, &mut i);
    eat(b, &mut i, b'[');
    loop {
        skip(b, &mut i);
        match b.get(i).copied() {
            Some(b']') => break,
            Some(b',') => i += 1,
            Some(b'{') => out.push(object(b, &mut i)),
            other => panic!("json at {i}: {other:?} in {json}"),
        }
    }
    out
}

fn object(b: &[u8], i: &mut usize) -> BTreeMap<String, Json> {
    eat(b, i, b'{');
    let mut map = BTreeMap::new();
    loop {
        skip(b, i);
        match b.get(*i).copied() {
            Some(b'}') => {
                *i += 1;
                return map;
            }
            Some(b',') => *i += 1,
            Some(b'"') => {
                let key = string_token(b, i);
                skip(b, i);
                eat(b, i, b':');
                skip(b, i);
                map.insert(key, value(b, i));
            }
            other => panic!("object at {i}: {other:?}"),
        }
    }
}

fn value(b: &[u8], i: &mut usize) -> Json {
    match b[*i] {
        b'"' => Json::String(string_token(b, i)),
        b'[' | b'{' => {
            let start = *i;
            skip_container(b, i);
            Json::Raw(bytes_to_string(&b[start..*i]))
        }
        _ => {
            let start = *i;
            while *i < b.len()
                && !matches!(b[*i], b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t')
            {
                *i += 1;
            }
            Json::Raw(bytes_to_string(&b[start..*i]))
        }
    }
}

fn string_token(b: &[u8], i: &mut usize) -> String {
    assert_eq!(b[*i], b'"');
    *i += 1;
    let mut out = Vec::new();
    loop {
        let c = b[*i];
        *i += 1;
        match c {
            b'"' => return bytes_to_string(&out),
            b'\\' => {
                let e = b[*i];
                *i += 1;
                out.push(match e {
                    b'"' | b'\\' | b'/' => e,
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'r' => b'\r',
                    _ => panic!("bad escape {e}"),
                });
            }
            c => out.push(c),
        }
    }
}

fn skip_container(b: &[u8], i: &mut usize) {
    let open = b[*i];
    let close = if open == b'[' { b']' } else { b'}' };
    let mut depth = 0;
    let mut in_string = false;
    let mut escape = false;
    while *i < b.len() {
        let c = b[*i];
        *i += 1;
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            continue;
        }
        if c == b'"' {
            in_string = true;
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return;
            }
        }
    }
    panic!("unclosed json container");
}

fn skip(b: &[u8], i: &mut usize) {
    while matches!(b.get(*i), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        *i += 1;
    }
}

fn eat(b: &[u8], i: &mut usize, want: u8) {
    assert_eq!(b.get(*i).copied(), Some(want), "at {i}");
    *i += 1;
}

fn bytes_to_string(b: &[u8]) -> String {
    String::from_utf8(b.to_vec()).expect("utf-8")
}
