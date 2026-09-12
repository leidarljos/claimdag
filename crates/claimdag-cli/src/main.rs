//! claimdag command line over a directory that holds work.bin.

use std::path::PathBuf;

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkNode, WorkRole, WorkStatus};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "claimdag", version, about = "CAS claim and complete on a DAG")]
struct Cli {
    /// Directory that holds work.bin (else CLAIMDAG_DIR, else the runtime dir).
    #[arg(long)]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List nodes, newest first. Default is live work only.
    List {
        /// Include non-archived terminal nodes.
        #[arg(long)]
        terminal: bool,
        /// Include archived nodes.
        #[arg(long)]
        archived: bool,
        /// Every node, including archived terminals.
        #[arg(long)]
        all: bool,
        /// Machine JSON (TUI and hosts). Default is text lines.
        #[arg(long)]
        json: bool,
    },
    /// Print one node by 32-hex id.
    Get { id: String },
    /// Create or update a node. Omit --id to mint.
    Upsert {
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value = "task")]
        kind: String,
        #[arg(long, default_value = "todo")]
        status: String,
        #[arg(long, default_value = "unset")]
        role: String,
        /// Parent work id (32 hex). Zero means unset.
        #[arg(long, default_value = "00000000000000000000000000000000")]
        parent: String,
        #[arg(long, default_value = "")]
        summary: String,
        #[arg(long, default_value = "00000000000000000000000000000000")]
        actor: String,
    },
    /// Compare-and-swap claim. Omit --gen to ignore generation.
    Claim {
        id: String,
        #[arg(long)]
        assignee: String,
        #[arg(long)]
        gen: Option<u64>,
    },
    /// Mark a node terminal (done, failed, or cancelled).
    ///
    /// Pass the --gen returned by claim to be refused if the lease was
    /// reclaimed while the work was in flight. Omitting it finishes the node
    /// whatever happened to the claim, which is the only way to speak for a
    /// node nobody holds.
    Complete {
        id: String,
        #[arg(long, default_value = "done")]
        status: String,
        #[arg(long, default_value = "")]
        summary: String,
        #[arg(long, default_value = "00000000000000000000000000000000")]
        actor: String,
        #[arg(long)]
        gen: Option<u64>,
    },
    /// Say the holder is still working, moving the lease without changing hands.
    Renew {
        id: String,
        #[arg(long, default_value = "00000000000000000000000000000000")]
        actor: String,
    },
    /// Hand back every claim quiet for longer than the lease, in seconds.
    ///
    /// A claim with no expiry is a claim a crashed worker keeps, and the
    /// holder's identity stays busy with it. This is what returns both.
    Reclaim {
        #[arg(long, default_value_t = claimdag::DEFAULT_LEASE_SECS)]
        lease: u64,
    },
    /// Add a boolean hard dependency (parent before child).
    Link {
        parent: String,
        child: String,
        #[arg(long, default_value = "00000000000000000000000000000000")]
        actor: String,
    },
    /// Drop a boolean hard dependency. Missing edge is ok.
    Unlink {
        parent: String,
        child: String,
        #[arg(long, default_value = "00000000000000000000000000000000")]
        actor: String,
    },
    /// Soft-hide a terminal node. Not a delete.
    Archive {
        id: String,
        #[arg(long, default_value = "00000000000000000000000000000000")]
        actor: String,
    },
    /// Clear the archive flag. Status stays terminal.
    Unarchive {
        id: String,
        #[arg(long, default_value = "00000000000000000000000000000000")]
        actor: String,
    },
}

fn parse_id(s: &str) -> Result<WorkId, String> {
    WorkId::from_hex(s).ok_or_else(|| format!("bad id {s}"))
}

