//! A server that is already running follows `claimdag where --move`.
//!
//! The server resolves a directory when it starts. The move then puts the
//! graph in an earlier default, in another process. A server that kept the
//! path from startup would read the old directory, and the next claim would
//! create a second graph there.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};
use serde_json::Value;

/// How long one reply may take.
///
/// The server is a process on this machine and a reply is one line. Five
/// seconds covers that reply and bounds a hang.
const REPLY_WAIT: Duration = Duration::from_secs(5);

const SUMMARY: &str = "moved with the graph";
/// A holder that is not the zero id, so the claim is one the graph records.
const ASSIGNEE: &str = "00000000000000070000000000000007";

fn euid() -> u32 {
    let out = Command::new("id").arg("-u").output().expect("id -u");
    let text = String::from_utf8(out.stdout).expect("uid");
    text.trim().parse().expect("uid number")
}

/// `/tmp/claimdag-UID`, parked for the test and put back afterwards.
///
/// That path is the real per-user default, not a directory under the temp
/// home. A graph already there belongs to whoever was using the seat, so it
/// is moved aside for the test and restored when the test drops this.
struct Parked {
    path: PathBuf,
    backup: Option<PathBuf>,
}

impl Parked {
    fn uid() -> Self {
        let path = PathBuf::from(format!("/tmp/claimdag-{}", euid()));
        let backup = if path.exists() {
            let backup = std::env::temp_dir().join(format!(
                "claimdag-uid-park-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::rename(&path, &backup)
                .unwrap_or_else(|err| panic!("park {}: {err}", path.display()));
            Some(backup)
        } else {
            None
        };
        Self { path, backup }
    }
}

impl Drop for Parked {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
        let _ = std::fs::remove_file(&self.path);
        if let Some(backup) = &self.backup {
            if let Err(err) = std::fs::rename(backup, &self.path) {
                eprintln!(
                    "could not restore {} from {}: {err}",
                    self.path.display(),
                    backup.display()
                );
            }
        }
    }
}

/// The variables the resolver reads, pointed at a private home.
///
/// The real `HOME` and `XDG_STATE_HOME` stay untouched. `XDG_STATE_HOME` is
/// unset, as is the runtime directory and `CLAIMDAG_DIR`, so the defaults
/// are the private home and then `/tmp/claimdag-UID`.
struct SeatEnv {
    keys: [&'static str; 4],
    saved: Vec<Option<OsString>>,
}

impl SeatEnv {
    fn private_home(home: &Path) -> Self {
        let keys = ["HOME", "XDG_STATE_HOME", "XDG_RUNTIME_DIR", "CLAIMDAG_DIR"];
        let saved = keys.iter().map(std::env::var_os).collect();
        std::env::set_var("HOME", home);
        std::env::remove_var("XDG_STATE_HOME");
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var("CLAIMDAG_DIR");
        Self { keys, saved }
    }
}

impl Drop for SeatEnv {
    fn drop(&mut self) {
        for (key, value) in self.keys.iter().zip(self.saved.drain(..)) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn apply_seat(cmd: &mut Command, home: &Path) {
    cmd.env("HOME", home)
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("CLAIMDAG_DIR")
        .env_remove("RUST_LOG");
}

fn seed(dir: &Path) -> WorkId {
    let mut graph = WorkGraph::load_for_write(dir).unwrap_or_else(|err| panic!("{err}"));
    let id = graph
        .upsert(
            WorkId::ZERO,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: WorkId { hi: 1, lo: 1 },
                summary: SUMMARY,
            },
        )
        .unwrap_or_else(|err| panic!("{err}"));
    graph.save_dir(dir).unwrap_or_else(|err| panic!("{err}"));
    id
}

/// What `claimdag where --move` runs: locate the defaults, then move.
///
/// The command prints a line and then the locations. The move itself is
/// this call, run with the environment the server was given.
fn move_default_graph() -> (PathBuf, PathBuf) {
    let located = claimdag::locate(None);
    match located.move_graph() {
        Ok(Some(moved)) => moved,
        Ok(None) => panic!("nothing to move\n{located}"),
        Err(err) => panic!("{err}\n{located}"),
    }
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
    fn start(home: &Path) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_claimdag-mcp"));
        apply_seat(&mut cmd, home);
        cmd.stdin(Stdio::piped())
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
            match msg.get("id").and_then(Value::as_i64) {
                Some(got) if got == id => return msg,
                Some(_) => panic!("reply for another id while waiting for {id}: {raw}"),
                None if msg.get("method").is_some() => continue,
                None => panic!("reply without an id while waiting for {id}: {raw}"),
            }
        }
    }

