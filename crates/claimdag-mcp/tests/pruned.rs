//! A node the graph pruned after it finished is still finished over MCP.
//! Completing it returns that status, and releasing it answers generation 0.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};
use serde_json::Value;

const SEAT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// No cap inside the budget in measurements/node-cap.org comes near this
/// many nodes, so a graph that has pruned nothing by then has no cap.
const NO_CAP: usize = 1 << 16;

/// How long one reply may take.
///
/// The server is a process on this machine and a reply is one line. Five
/// seconds covers that reply and bounds a hang.
const REPLY_WAIT: Duration = Duration::from_secs(5);

fn add(graph: &mut WorkGraph, summary: &str) -> WorkId {
    graph
        .upsert(
            WorkId::ZERO,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: WorkId::ZERO,
                summary,
            },
        )
        .expect("upsert")
}

/// A graph in `dir` that finished one node, then took on live work until
/// prune dropped it. Returns the dropped node.
fn finished_then_pruned(dir: &Path) -> String {
    let seat = WorkId::from_hex(SEAT).unwrap();
    let mut graph = WorkGraph::default();
    let first = add(&mut graph, "finished first");
    graph.claim(first, seat, None).expect("claim");
    graph
        .complete(first, WorkStatus::Done, "", seat, None)
        .expect("complete");
    let mut made = 1;
    while graph.get(first).is_some() {
        assert!(made < NO_CAP, "nothing was pruned past {made} nodes");
        add(&mut graph, &format!("live {made}"));
        made += 1;
    }
    graph.save_dir(dir).expect("save");
    first.to_hex()
}

fn parse(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|err| panic!("{err} in {raw}"))
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value
        .get(key)
        .unwrap_or_else(|| panic!("no {key} in {value}"))
}

fn text(value: &Value) -> &str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("not a string: {value}"))
}

fn num(value: &Value) -> i64 {
    value
        .as_i64()
        .unwrap_or_else(|| panic!("not an int: {value}"))
}

fn items(value: &Value) -> &[Value] {
    value
        .as_array()
        .unwrap_or_else(|| panic!("not an array: {value}"))
}

struct Seat {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<String>,
    stderr: Arc<Mutex<String>>,
    next_id: i64,
}

impl Seat {
    fn start(dir: &Path) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_claimdag-mcp"));
        cmd.env("CLAIMDAG_DIR", dir)
            .env_remove("RUST_LOG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("start claimdag-mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let stderr_pipe = child.stderr.take().expect("stderr");
        let (tx, replies) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(line.trim_end().to_string()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let stderr = Arc::new(Mutex::new(String::new()));
        let err_buf = Arc::clone(&stderr);
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr_pipe);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        err_buf
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                            .push_str(&line);
                    }
                }
            }
        });
        Self {
            child,
            stdin,
            replies,
            stderr,
            next_id: 1,
        }
    }

    fn stderr_text(&self) -> String {
        self.stderr
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    fn send(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").expect("write to the server");
        self.stdin.flush().expect("flush to the server");
    }

    fn read_line(&self) -> String {
        self.replies.recv_timeout(REPLY_WAIT).unwrap_or_else(|err| {
            panic!("no reply ({err}); stderr: {}", self.stderr_text());
        })
    }

    fn rpc(&mut self, method: &str, params: &str) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#
        ));
        loop {
            let raw = self.read_line();
            let msg = parse(&raw);
            match msg.get("id").map(num) {
                Some(got) if got == id => return msg,
                Some(_) => panic!("reply for another id while waiting for {id}: {raw}"),
                None if msg.get("method").is_some() => continue,
                None => panic!("reply without an id while waiting for {id}: {raw}"),
            }
        }
    }

    fn handshake(&mut self) {
        let hello = self.rpc(
            "initialize",
            r#"{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"seat","version":"0"}}"#,
        );
        let result = field(&hello, "result");
        assert_eq!(text(field(result, "protocolVersion")), "2025-11-25");
        assert_eq!(text(field(field(result, "serverInfo"), "name")), "claimdag");
        self.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    }

    /// The structured answer of one tool call. An error is the failure.
    fn call(&mut self, name: &str, arguments: &str) -> Value {
        let msg = self.rpc(
            "tools/call",
            &format!(r#"{{"name":"{name}","arguments":{arguments}}}"#),
        );
        if let Some(err) = msg.get("error") {
            panic!(
                "{name} raised {} {}\n{msg}",
                num(field(err, "code")),
                text(field(err, "message"))
            );
        }
        let result = field(&msg, "result");
        if result.get("isError") == Some(&Value::Bool(true)) {
            panic!("{name} returned an error result\n{msg}");
        }
        let block = &items(field(result, "content"))[0];
        assert_eq!(text(field(block, "type")), "text", "{msg}");
        let structured = result
            .get("structuredContent")
            .cloned()
            .unwrap_or_else(|| panic!("{name} returned no structuredContent\n{msg}"));
        assert_eq!(
            parse(text(field(block, "text"))),
            structured,
            "{name}: text and structuredContent differ\n{msg}"
        );
        structured
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn completing_a_pruned_node_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let id = finished_then_pruned(dir.path());
    let mut seat = Seat::start(dir.path());
    seat.handshake();

    // Claim moved this node from generation 1 to 2. That is the token a
    // holder still carries when it retries the finish after prune.
    let done = seat.call(
        "claimdag_complete",
        &format!(r#"{{"id":"{id}","actor":"{SEAT}","generation":2}}"#),
    );
    assert_eq!(text(field(&done, "id")), id);
    assert_eq!(text(field(&done, "status")), "done");
    assert_eq!(text(field(&done, "summary")), "");
    assert!(field(&done, "assignee").is_null(), "{done}");
    assert_eq!(num(field(&done, "generation")), 0);
    assert!(items(field(&done, "blocked_by")).is_empty(), "{done}");
    assert!(field(&done, "quiet_seconds").is_null(), "{done}");
    assert_eq!(num(field(&done, "waiting_below")), 0);

    let released = seat.call(
        "claimdag_release",
        &format!(r#"{{"id":"{id}","actor":"{SEAT}"}}"#),
    );
    assert_eq!(text(field(&released, "id")), id);
    assert_eq!(num(field(&released, "generation")), 0);
}