fn print_list_json(nodes: &[&WorkNode]) -> Result<(), String> {
    let rows: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            serde_json::json!({
                "id": n.id.to_hex(),
                "kind": n.kind.as_str(),
                "status": n.status.as_str(),
                "role": n.role.as_str(),
                "assignee": n.assignee.to_hex(),
                "parent": n.parent.to_hex(),
                "deps": n.deps.iter().map(|d| d.to_hex()).collect::<Vec<_>>(),
                "cas_gen": n.cas_gen,
                "created_unix": n.created_unix,
                "updated_unix": n.updated_unix,
                "finished_unix": n.finished_unix,
                "summary": n.summary,
                "archived": n.archived,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string(&rows).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn print_list_line(n: &WorkNode) {
    let flag = if n.archived { "  archived" } else { "" };
    let who = if n.assignee.is_zero() {
        String::new()
    } else {
        format!("  {}", &n.assignee.to_hex()[..8])
    };
    let role = if n.role.as_str() == "unset" {
        String::new()
    } else {
        format!("  {}", n.role.as_str())
    };
    println!(
        "{}  {}  {}{}{}  gen={}{}  {}",
        n.id.to_hex(),
        n.status.as_str(),
        n.kind.as_str(),
        who,
        role,
        n.cas_gen,
        flag,
        n.summary
    );
}

fn print_get(n: &WorkNode) {
    let deps = if n.deps.is_empty() {
        "-".to_string()
    } else {
        n.deps
            .iter()
            .map(|d| d.to_hex())
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!(
        "{}  {}  {}  {}  gen={}  assignee={}  parent={}  {}",
        n.id.to_hex(),
        n.status.as_str(),
        n.kind.as_str(),
        n.role.as_str(),
        n.cas_gen,
        n.assignee.to_hex(),
        n.parent.to_hex(),
        n.summary
    );
    println!("deps  {deps}");
}

/// Whether a verb only reads the graph.
///
/// A writer opening a directory with no graph in it is a cold start and has to
/// work: the first claim on a seat is what creates the snapshot. A reader has
/// nothing to create, so an absent graph is something it has to say rather
/// than something it can answer around.
fn reads_only(cmd: &Cmd) -> bool {
    matches!(cmd, Cmd::List { .. } | Cmd::Get { .. })
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let dir = claimdag::resolve_dir(cli.dir.clone());
    // A verb that only reads says when there is no graph to read. An empty
    // list and a seat pointed at nothing look the same to a caller, and they
    // mean opposite things: one is an answer, the other is a wrong question.
    // A writer holds the directory from load to save, so two processes
    // cannot both read the same snapshot and each write back without the
    // other's change.
    let _lock = if reads_only(&cli.cmd) {
        None
    } else {
        Some(claimdag::lock_dir(&dir)?)
    };
    let mut g = if reads_only(&cli.cmd) {
        WorkGraph::open_dir(&dir).map_err(|absent| absent.to_string())?
    } else {
        WorkGraph::load_dir(&dir)
    };
    match cli.cmd {
        Cmd::List {
            terminal,
            archived,
            all,
            json,
        } => {
            let (show_terminal, show_archived) = if all {
                (true, true)
            } else {
                (terminal, archived)
            };
            let nodes = g.list_view(show_terminal, show_archived);
            if json {
                print_list_json(&nodes)?;
            } else {
                for n in nodes {
                    print_list_line(n);
                }
            }
        }
        Cmd::Get { id } => {
            let id = parse_id(&id)?;
            let n = g.get(id).ok_or("not found")?;
            print_get(n);
        }
        Cmd::Upsert {
            id,
            kind,
            status,
            role,
            parent,
            summary,
            actor,
        } => {
            let wid = match id {
                Some(s) => parse_id(&s)?,
                None => WorkId::ZERO,
            };
            let kind = WorkKind::parse_str(&kind).ok_or_else(|| format!("bad kind {kind}"))?;
            let status =
                WorkStatus::parse_str(&status).ok_or_else(|| format!("bad status {status}"))?;
            let role = WorkRole::parse_str(&role).ok_or_else(|| format!("bad role {role}"))?;
            let parent = parse_id(&parent)?;
            let actor = parse_id(&actor)?;
            let out = g.upsert(
                wid,
                WorkFields {
                    kind,
                    status,
                    role,
                    parent,
                    actor,
                    summary: &summary,
                },
            )?;
            g.save_dir(&dir)?;
            println!("{}", out.to_hex());
        }
        Cmd::Claim { id, assignee, gen } => {
            let expected = match gen {
                None | Some(0) => None,
                Some(g) => Some(g),
            };
            let cas = g.claim(parse_id(&id)?, parse_id(&assignee)?, expected)?;
            g.save_dir(&dir)?;
            println!("gen={cas}");
        }
        Cmd::Complete {
            id,
            status,
            summary,
            actor,
            gen,
        } => {
            let status =
                WorkStatus::parse_str(&status).ok_or_else(|| format!("bad status {status}"))?;
            let id = parse_id(&id)?;
            g.complete(id, status, &summary, parse_id(&actor)?, gen)?;
            g.save_dir(&dir)?;
            println!("{}  {}", id.to_hex(), status.as_str());
        }
        Cmd::Renew { id, actor } => {
            let id = parse_id(&id)?;
            let cas = g.renew(id, parse_id(&actor)?)?;
            g.save_dir(&dir)?;
            println!("gen={cas}");
        }
        Cmd::Reclaim { lease } => {
            let handed = g.reclaim(lease);
            if !handed.is_empty() {
                g.save_dir(&dir)?;
            }
            for id in &handed {
                println!("{}  reclaimed", id.to_hex());
            }
            if handed.is_empty() {
                println!("none");
            }
        }
        Cmd::Link {
            parent,
            child,
            actor,
        } => {
            let parent = parse_id(&parent)?;
            let child = parse_id(&child)?;
            g.link_dep(parent, child, parse_id(&actor)?)?;
            g.save_dir(&dir)?;
            println!("{}  {}", parent.to_hex(), child.to_hex());
        }
        Cmd::Unlink {
            parent,
            child,
            actor,
        } => {
            let parent = parse_id(&parent)?;
            let child = parse_id(&child)?;
            g.unlink_dep(parent, child, parse_id(&actor)?)?;
            g.save_dir(&dir)?;
            println!("{}  {}", parent.to_hex(), child.to_hex());
        }
        Cmd::Archive { id, actor } => {
            let id = parse_id(&id)?;
            g.archive(id, parse_id(&actor)?)?;
            g.save_dir(&dir)?;
            println!("{}  archived", id.to_hex());
        }
        Cmd::Unarchive { id, actor } => {
            let id = parse_id(&id)?;
            g.unarchive(id, parse_id(&actor)?)?;
            g.save_dir(&dir)?;
            println!("{}  live", id.to_hex());
        }
    }
    Ok(())
}
