//! claimdag command line over a directory that holds work.bin.

use std::collections::HashMap;
use std::path::PathBuf;

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkNode, WorkRole, WorkStatus};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "claimdag",
    version,
    about = "CAS claim and complete on a DAG",
    after_help = "Exit status is 0 when the verb ran, 1 when it was refused, and 2 when the arguments could not be read. A refusal names the node, who holds it and the unix second that holder was last written when those apply, and the command that resolves it."
)]
struct Cli {
    /// Directory that holds work.bin (else CLAIMDAG_DIR, else the runtime dir, else the state dir).
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
        /// Omit to keep the status an existing node already has. A new node starts at todo.
        #[arg(long)]
        status: Option<String>,
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
    /// View ready nodes that can be taken now: unblocked and unfinished.
    Ready {
        /// Filter or prioritize by role ('explore', 'architect', 'implementor', 'verifier', 'orchestrator', 'general').
        #[arg(long, default_value = "unset")]
        role: String,
        /// Requesting worker id (32 hex) for dispersed load balancing.
        #[arg(long, default_value = "00000000000000000000000000000000")]
        assignee: String,
        /// Critical depth slack for candidate dispersion.
        #[arg(long, default_value_t = 1)]
        slack: usize,
        /// Order by balanced suitability and worker dispersion rather than static depth.
        #[arg(long)]
        balanced: bool,
        /// Machine JSON (default is text lines).
        #[arg(long)]
        json: bool,
    },
    /// Atomically claim the best available ready node with load-balanced selection.
    ClaimNext {
        /// Who is taking it, 32 hex characters.
        #[arg(long)]
        assignee: String,
        /// Desired role affinity ('explore', 'architect', 'implementor', 'verifier', 'orchestrator', 'general').
        #[arg(long, default_value = "unset")]
        role: String,
        /// Allowed critical depth slack for candidate dispersion (default 1).
        #[arg(long, default_value_t = 1)]
        slack: usize,
    },
    /// Mark a node terminal (done, failed, or cancelled).
    ///
    /// Pass the --gen returned by claim to be refused if the lease was
    /// reclaimed while the work was in flight. Omitting it finishes the node
    /// whatever happened to the claim, which is the only way to speak for a
    /// node nobody holds.
    ///
    /// --actor names who is finishing it.
    ///
    /// With none, the call still finishes the node.
    ///
    /// It records a warning on stderr and in warn.log.
    ///
    /// The warning is allow, warn, and no-actor, separated by tabs.
    ///
    /// An actor who is not the holder is recorded the same way, with not-assignee.
    ///
    /// The call still finishes the node.
    ///
    /// --force finishes it for nobody.
    ///
    /// It appends a line to forced.log instead of that warning.
    Complete {
        id: String,
        #[arg(long, default_value = "done")]
        status: String,
        #[arg(long, default_value = "")]
        summary: String,
        #[arg(long)]
        actor: Option<String>,
        /// Finish it with no actor, whoever holds it. Logged to forced.log.
        #[arg(long, conflicts_with = "actor")]
        force: bool,
        #[arg(long)]
        gen: Option<u64>,
    },
    /// Say the holder is still working, moving the lease without changing hands.
    Renew {
        id: String,
        #[arg(long, default_value = "00000000000000000000000000000000")]
        actor: String,
    },
    /// Hand one claim back on purpose: ready again, assignee cleared, generation moved.
    ///
    /// The way off a claim before the work is terminal. A holder that stops
    /// without this stays busy until the lease runs out, and cannot claim
    /// anything else meanwhile.
    ///
    /// --actor names the holder.
    ///
    /// With none, the call still hands the claim back.
    ///
    /// It records a warning on stderr and in warn.log.
    ///
    /// The warning is allow, warn, and no-actor, separated by tabs.
    ///
    /// An actor who is not the holder is recorded the same way, with not-assignee.
    ///
    /// The call still hands the claim back.
    ///
    /// --force hands the claim back whoever holds it.
    ///
    /// It appends a line to forced.log instead of that warning.
    Release {
        id: String,
        #[arg(long)]
        actor: Option<String>,
        /// Hand it back with no actor, whoever holds it. Logged to forced.log.
        #[arg(long, conflicts_with = "actor")]
        force: bool,
    },
    /// Bring a finished node back to ready, generation moved, so the same work can be taken again.
    Reopen {
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
    /// Print every location the graph can be in, in the order they are tried:
    /// the directory, whether it is in use, holds work or has no graph, and
    /// the rule that names it.
    Where {
        /// Move the graph in use into the first default location, when the
        /// defaults picked a later one because only it held a graph.
        #[arg(long = "move")]
        move_graph: bool,
    },
}

/// Characters in a work id. `WorkId` is two `u64`s, printed as 16 hex
/// characters each, and `WorkId::from_hex` accepts that width only.
const ID_HEX_CHARS: usize = 16 + 16;

fn parse_id(s: &str, g: &WorkGraph) -> Result<WorkId, String> {
    if let Some(id) = WorkId::from_hex(s) {
        return Ok(id);
    }
    // Not an id the graph could hold. "unknown" is a well-formed id that
    // is absent, so a short or non-hex paste must not use that word.
    // The count is characters, the same count as the width in the message.
    // A byte length would disagree once the text is not ASCII.
    let n = s.chars().count();
    let mut msg = format!("bad id {s}: an id is {ID_HEX_CHARS} hex characters, this is {n}");
    if let Some(full) = unique_hex_prefix(g, s) {
        msg.push_str(&format!(". did you mean {full}"));
    }
    Err(msg)
}

/// The one node id that `s` prefixes, when `s` is hex and exactly one node matches.
///
/// The caller still refuses the verb. A prefix is a hint, not an id to act on.
fn unique_hex_prefix(g: &WorkGraph, s: &str) -> Option<String> {
    // Empty is a prefix of every id, and a string as long as an id that
    // failed `from_hex` is not a prefix of one. Only a shorter hex string can be.
    if s.is_empty()
        || s.chars().count() >= ID_HEX_CHARS
        || !s.chars().all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    let needle = s.to_ascii_lowercase();
    let mut found: Option<String> = None;
    for node in g.list() {
        let hex = node.id.to_hex();
        if !hex.starts_with(&needle) {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(hex);
    }
    found
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

/// Unfinished chain below `id`. A node the depth map skipped has no
/// unfinished successor, the same depth as a leaf.
fn chain_below(depth: &HashMap<WorkId, u32>, id: WorkId) -> u32 {
    depth.get(&id).copied().unwrap_or(0)
}

/// Ready rows. Each carries the unfinished chain below it, as
/// `waiting_below` in JSON and `below=N` on a text line, so the order is
/// visible. `list` keeps its own printer and does not grow this field.
fn print_ready(
    nodes: &[&WorkNode],
    depth: &HashMap<WorkId, u32>,
    json: bool,
) -> Result<(), String> {
    if json {
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
                    "waiting_below": chain_below(depth, n.id),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string(&rows).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    for n in nodes {
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
            "{}  {}  {}{}{}  gen={}  below={}{}  {}",
            n.id.to_hex(),
            n.status.as_str(),
            n.kind.as_str(),
            who,
            role,
            n.cas_gen,
            chain_below(depth, n.id),
            flag,
            n.summary
        );
    }
    Ok(())
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

/// The answer to a complete or release of a node the graph dropped after it
/// finished: there is nothing left to finish or hand back.
fn print_pruned(id: WorkId, was: WorkStatus) {
    println!(
        "{}  {}  already terminal and pruned; nothing changed",
        id.to_hex(),
        was.as_str()
    );
}

/// The file in the graph directory that keeps every forced release and
/// complete, one line each.
const FORCED_LOG: &str = "forced.log";

/// Record a `--force` call: a line on stderr, and the same line appended to
/// `forced.log` beside `work.bin`. The caller holds the directory lock.
fn log_forced(
    dir: &std::path::Path,
    verb: &str,
    id: WorkId,
    holder: WorkId,
    outcome: &str,
) -> Result<(), String> {
    use std::io::Write;
    let line = format!(
        "{}  {verb} --force  {}  holder={}  {outcome}",
        WorkGraph::now(),
        id.to_hex(),
        holder.to_hex()
    );
    eprintln!("claimdag: forced: {line}");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(FORCED_LOG))
        .map_err(|e| format!("{FORCED_LOG}: {e}"))?;
    writeln!(f, "{line}").map_err(|e| format!("{FORCED_LOG}: {e}"))?;
    f.sync_all().map_err(|e| format!("{FORCED_LOG}: {e}"))
}

/// Whether a verb only reads the graph.
///
/// A writer opening a directory with no graph in it is a cold start and has to
/// work: the first claim on a seat is what creates the snapshot. A reader has
/// nothing to create, so an absent graph is something it has to say rather
/// than something it can answer around.
fn reads_only(cmd: &Cmd) -> bool {
    matches!(cmd, Cmd::List { .. } | Cmd::Get { .. } | Cmd::Ready { .. })
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        // Refusals exit 1. Clap exits 2 itself when it cannot read the
        // arguments, which is clap 4's default for a parse error. `--help`
        // states both.
        std::process::exit(1);
    }
}

/// `where`, after the move when one was asked for and there was a graph to
/// move. It opens no graph, so it answers on a seat that has none.
fn where_is(dir: Option<PathBuf>, move_graph: bool) -> Result<(), String> {
    let located = claimdag::locate(dir.clone());
    if move_graph {
        if let Some((from, to)) = located.move_graph()? {
            println!("moved {} to {}", from.display(), to.display());
            print!("{}", claimdag::locate(dir));
            return Ok(());
        }
    }
    print!("{located}");
    Ok(())
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    if let Cmd::Where { move_graph } = cli.cmd {
        return where_is(cli.dir, move_graph);
    }
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
        WorkGraph::load_for_write(&dir).map_err(|unreadable| unreadable.to_string())?
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
            let id = parse_id(&id, &g)?;
            let n = g
                .get(id)
                .ok_or_else(|| format!("get: unknown id {}. claimdag list --all", id.to_hex()))?;
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
                Some(s) => parse_id(&s, &g)?,
                None => WorkId::ZERO,
            };
            let kind = WorkKind::parse_str(&kind).ok_or_else(|| format!("bad kind {kind}"))?;
            // An omitted --status on a node already in the graph repeats that
            // node's status, so the write changes metadata only. A new node,
            // including an id the graph does not hold yet, still starts at
            // todo. Readiness is derived from the dependencies after that.
            let status = match status {
                Some(raw) => {
                    WorkStatus::parse_str(&raw).ok_or_else(|| format!("bad status {raw}"))?
                }
                None if !wid.is_zero() => g
                    .get(wid)
                    .map(|node| node.status)
                    .unwrap_or(WorkStatus::Todo),
                None => WorkStatus::Todo,
            };
            let role = WorkRole::parse_str(&role).ok_or_else(|| format!("bad role {role}"))?;
            let parent = parse_id(&parent, &g)?;
            let actor = parse_id(&actor, &g)?;
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
            let id = parse_id(&id, &g)?;
            let assignee = parse_id(&assignee, &g)?;
            let cas = g.claim(id, assignee, expected)?;
            g.save_dir(&dir)?;
            println!("gen={cas}");
        }
        Cmd::Ready {
            role,
            assignee,
            slack,
            balanced,
            json,
        } => {
            let role = WorkRole::parse_str(&role).ok_or_else(|| format!("bad role {role}"))?;
            let assignee = parse_id(&assignee, &g)?;
            let nodes = if balanced || role != WorkRole::Unset || !assignee.is_zero() {
                g.ready_view_balanced(role, assignee, slack)
            } else {
                g.ready_view()
            };
            // One walk for every row. The view sorts by its own pass over
            // the same depth; this one is what the seat reads.
            let depth = g.critical_depth();
            print_ready(&nodes, &depth, json)?;
        }
        Cmd::ClaimNext {
            assignee,
            role,
            slack,
        } => {
            let assignee = parse_id(&assignee, &g)?;
            let role = WorkRole::parse_str(&role).ok_or_else(|| format!("bad role {role}"))?;
            let (id, cas) = g.claim_next(assignee, role, slack)?;
            g.save_dir(&dir)?;
            println!("{}  gen={cas}", id.to_hex());
        }
        Cmd::Complete {
            id,
            status,
            summary,
            actor,
            force,
            gen,
        } => {
            let status =
                WorkStatus::parse_str(&status).ok_or_else(|| format!("bad status {status}"))?;
            let id = parse_id(&id, &g)?;
            if force {
                let holder = g.complete_forced(id, status, &summary, gen)?;
                g.save_dir(&dir)?;
                log_forced(&dir, "complete", id, holder, status.as_str())?;
            } else {
                let actor = match actor {
                    Some(actor) => parse_id(&actor, &g)?,
                    None => WorkId::ZERO,
                };
                g.complete(id, status, &summary, actor, gen)?;
                g.save_dir(&dir)?;
            }
            match g.pruned(id) {
                Some(was) => print_pruned(id, was),
                None => println!("{}  {}", id.to_hex(), status.as_str()),
            }
        }
        Cmd::Renew { id, actor } => {
            let id = parse_id(&id, &g)?;
            let actor = parse_id(&actor, &g)?;
            let cas = g.renew(id, actor)?;
            g.save_dir(&dir)?;
            println!("gen={cas}");
        }
        Cmd::Release { id, actor, force } => {
            let id = parse_id(&id, &g)?;
            let cas = if force {
                let (cas, holder) = g.release_forced(id)?;
                g.save_dir(&dir)?;
                log_forced(&dir, "release", id, holder, &format!("gen={cas}"))?;
                cas
            } else {
                let actor = match actor {
                    Some(actor) => parse_id(&actor, &g)?,
                    None => WorkId::ZERO,
                };
                let cas = g.release(id, actor)?;
                g.save_dir(&dir)?;
                cas
            };
            match g.pruned(id) {
                Some(was) => print_pruned(id, was),
                None => println!("gen={cas}"),
            }
        }
        Cmd::Reopen { id, actor } => {
            let id = parse_id(&id, &g)?;
            let actor = parse_id(&actor, &g)?;
            let cas = g.reopen(id, actor)?;
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
            let parent = parse_id(&parent, &g)?;
            let child = parse_id(&child, &g)?;
            let actor = parse_id(&actor, &g)?;
            g.link_dep(parent, child, actor)?;
            g.save_dir(&dir)?;
            println!("{}  {}", parent.to_hex(), child.to_hex());
        }
        Cmd::Unlink {
            parent,
            child,
            actor,
        } => {
            let parent = parse_id(&parent, &g)?;
            let child = parse_id(&child, &g)?;
            let actor = parse_id(&actor, &g)?;
            g.unlink_dep(parent, child, actor)?;
            g.save_dir(&dir)?;
            println!("{}  {}", parent.to_hex(), child.to_hex());
        }
        Cmd::Archive { id, actor } => {
            let id = parse_id(&id, &g)?;
            let actor = parse_id(&actor, &g)?;
            g.archive(id, actor)?;
            g.save_dir(&dir)?;
            println!("{}  archived", id.to_hex());
        }
        Cmd::Unarchive { id, actor } => {
            let id = parse_id(&id, &g)?;
            let actor = parse_id(&actor, &g)?;
            g.unarchive(id, actor)?;
            g.save_dir(&dir)?;
            println!("{}  live", id.to_hex());
        }
        Cmd::Where { .. } => unreachable!("where is answered before the graph is opened"),
    }
    for line in g.warnings() {
        claimdag::emit_warn(&dir, line);
    }
    Ok(())
}
