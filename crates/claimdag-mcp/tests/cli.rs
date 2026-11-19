//! Command-line options for `claimdag-mcp`: `--help`, `--version`, and `--dir`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};
use serde_json::Value;

#[test]
fn help_and_version_exit_zero_without_reading_stdin() {
    for flag in ["--help", "-h"] {
        let out = Command::new(env!("CARGO_BIN_EXE_claimdag-mcp"))
            .arg(flag)
            .stdin(Stdio::null())
            .output()
            .expect("run claimdag-mcp --help");
        assert_eq!(out.status.code(), Some(0), "{flag}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("Usage: claimdag-mcp [OPTIONS]"), "{stdout}");
        assert!(stdout.contains("--dir <DIR>"), "{stdout}");
        assert!(stdout.contains("--version"), "{stdout}");
        assert!(
            out.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    for flag in ["--version", "-V"] {
        let out = Command::new(env!("CARGO_BIN_EXE_claimdag-mcp"))
            .arg(flag)
            .stdin(Stdio::null())
            .output()
            .expect("run claimdag-mcp --version");
        assert_eq!(out.status.code(), Some(0), "{flag}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            stdout.trim(),
            format!("claimdag-mcp {}", env!("CARGO_PKG_VERSION"))
        );
        assert!(
            out.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn unknown_or_incomplete_flags_exit_two() {
    for args in [&["--not-a-flag"][..], &["--dir"][..], &["unexpected"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_claimdag-mcp"))
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("run claimdag-mcp with bad args");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("Usage: claimdag-mcp [OPTIONS]"), "{stderr}");
        assert!(stderr.contains("--help"), "{stderr}");
    }
}

#[test]
fn eof_before_initialize_prints_a_clear_line() {
    let out = Command::new(env!("CARGO_BIN_EXE_claimdag-mcp"))
        .stdin(Stdio::null())
        .output()
        .expect("run claimdag-mcp with closed stdin");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "claimdag-mcp: stdin closed before initialize\n"
    );
}

#[test]
fn dir_flag_overrides_environment_and_serves_that_graph() {
    let dir_flag = tempfile::tempdir().expect("flag dir");
    let dir_env = tempfile::tempdir().expect("env dir");

    let mut graph = WorkGraph::default();
    graph
        .upsert(
            WorkId::ZERO,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: WorkId::ZERO,
                summary: "from --dir",
            },
        )
        .expect("upsert");
    graph.save_dir(dir_flag.path()).expect("save");

    for arg_style in [
        vec!["--dir".to_string(), dir_flag.path().display().to_string()],
        vec![format!("--dir={}", dir_flag.path().display())],
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_claimdag-mcp"))
            .args(&arg_style)
            .env("CLAIMDAG_DIR", dir_env.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn claimdag-mcp");
        let mut stdin = child.stdin.take().expect("stdin");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-11-25","capabilities":{{}},"clientInfo":{{"name":"test","version":"0"}}}}}}"#
        )
        .expect("write init");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
        )
        .expect("write initialized");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"claimdag_ready","arguments":{{}}}}}}"#
        )
        .expect("write ready");
        stdin.flush().expect("flush");
        drop(stdin);

        let mut lines = Vec::new();
        let mut line = String::new();
        while stdout.read_line(&mut line).expect("read stdout") > 0 {
            lines.push(line.trim_end().to_string());
            line.clear();
        }
        let status = child.wait().expect("wait");
        assert!(status.success(), "{status}");

        let ready_msg: Value = lines
            .iter()
            .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
            .find(|v| v.get("id").and_then(Value::as_i64) == Some(2))
            .expect("ready reply");
        let text = ready_msg["result"]["content"][0]["text"]
            .as_str()
            .expect("ready text");
        assert!(text.contains("from --dir"), "{text}");
    }
}
