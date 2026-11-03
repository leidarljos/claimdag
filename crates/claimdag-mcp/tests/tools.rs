//! Every tool the MCP server serves, called the way a seat calls it.
//!
//! The test starts `claimdag-mcp` on a scratch directory and speaks JSON-RPC
//! on its stdin. A good call checks the fields of the answer. A bad call
//! checks the error text. Renaming a field of either one fails the test.

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

/// JSON-RPC 2.0 invalid params. An id that is not 32 hex characters, or a
/// status outside done, failed and cancelled, is this. A graph refusal is
/// a tool result instead.
const INVALID_PARAMS: i64 = -32602;

/// A generation no node in the fixture is at. Passing it shows the
/// generation field was read, because the claim is refused.
const STALE_GENERATION: i64 = 99;

/// `...0001`, ready, implementor, "write the parser". The other node waits
/// on it, so it is the head of the chain.
const PARSER: &str = "00000000000000010000000000000001";
/// `...0002`, ready, explore, "read the spec".
const SPEC: &str = "00000000000000020000000000000002";
/// `...0003`, "ship it", waiting on the parser, so it starts as todo.
const SHIP: &str = "00000000000000030000000000000003";
const WRITER: &str = "00000000000000070000000000000007";
const READER: &str = "00000000000000080000000000000008";

const PARSER_SUMMARY: &str = "write the parser";
const SPEC_SUMMARY: &str = "read the spec";
const SHIP_SUMMARY: &str = "ship it";
const DONE_SUMMARY: &str = "parser landed";

/// The snapshot the server reads. Three nodes, generations starting at 1,
/// with the ship node depending on the parser.
const THREE_NODES: &[u8] = include_bytes!("fixtures/three-nodes.bin");

const NODE_FIELDS: &[&str] = &[
    "assignee",
    "blocked_by",
    "generation",
    "id",
    "quiet_seconds",
    "status",
    "summary",
    "waiting_below",
];
const CLAIM_FIELDS: &[&str] = &["generation", "id"];

const TOOLS: &[&str] = &[
    "claimdag_claim",
    "claimdag_claim_next",
    "claimdag_complete",
    "claimdag_list",
    "claimdag_ready",
    "claimdag_ready_balanced",
    "claimdag_reclaim",
    "claimdag_release",
    "claimdag_renew",
    "claimdag_reopen",
];

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

fn id_of(row: &Value) -> &str {
    text(field(row, "id"))
}

fn property_names(schema: &Value) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    fn walk(value: &Value, names: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if key == "properties" {
                        if let Some(props) = child.as_object() {
                            names.extend(props.keys().cloned());
                        }
                    }
                    walk(child, names);
                }
            }
            Value::Array(entries) => {
                for entry in entries {
                    walk(entry, names);
                }
            }
            _ => {}
        }
    }
    walk(schema, &mut names);
    names
}

fn field_set(fields: &[&str]) -> BTreeSet<String> {
    fields.iter().map(|field| (*field).to_string()).collect()
}

fn input_fields(tool: &str) -> &'static [&'static str] {
    match tool {
        "claimdag_ready" => &[],
        "claimdag_ready_balanced" => &["assignee", "balanced", "role", "slack"],
        "claimdag_list" => &["all"],
        "claimdag_claim" => &["assignee", "generation", "id"],
        "claimdag_claim_next" => &["assignee", "role", "slack"],
        "claimdag_renew" | "claimdag_release" | "claimdag_reopen" => &["actor", "id"],
        "claimdag_complete" => &["actor", "generation", "id", "status", "summary"],
        "claimdag_reclaim" => &["lease_seconds"],
        other => panic!("no input fields for {other}"),
    }
}

fn output_fields(tool: &str) -> &'static [&'static str] {
    match tool {
        "claimdag_ready" | "claimdag_ready_balanced" | "claimdag_list" | "claimdag_complete" => {
            NODE_FIELDS
        }
        "claimdag_claim"
        | "claimdag_claim_next"
        | "claimdag_renew"
        | "claimdag_release"
        | "claimdag_reopen" => CLAIM_FIELDS,
        // A list of id strings has no object fields.
        "claimdag_reclaim" => &[],
        other => panic!("no output fields for {other}"),
    }
}

fn assert_keys(value: &Value, fields: &[&str]) {
    let mut got: Vec<&str> = value
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {value}"))
        .keys()
        .map(String::as_str)
        .collect();
    got.sort_unstable();
    let mut want: Vec<&str> = fields.to_vec();
    want.sort_unstable();
    assert_eq!(got, want, "{value}");
}