    fn notify(&mut self, method: &str) {
        self.send(&format!(r#"{{"jsonrpc":"2.0","method":"{method}"}}"#));
    }

    fn handshake(&mut self) {
        let hello = self.rpc(
            "initialize",
            r#"{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"seat","version":"0"}}"#,
        );
        let result = field(&hello, "result");
        assert_eq!(text(field(result, "protocolVersion")), "2025-11-25");
        self.notify("notifications/initialized");
    }

    fn tool(&mut self, name: &str, arguments: &str) -> Value {
        let msg = self.rpc(
            "tools/call",
            &format!(r#"{{"name":"{name}","arguments":{arguments}}}"#),
        );
        if let Some(err) = msg.get("error") {
            panic!(
                "{name} raised {} {}\nstderr: {}",
                field(err, "code"),
                text(field(err, "message")),
                self.stderr_text()
            );
        }
        let result = field(&msg, "result");
        if result.get("isError") == Some(&Value::Bool(true)) {
            panic!("{name} returned an error\n{msg}");
        }
        let block = &items(field(result, "content"))[0];
        let body = text(field(block, "text"));
        let parsed = parse(body);
        if let Some(structured) = result.get("structuredContent") {
            assert_eq!(
                &parsed, structured,
                "{name}: text and structuredContent differ"
            );
        }
        parsed
    }

    fn ready(&mut self) -> Vec<Value> {
        let value = self.tool("claimdag_ready", "{}");
        items(&value).to_vec()
    }

    fn claim(&mut self, id: &str, assignee: &str) -> Value {
        self.tool(
            "claimdag_claim",
            &format!(r#"{{"id":"{id}","assignee":"{assignee}"}}"#),
        )
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The graph starts in `/tmp/claimdag-UID`, with `HOME` private and
/// `XDG_STATE_HOME` unset. After the move, ready still returns the node and
/// a claim lands in the new directory. The old directory holds no `work.bin`.
#[test]
fn a_server_started_before_a_move_reads_and_writes_the_moved_graph() {
    let parked = Parked::uid();
    let root = tempfile::tempdir().expect("private home");
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let _env = SeatEnv::private_home(&home);

    let old = parked.path.clone();
    let new = home.join(".local/state/claimdag");
    let id = seed(&old);

    let mut server = Seat::start(&home);
    server.handshake();
    let before = server.ready();
    assert_eq!(before.len(), 1, "{before:?}");
    assert_eq!(text(field(&before[0], "id")), id.to_hex());
    assert_eq!(text(field(&before[0], "summary")), SUMMARY);

    let (from, to) = move_default_graph();
    assert_eq!(from, old, "moved from {from:?}, the graph was at {old:?}");
    assert_eq!(to, new);
    assert!(
        !old.join(claimdag::SNAP_BIN).exists(),
        "move left {} in {}",
        claimdag::SNAP_BIN,
        old.display()
    );
    assert!(new.join(claimdag::SNAP_BIN).is_file());

    let after = server.ready();
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(text(field(&after[0], "id")), id.to_hex());
    assert_eq!(text(field(&after[0], "summary")), SUMMARY);

    let claimed = server.claim(&id.to_hex(), ASSIGNEE);
    assert_eq!(text(field(&claimed, "id")), id.to_hex());
    assert!(
        !old.join(claimdag::SNAP_BIN).exists(),
        "claim recreated {} in {}",
        claimdag::SNAP_BIN,
        old.display()
    );
    let graph = WorkGraph::open_dir(&new).unwrap_or_else(|err| panic!("{err}"));
    let node = graph
        .get(id)
        .unwrap_or_else(|| panic!("no {id:?} in {}", new.display()));
    assert_eq!(node.summary, SUMMARY);
    assert_eq!(node.assignee.to_hex(), ASSIGNEE);
    assert_eq!(node.status, WorkStatus::Claimed);
}
