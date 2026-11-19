//! `claimdag-mcp`: the claim graph over the Model Context Protocol.
//!
//! One graph per seat, in the directory `--dir`, `CLAIMDAG_DIR`, or the
//! runtime and state defaults name, the same as the command line and the pane.
//! Stdio, because a seat runs this beside the agent rather than as a service.

mod args;
mod prompts;
mod server;

use std::ffi::OsString;
use std::path::PathBuf;

use rmcp::{transport::stdio, ServiceExt};

const HELP: &str = "\
Model Context Protocol server over a claimdag work.bin

Usage: claimdag-mcp [OPTIONS]

Options:
      --dir <DIR>  Directory with work.bin (else CLAIMDAG_DIR, else the runtime dir, else the state dir)
  -h, --help       Print help
  -V, --version    Print version";

enum CliAction {
    Serve(Option<PathBuf>),
    Help,
    Version,
}

fn parse_args<I>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut dir: Option<PathBuf> = None;
    let mut iter = args.into_iter().skip(1);
    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "-h" | "--help" => return Ok(CliAction::Help),
            "-V" | "--version" => return Ok(CliAction::Version),
            "--dir" => {
                let value = iter.next().ok_or_else(|| {
                    "error: a value is required for '--dir <DIR>' but none was supplied".to_string()
                })?;
                dir = Some(PathBuf::from(value));
            }
            _ => {
                if let Some(rest) = arg.to_str().and_then(|s| s.strip_prefix("--dir=")) {
                    dir = Some(PathBuf::from(rest));
                } else {
                    return Err(format!("error: unexpected argument '{text}' found"));
                }
            }
        }
    }
    Ok(CliAction::Serve(dir))
}

fn closed_before_initialize(err: &impl std::fmt::Display) -> bool {
    err.to_string()
        .contains("connection closed: initialize request")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = match parse_args(std::env::args_os()) {
        Ok(CliAction::Help) => {
            println!("{HELP}");
            return Ok(());
        }
        Ok(CliAction::Version) => {
            println!("claimdag-mcp {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(CliAction::Serve(dir)) => dir,
        Err(msg) => {
            eprintln!(
                "{msg}\n\nUsage: claimdag-mcp [OPTIONS]\n\nFor more information, try '--help'."
            );
            std::process::exit(2);
        }
    };
    let running = match server::ClaimdagServer::from_dir(dir).serve(stdio()).await {
        Ok(running) => running,
        Err(err) => {
            if closed_before_initialize(&err) {
                eprintln!("claimdag-mcp: stdin closed before initialize");
                std::process::exit(1);
            }
            return Err(err.into());
        }
    };
    if let Err(err) = running.waiting().await {
        if closed_before_initialize(&err) {
            eprintln!("claimdag-mcp: stdin closed before initialize");
            std::process::exit(1);
        }
        return Err(err.into());
    }
    Ok(())
}