struct Expect<'a> {
    id: &'a str,
    status: &'a str,
    summary: &'a str,
    assignee: Option<&'a str>,
    generation: i64,
    blocked_by: &'a [&'a str],
    /// A held node reports how long it has been quiet. The count moves
    /// with the clock, so the test only checks that it is a number.
    held: bool,
    waiting_below: i64,
}

fn assert_node(row: &Value, expect: Expect<'_>) {
    assert_keys(row, NODE_FIELDS);
    assert_eq!(id_of(row), expect.id);
    assert_eq!(text(field(row, "status")), expect.status);
    assert_eq!(text(field(row, "summary")), expect.summary);
    match expect.assignee {
        Some(id) => assert_eq!(text(field(row, "assignee")), id),
        None => assert!(field(row, "assignee").is_null(), "{row}"),
    }
    assert_eq!(num(field(row, "generation")), expect.generation);
    let blocked: Vec<&str> = items(field(row, "blocked_by")).iter().map(text).collect();
    assert_eq!(blocked, expect.blocked_by);
    match field(row, "quiet_seconds") {
        Value::Null => assert!(!expect.held, "a held node omitted quiet_seconds: {row}"),
        Value::Number(seconds) => {
            assert!(expect.held, "quiet_seconds on a free node: {row}");
            assert!(seconds.as_i64().unwrap_or(-1) >= 0, "{row}");
        }
        other => panic!("quiet_seconds {other}"),
    }
    assert_eq!(num(field(row, "waiting_below")), expect.waiting_below);
}

fn assert_claim(value: &Value, id: &str, generation: i64) {
    assert_keys(value, CLAIM_FIELDS);
    assert_eq!(id_of(value), id);
    assert_eq!(num(field(value, "generation")), generation);
}

fn find<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter()
        .find(|row| id_of(row) == id)
        .unwrap_or_else(|| panic!("no {id} in {rows:?}"))
}

fn ids(rows: &[Value]) -> Vec<&str> {
    rows.iter().map(id_of).collect()
}

struct Listed {
    name: String,
    description: String,
    input: Value,
    output: Value,
}

#[derive(Debug)]
enum Answer {
    Rpc {
        code: i64,
        message: String,
        raw: String,
    },
    Tool {
        is_error: bool,
        text: String,
        structured: Option<Value>,
        raw: String,
    },
}

struct Seat {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<String>,
    stderr: Arc<Mutex<String>>,
    next_id: i64,
}

impl Seat {
    fn start(dir: &std::path::Path) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_claimdag-mcp"));
        // Cargo names this variable after the binary, hyphen included.
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

