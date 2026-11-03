//! Role and assignee arguments, as a seat sends them.
//!
//! `role` is a closed list. A word the graph does not store is an error that
//! names the list. A short assignee is a bad id, and the graph is unchanged.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

/// How long one reply may take.
///
/// The server is a process on this machine and a reply is one line. Five
/// seconds covers that reply and bounds a hang.
const REPLY_WAIT: Duration = Duration::from_secs(5);

/// JSON-RPC 2.0 invalid params. A handler's `invalid_params` is raised as this.
const INVALID_PARAMS: i64 = -32602;

/// The roles the graph stores, in the order the schema lists them.
const ROLES: &[&str] = &[
    "unset",
    "explore",
    "architect",
    "implementor",
    "verifier",
    "orchestrator",
    "general",
];

const READY_BALANCED: &str = "claimdag_ready_balanced";
const CLAIM_NEXT: &str = "claimdag_claim_next";

/// 32 hex characters, a real id, so a refused call fails on the field under
/// test and not on this one.
const WRITER: &str = "00000000000000070000000000000007";

/// The snapshot the server reads. Three nodes, so a claim would show in list.
const THREE_NODES: &[u8] = include_bytes!("fixtures/three-nodes.bin");

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

/// `#/$defs/Role` resolved against the tool's input schema.
fn resolve<'a>(root: &'a Value, reference: &str) -> &'a Value {
    let path = reference
        .strip_prefix("#/")
        .unwrap_or_else(|| panic!("ref {reference} in {root}"));
    let mut cur = root;
    for part in path.split('/') {
        let part = part.replace("~1", "/").replace("~0", "~");
        cur = cur
            .get(&part)
            .unwrap_or_else(|| panic!("no {part} for {reference} in {root}"));
    }
    cur
}

/// String values of every `enum` reachable from `node`, following `$ref`.
fn enums_from(root: &Value, node: &Value, seen: &mut BTreeSet<String>) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        if seen.insert(reference.to_string()) {
            found.extend(enums_from(root, resolve(root, reference), seen));
        }
        return found;
    }
    if let Some(entries) = node.get("enum").and_then(Value::as_array) {
        let names: Vec<String> = entries
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        if !names.is_empty() {
            found.push(names);
        }
    }
    match node {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "enum" || key == "$ref" {
                    continue;
                }
                found.extend(enums_from(root, child, seen));
            }
        }
        Value::Array(entries) => {
            for child in entries {
                found.extend(enums_from(root, child, seen));
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    found
}

fn role_enum(schema: &Value) -> Vec<String> {
    let role = field(field(schema, "properties"), "role");
    let mut seen = BTreeSet::new();
    let found = enums_from(schema, role, &mut seen);
    assert_eq!(found.len(), 1, "role enum in {schema}");
    found.into_iter().next().expect("one enum")
}

#[derive(Debug)]
enum Reply {
    Rpc { code: i64, message: String },
    Tool { is_error: bool, text: String },
}

struct Seat {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<String>,
    stderr: Arc<Mutex<String>>,
    next_id: i64,
}

struct Opened {
    seat: Seat,
    _dir: tempfile::TempDir,
}

impl Opened {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("scratch dir");
        // `work.bin` is the snapshot name the server opens.
        std::fs::write(dir.path().join("work.bin"), THREE_NODES).expect("seed the snapshot");
        let seat = Seat::start(dir.path());
        Self { seat, _dir: dir }
    }
}

impl Seat {
    fn start(dir: &std::path::Path) -> Self {
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
        self.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    }

    fn call(&mut self, name: &str, arguments: &str) -> Reply {
        let msg = self.rpc(
            "tools/call",
            &format!(r#"{{"name":"{name}","arguments":{arguments}}}"#),
        );
        if let Some(err) = msg.get("error") {
            return Reply::Rpc {
                code: num(field(err, "code")),
                message: text(field(err, "message")).to_string(),
            };
        }
        let result = field(&msg, "result");
        let is_error = match result.get("isError") {
            Some(Value::Bool(flag)) => *flag,
            None => false,
            Some(other) => panic!("isError {other} in {msg}"),
        };
        let block = &items(field(result, "content"))[0];
        assert_eq!(text(field(block, "type")), "text", "{msg}");
        Reply::Tool {
            is_error,
            text: text(field(block, "text")).to_string(),
        }
    }

    fn list_text(&mut self) -> String {
        match self.call("claimdag_list", "{}") {
            Reply::Tool {
                is_error: false,
                text,
            } => text,
            other => panic!("list failed: {other:?}"),
        }
    }

    fn input_schema(&mut self, name: &str) -> Value {
        let msg = self.rpc("tools/list", "{}");
        items(field(field(&msg, "result"), "tools"))
            .iter()
            .find(|tool| text(field(tool, "name")) == name)
            .unwrap_or_else(|| panic!("no {name} in {msg}"))
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| panic!("no inputSchema for {name}"))
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn assert_role_refused(seat: &mut Seat, name: &str, arguments: &str) {
    match seat.call(name, arguments) {
        Reply::Tool {
            is_error: true,
            text,
        } => {
            assert!(
                text.contains("failed to deserialize parameters"),
                "{name}: {text}"
            );
            assert!(text.contains("implementor"), "{name}: {text}");
        }
        other => panic!("{name} accepted implementer: {other:?}"),
    }
}

/// `tools/list` shows `enum` with the seven roles for `role` on both tools.
#[test]
fn tools_list_names_the_seven_roles() {
    let mut open = Opened::start();
    open.seat.handshake();
    let want: Vec<String> = ROLES.iter().map(|role| (*role).to_string()).collect();
    for name in [READY_BALANCED, CLAIM_NEXT] {
        assert_eq!(role_enum(&open.seat.input_schema(name)), want, "{name}");
    }
}

/// `implementer` is refused, and the error names `implementor`.
#[test]
fn a_misspelled_role_is_refused_and_names_implementor() {
    let mut open = Opened::start();
    open.seat.handshake();
    let before = open.seat.list_text();
    assert_role_refused(&mut open.seat, READY_BALANCED, r#"{"role":"implementer"}"#);
    assert_role_refused(
        &mut open.seat,
        CLAIM_NEXT,
        &format!(r#"{{"assignee":"{WRITER}","role":"implementer"}}"#),
    );
    assert_eq!(open.seat.list_text(), before);
}

/// A short assignee is a bad id, and list is unchanged.
#[test]
fn a_short_assignee_is_refused_and_claims_nothing() {
    let mut open = Opened::start();
    open.seat.handshake();
    let before = open.seat.list_text();
    for name in [READY_BALANCED, CLAIM_NEXT] {
        match open.seat.call(name, r#"{"assignee":"07"}"#) {
            Reply::Rpc { code, message } => {
                assert_eq!(code, INVALID_PARAMS, "{name}: {message}");
                assert!(message.contains("bad id 07"), "{name}: {message}");
            }
            other => panic!("{name} accepted assignee 07: {other:?}"),
        }
    }
    assert_eq!(open.seat.list_text(), before);
}