    fn rpc(&mut self, method: &str, params: &str) -> (String, Value) {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#
        ));
        loop {
            let raw = self.read_line();
            let msg = parse(&raw);
            match msg.get("id").map(num) {
                Some(got) if got == id => return (raw, msg),
                Some(_) => panic!("reply for another id while waiting for {id}: {raw}"),
                None if msg.get("method").is_some() => continue,
                None => panic!("reply without an id while waiting for {id}: {raw}"),
            }
        }
    }

    fn notify(&mut self, method: &str) {
        self.send(&format!(r#"{{"jsonrpc":"2.0","method":"{method}"}}"#));
    }

    /// `2025-11-25` is a protocol revision this server still answers with an
    /// initialize result.
    fn handshake(&mut self) {
        let (_raw, hello) = self.rpc(
            "initialize",
            r#"{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"seat","version":"0"}}"#,
        );
        let result = field(&hello, "result");
        assert_eq!(text(field(result, "protocolVersion")), "2025-11-25");
        assert_eq!(text(field(field(result, "serverInfo"), "name")), "claimdag");
        assert!(field(result, "capabilities").get("tools").is_some());
        assert!(
            text(field(result, "instructions")).contains("generation"),
            "instructions"
        );
        self.notify("notifications/initialized");
    }

    fn listed_tools(&mut self) -> Vec<Listed> {
        let (_raw, msg) = self.rpc("tools/list", "{}");
        items(field(field(&msg, "result"), "tools"))
            .iter()
            .map(|tool| Listed {
                name: text(field(tool, "name")).to_string(),
                description: tool
                    .get("description")
                    .map(text)
                    .unwrap_or_default()
                    .to_string(),
                input: field(tool, "inputSchema").clone(),
                output: tool.get("outputSchema").cloned().unwrap_or(Value::Null),
            })
            .collect()
    }

    fn tool(&mut self, name: &str, arguments: &str) -> Answer {
        let (raw, msg) = self.rpc(
            "tools/call",
            &format!(r#"{{"name":"{name}","arguments":{arguments}}}"#),
        );
        if let Some(err) = msg.get("error") {
            return Answer::Rpc {
                code: num(field(err, "code")),
                message: text(field(err, "message")).to_string(),
                raw,
            };
        }
        let result = field(&msg, "result");
        let is_error = match result.get("isError") {
            Some(Value::Bool(flag)) => *flag,
            None => false,
            Some(other) => panic!("isError {other} in {raw}"),
        };
        let block = &items(field(result, "content"))[0];
        assert_eq!(text(field(block, "type")), "text", "{raw}");
        Answer::Tool {
            is_error,
            text: text(field(block, "text")).to_string(),
            structured: result.get("structuredContent").cloned(),
            raw,
        }
    }

    fn answer(&mut self, name: &str, arguments: &str) -> Value {
        match self.tool(name, arguments) {
            Answer::Tool {
                is_error: false,
                text,
                structured: Some(value),
                raw,
            } => {
                assert_eq!(
                    parse(&text),
                    value,
                    "{name}: text and structuredContent differ\n{raw}"
                );
                value
            }
            Answer::Tool { raw, .. } => panic!("{name} returned an error result\n{raw}"),
            Answer::Rpc { code, message, raw } => panic!("{name} raised {code} {message}\n{raw}"),
        }
    }

    fn rows(&mut self, name: &str, arguments: &str) -> Vec<Value> {
        let value = self.answer(name, arguments);
        let rows = items(&value).to_vec();
        for row in &rows {
            assert_keys(row, NODE_FIELDS);
        }
        rows
    }

    fn tool_err(&mut self, name: &str, arguments: &str, phrase: &str) {
        match self.tool(name, arguments) {
            Answer::Tool {
                is_error: true,
                text,
                raw,
                ..
            } => {
                assert!(
                    text.contains("failed to deserialize parameters"),
                    "{name}: {text}\n{raw}"
                );
                assert!(text.contains(phrase), "{name}: {text}\n{raw}");
            }
            Answer::Tool { raw, .. } => panic!("{name} succeeded\n{raw}"),
            Answer::Rpc { code, message, raw } => panic!("{name} raised {code} {message}\n{raw}"),
        }
    }

    fn rpc_err(&mut self, name: &str, arguments: &str, code: i64, prefix: &str) {
        match self.tool(name, arguments) {
            Answer::Rpc {
                code: got,
                message: text,
                raw,
            } => {
                assert_eq!(got, code, "{name}\n{raw}");
                assert!(text.starts_with(prefix), "{name}: {text}\n{raw}");
            }
            Answer::Tool { raw, .. } => panic!("{name} returned a tool result\n{raw}"),
        }
    }

    /// A graph refusal is a tool result the seat reads, not a JSON-RPC error.
    /// `phrase` is the start of that text.
    fn refused(&mut self, name: &str, arguments: &str, phrase: &str) {
        match self.tool(name, arguments) {
            Answer::Tool {
                is_error: true,
                text,
                raw,
                ..
            } => {
                assert!(text.starts_with(phrase), "{name}: {text}\n{raw}");
            }
            Answer::Tool { raw, .. } => panic!("{name} succeeded\n{raw}"),
            Answer::Rpc { code, message, raw } => panic!("{name} raised {code} {message}\n{raw}"),
        }
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A seat starts the server, lists the tools, and calls each one.
#[test]
fn every_tool_returns_an_answer_and_an_error() {
    let dir = tempfile::tempdir().expect("scratch dir");
    let mut seat = Seat::start(dir.path());
    seat.handshake();

    let listed = seat.listed_tools();
    let names: Vec<&str> = listed.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, TOOLS);
    for tool in &listed {
        assert!(!tool.description.is_empty(), "{}", tool.name);
        assert_eq!(
            property_names(&tool.input),
            field_set(input_fields(&tool.name)),
            "input fields of {}",
            tool.name
        );
        assert_eq!(
            property_names(&tool.output),
            field_set(output_fields(&tool.name)),
            "output fields of {}",
            tool.name
        );
    }

    // The directory exists and holds no snapshot. That is a missing graph,
    // which is a different answer from an empty one.
    let missing = format!(
        "no work graph at {}: the directory holds no work.bin. Claim something to create it",
        dir.path().display()
    );
    seat.refused("claimdag_ready", "{}", &missing);
    // `work.bin` is the snapshot name the server opens.
    std::fs::write(dir.path().join("work.bin"), THREE_NODES).expect("seed the snapshot");

    let ready = seat.rows("claimdag_ready", "{}");
    assert_eq!(ids(&ready), [PARSER, SPEC]);
    assert_node(
        &ready[0],
        Expect {
            id: PARSER,
            status: "ready",
            summary: PARSER_SUMMARY,
            assignee: None,
            generation: 1,
            blocked_by: &[],
            held: false,
            waiting_below: 1,
        },
    );
    assert_node(
        &ready[1],
        Expect {
            id: SPEC,
            status: "ready",
            summary: SPEC_SUMMARY,
            assignee: None,
            generation: 1,
            blocked_by: &[],
            held: false,
            waiting_below: 0,
        },
    );

    // `slack` is a number. A string is refused at that field.
    seat.tool_err(
        "claimdag_ready_balanced",
        r#"{"slack":"wide"}"#,
        "invalid type",
    );
    // Slack of 1 is the server's own default, and it is wide enough that
    // the shorter chain shares a band with the head, so role order shows.
    let balanced = seat.rows(
        "claimdag_ready_balanced",
        &format!(r#"{{"role":"explore","assignee":"{READER}","balanced":true,"slack":1}}"#),
    );
    assert_eq!(ids(&balanced), [SPEC, PARSER]);
    // `balanced: false` asks for depth order. The call sets the field, and
    // the head of the chain comes first.
    let depth_order = seat.rows(
        "claimdag_ready_balanced",
        &format!(r#"{{"role":"explore","assignee":"{READER}","balanced":false,"slack":1}}"#),
    );
    assert_eq!(ids(&depth_order), [PARSER, SPEC]);

    seat.tool_err("claimdag_list", r#"{"all":"yes"}"#, "invalid type");
    let live = seat.rows("claimdag_list", r#"{"all":false}"#);
    assert_eq!(ids(&live), [PARSER, SPEC, SHIP]);
    assert_node(
        find(&live, SHIP),
        Expect {
            id: SHIP,
            status: "todo",
            summary: SHIP_SUMMARY,
            assignee: None,
            generation: 1,
            blocked_by: &[PARSER],
            held: false,
            waiting_below: 0,
        },
    );

    // An id that is not 32 hex characters never reaches the graph.
    seat.rpc_err(
        "claimdag_claim",
        &format!(r#"{{"id":"zz","assignee":"{WRITER}"}}"#),
        INVALID_PARAMS,
        "bad id zz",
    );
    seat.refused(
        "claimdag_claim",
        &format!(r#"{{"id":"{PARSER}","assignee":"{WRITER}","generation":{STALE_GENERATION}}}"#),
        "claim: gen mismatch",
    );
    let claimed = seat.answer(
        "claimdag_claim",
        &format!(r#"{{"id":"{PARSER}","assignee":"{WRITER}","generation":1}}"#),
    );
    assert_claim(&claimed, PARSER, 2);
    // The parser is held. A second claim is a refusal the seat can read.
    seat.refused(
        "claimdag_claim",
        &format!(r#"{{"id":"{PARSER}","assignee":"{READER}"}}"#),
        "claim:",
    );
    let ready = seat.rows("claimdag_ready", "{}");
    assert_eq!(ids(&ready), [SPEC]);

    seat.tool_err("claimdag_claim_next", "{}", "missing field `assignee`");
    let next = seat.answer(
        "claimdag_claim_next",
        &format!(r#"{{"assignee":"{READER}","role":"explore","slack":1}}"#),
    );
    assert_claim(&next, SPEC, 2);

    seat.refused(
        "claimdag_renew",
        &format!(r#"{{"id":"{SPEC}","actor":"{WRITER}"}}"#),
        "renew: not assignee",
    );
    let renewed = seat.answer(
        "claimdag_renew",
        &format!(r#"{{"id":"{PARSER}","actor":"{WRITER}"}}"#),
    );
    assert_claim(&renewed, PARSER, 2);
    let live = seat.rows("claimdag_list", r#"{"all":false}"#);
    assert_node(
        find(&live, PARSER),
        Expect {
            id: PARSER,
            status: "claimed",
            summary: PARSER_SUMMARY,
            assignee: Some(WRITER),
            generation: 2,
            blocked_by: &[],
            held: true,
            waiting_below: 1,
        },
    );

    seat.refused(
        "claimdag_release",
        &format!(r#"{{"id":"{PARSER}","actor":"{READER}"}}"#),
        "release: not assignee",
    );
    let released = seat.answer(
        "claimdag_release",
        &format!(r#"{{"id":"{SPEC}","actor":"{READER}"}}"#),
    );
    assert_claim(&released, SPEC, 3);
    let ready = seat.rows("claimdag_ready", "{}");
    assert_eq!(ids(&ready), [SPEC]);
    assert_node(
        &ready[0],
        Expect {
            id: SPEC,
            status: "ready",
            summary: SPEC_SUMMARY,
            assignee: None,
            generation: 3,
            blocked_by: &[],
            held: false,
            waiting_below: 0,
        },
    );

    seat.rpc_err(
        "claimdag_complete",
        &format!(r#"{{"id":"{PARSER}","actor":"{WRITER}","status":"nope","generation":2}}"#),
        INVALID_PARAMS,
        "status is done, failed or cancelled, not nope",
    );

    seat.tool_err("claimdag_reclaim", "{}", "missing field `lease_seconds`");
    // A lease of zero is already past, so the claim just taken is handed
    // back on this call. The answer is that id.
    let handed = seat.answer("claimdag_reclaim", r#"{"lease_seconds":0}"#);
    let handed_ids: Vec<&str> = items(&handed).iter().map(text).collect();
    assert_eq!(handed_ids, [PARSER]);
    let ready = seat.rows("claimdag_ready", "{}");
    assert_eq!(ids(&ready), [PARSER, SPEC]);
    assert_node(
        &ready[0],
        Expect {
            id: PARSER,
            status: "ready",
            summary: PARSER_SUMMARY,
            assignee: None,
            generation: 3,
            blocked_by: &[],
            held: false,
            waiting_below: 1,
        },
    );

    let claimed = seat.answer(
        "claimdag_claim",
        &format!(r#"{{"id":"{PARSER}","assignee":"{WRITER}","generation":3}}"#),
    );
    assert_claim(&claimed, PARSER, 4);
    let done = seat.answer(
        "claimdag_complete",
        &format!(
            r#"{{"id":"{PARSER}","actor":"{WRITER}","status":"done","summary":"{DONE_SUMMARY}","generation":4}}"#
        ),
    );
    assert_node(
        &done,
        Expect {
            id: PARSER,
            status: "done",
            summary: DONE_SUMMARY,
            assignee: Some(WRITER),
            generation: 4,
            blocked_by: &[],
            held: false,
            waiting_below: 1,
        },
    );

    // Live is the unfinished work. The parser is done, so it is gone here
    // and comes back only when the caller asks for everything.
    let live = seat.rows("claimdag_list", r#"{"all":false}"#);
    assert_eq!(ids(&live), [SPEC, SHIP]);
    assert_node(
        find(&live, SHIP),
        Expect {
            id: SHIP,
            status: "ready",
            summary: SHIP_SUMMARY,
            assignee: None,
            generation: 1,
            blocked_by: &[PARSER],
            held: false,
            waiting_below: 0,
        },
    );
    let all = seat.rows("claimdag_list", r#"{"all":true}"#);
    assert_eq!(ids(&all), [PARSER, SPEC, SHIP]);
    assert_node(
        find(&all, PARSER),
        Expect {
            id: PARSER,
            status: "done",
            summary: DONE_SUMMARY,
            assignee: Some(WRITER),
            generation: 4,
            blocked_by: &[],
            held: false,
            waiting_below: 1,
        },
    );

    seat.refused(
        "claimdag_reopen",
        &format!(r#"{{"id":"{SPEC}","actor":"{READER}"}}"#),
        "reopen: status ready is not terminal",
    );
    let reopened = seat.answer(
        "claimdag_reopen",
        &format!(r#"{{"id":"{PARSER}","actor":"{WRITER}"}}"#),
    );
    assert_claim(&reopened, PARSER, 5);
    let live = seat.rows("claimdag_list", r#"{"all":true}"#);
    assert_node(
        find(&live, PARSER),
        Expect {
            id: PARSER,
            status: "ready",
            summary: DONE_SUMMARY,
            assignee: None,
            generation: 5,
            blocked_by: &[],
            held: false,
            waiting_below: 1,
        },
    );
}
