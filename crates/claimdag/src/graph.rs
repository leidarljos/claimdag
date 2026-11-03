//! Claim/complete DAG. Host process is the sole mutator.
//!
//! Identity is [`WorkId`] (xxh3-128). Closed enums for kind/status/role.
//! One open text field: summary. Snapshot is unpacked Cap'n `work.bin`
//! (mmap).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use xxhash_rust::xxh3::xxh3_64_with_seed;

use crate::id::{mint_work_id, WorkId};

/// How long a claim stands before anybody may take it back, in seconds. One
/// value for every seat, or one takes back what another still holds.
///
/// A sitting does not renew while its holder still runs, so this is how long
/// a gone holder may stay quiet before the next sitting hands the claim back.
/// On an Intel Xeon (family 6, model 207, 8 cores), at e0f79d7, five runs of
/// `ljos sitting ISSUE --verbose` (ljos 0.37.0, vissue 0.23.1, deedar 0.6.1,
/// this claimdag, `PACKSET_URL=off`, `LJOS_TRACKER_GIT=off`) put the longest
/// section at `== doctor`, 242.9 ms median (242.2 to 243.1), and the whole
/// sitting at 541 ms. One sitting on a graph of 4096 nodes took 162 ms to
/// list that graph and reclaim, still under the doctor. Fifteen minutes is many
/// of those sittings, so one that is still printing is not quiet, and a
/// crashed holder does not keep the node, or the one claim its assignee is
/// allowed, for the rest of the day. The measured step is under this, so the
/// value stays 900.
pub const DEFAULT_LEASE_SECS: u64 = 900;

/// Snapshot format written by this crate.
pub const FORMAT_V1: &str = "claimdag/v1";

/// Snapshot filename under the host state dir (unpacked Cap'n, mmap).
pub const SNAP_FILE: &str = crate::snap::SNAP_BIN;

/// How many nodes the graph holds before prune drops finished work. Every
/// call loads the whole snapshot, and a writing call saves it whole, so the
/// cost grows with the node count (measurements/node-cap.org, 8-core Xeon).
/// The budget is 10 ms for a writing call on a full graph, a tenth of the
/// 100 ms within which an answer still reads as immediate (Miller 1968,
/// Nielsen 1993), so a seat can chain ten calls in that time. 4096 is the
/// largest measured size inside it (7.1 ms; 8192 took 11.4 ms), and what
/// is left covers summaries up to 256 bytes (9.1 ms).
const MAX_WORK_NODES: usize = 4096;

/// Nothing outside this crate reads the ledger, and the command line, the
/// MCP server and the TUI each load a fresh graph for every call, so the
/// cap only bounds a program that keeps one graph for many calls. The
/// budget is the graph itself: room for two ops, a creation and a finish,
/// for every node a full graph holds is 512 KiB of 64-byte entries, under
/// the 768 KiB snapshot of that graph (measurements/node-cap.org).
const MAX_LEDGER: usize = 2 * MAX_WORK_NODES;

/// How many pruned nodes the graph answers for at least: one full graph's
/// worth, so a finished node is remembered until as many nodes again have
/// been dropped after it. measurements/node-cap.org has the cost.
const MAX_PRUNED: usize = MAX_WORK_NODES;

/// The file beside `work.bin` that names each pruned node and the status
/// it finished with, one `ID STATUS` line each, oldest first. It is not in
/// `work.bin`, so the snapshot keeps the bytes earlier releases wrote.
const PRUNED_FILE: &str = "pruned.log";

/// An id that is not in the snapshot.
///
/// `list --all` is the inventory of ids. The default list hides terminal and
/// archived nodes, so it cannot answer which ids are there.
fn unknown_id(verb: &str, id: WorkId) -> String {
    format!("{verb}: unknown id {}. claimdag list --all", id.to_hex())
}

/// The actor is not the holder.
///
/// `updated_unix` is the lease clock: claim, renew and running all write it,
/// so it is the second this holder was last recorded. `set_running` has no
/// command-line verb, so that refusal says who may call it.
fn not_assignee(verb: &str, node: &WorkNode) -> String {
    let id = node.id.to_hex();
    let holder = node.assignee.to_hex();
    let command = if verb == "running" {
        format!("pass actor {holder} to set_running")
    } else {
        format!("claimdag {verb} {id} --actor {holder}")
    };
    format!(
        "{verb}: not assignee. node {id} is held by {holder} since unix {}. {command}",
        node.updated_unix
    )
}

/// A claim of a node that is already claimed or running.
///
/// `updated_unix` is the lease clock, the second this holder was last
/// recorded. Release hands the claim back. With no holder, `--force` is the
/// logged way, as it is on release itself.
fn claim_held(node: &WorkNode) -> String {
    let id = node.id.to_hex();
    let status = node.status.as_str();
    if node.assignee.is_zero() {
        return format!(
            "claim: status {status}. node {id} has no holder. claimdag release {id} --force"
        );
    }
    let holder = node.assignee.to_hex();
    format!(
        "claim: status {status}. node {id} is held by {holder} since unix {}. claimdag release {id} --actor {holder}",
        node.updated_unix
    )
}

/// The generation the caller sent is not the one stored.
///
/// That stored generation is the fencing token a later claim has to repeat.
fn claim_gen_mismatch(node: &WorkNode) -> String {
    format!(
        "claim: gen mismatch. node {} is generation {}",
        node.id.to_hex(),
        node.cas_gen
    )
}

/// This assignee already holds a live claim, and one is the limit.
///
/// Each held id has to be released or completed before another claim. The
/// ids are sorted so the line does not depend on map order.
fn assignee_busy(assignee: WorkId, mut held: Vec<WorkId>) -> String {
    held.sort();
    let who = assignee.to_hex();
    let listed = held
        .iter()
        .map(|id| id.to_hex())
        .collect::<Vec<_>>()
        .join(" ");
    let commands = held
        .iter()
        .map(|id| {
            let hex = id.to_hex();
            format!("claimdag release {hex} --actor {who} or claimdag complete {hex} --actor {who}")
        })
        .collect::<Vec<_>>()
        .join(". ");
    format!("claim: assignee busy {listed}. {commands}")
}

/// Upsert does not reopen a finished node. `reopen` does.
fn upsert_terminal(node: &WorkNode) -> String {
    let id = node.id.to_hex();
    format!(
        "upsert: node is terminal. node {id} is {}. claimdag reopen {id}",
        node.status.as_str()
    )
}

/// The command that ends a live claim: complete it, or hand it back.
///
/// `completeWork` is not a command. With no holder, `--force` is the logged
/// way to act for nobody.
fn release_or_complete(node: &WorkNode) -> String {
    let id = node.id.to_hex();
    if node.assignee.is_zero() {
        return format!("claimdag complete {id} --force or claimdag release {id} --force");
    }
    let holder = node.assignee.to_hex();
    format!("claimdag complete {id} --actor {holder} or claimdag release {id} --actor {holder}")
}

/// Upsert does not end a live claim. Complete and release do.
fn upsert_claimed(node: &WorkNode) -> String {
    let id = node.id.to_hex();
    let next = release_or_complete(node);
    if node.assignee.is_zero() {
        return format!(
            "upsert: node is claimed. node {id} is {}. {next}",
            node.status.as_str()
        );
    }
    let holder = node.assignee.to_hex();
    format!(
        "upsert: node is claimed. node {id} is held by {holder} since unix {}. {next}",
        node.updated_unix
    )
}

/// Claim, complete and release own claimed, running and terminal.
///
/// Upsert does not, and there is no `claimWork` or `completeWork` command.
/// A live node is already held, so the way off it is complete or release.
fn upsert_lifecycle(node: &WorkNode, asked: WorkStatus) -> String {
    let id = node.id.to_hex();
    let command = if matches!(node.status, WorkStatus::Claimed | WorkStatus::Running) {
        release_or_complete(node)
    } else {
        match asked {
            WorkStatus::Claimed | WorkStatus::Running => {
                format!("claimdag claim {id} --assignee")
            }
            WorkStatus::Done | WorkStatus::Failed | WorkStatus::Cancelled => {
                format!("claimdag complete {id} --status {}", asked.as_str())
            }
            WorkStatus::Todo | WorkStatus::Ready | WorkStatus::Blocked => {
                format!("claimdag claim {id} --assignee or claimdag complete {id}")
            }
        }
    };
    format!("upsert: status claim/run/terminal requires {command}")
}

/// An endpoint of a dependency edge is not in the graph.
///
/// `list --all` is the inventory of ids. The default list hides terminal
/// and archived nodes, so it cannot answer which id is missing.
fn missing_endpoint(verb: &str, role: &str, id: WorkId) -> String {
    format!(
        "{verb}: {role} missing. unknown id {}. claimdag list --all",
        id.to_hex()
    )
}

/// Archive applies to a finished node.
///
/// The id and its status are what `get` would have said. `list --all` is
/// the inventory when this was the wrong id. Complete is what makes the
/// node archivable.
fn archive_not_terminal(node: &WorkNode) -> String {
    let id = node.id.to_hex();
    let status = node.status.as_str();
    if node.assignee.is_zero() {
        return format!(
            "archive: not terminal. node {id} is {status}. claimdag complete {id} --force. claimdag list --all"
        );
    }
    let holder = node.assignee.to_hex();
    format!(
        "archive: not terminal. node {id} is {status} and held by {holder} since unix {}. claimdag complete {id} --actor {holder}. claimdag list --all",
        node.updated_unix
    )
}

/// Work node status (CAS claim/complete).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkStatus {
    Todo,
    Ready,
    Claimed,
    Running,
    Blocked,
    Done,
    Failed,
    Cancelled,
}

impl WorkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Ready => "ready",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(Self::Todo),
            "ready" => Some(Self::Ready),
            "claimed" => Some(Self::Claimed),
            "running" => Some(Self::Running),
            "blocked" => Some(Self::Blocked),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkRole {
    Unset,
    Explore,
    Architect,
    Implementor,
    Verifier,
    Orchestrator,
    General,
}

impl WorkRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Explore => "explore",
            Self::Architect => "architect",
            Self::Implementor => "implementor",
            Self::Verifier => "verifier",
            Self::Orchestrator => "orchestrator",
            Self::General => "general",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "unset" => Some(Self::Unset),
            "explore" => Some(Self::Explore),
            "architect" => Some(Self::Architect),
            "implementor" => Some(Self::Implementor),
            "verifier" => Some(Self::Verifier),
            "orchestrator" => Some(Self::Orchestrator),
            "general" => Some(Self::General),
            _ => None,
        }
    }
}

/// Affinity tier of a work node to a worker's role:
/// - 0: Exact match (node.role == worker_role), or both are Unset/General
/// - 1: Flexible match (node.role is Unset/General, or worker_role is Unset/General)
/// - 2: Mismatched specialization (e.g. Explorer assigned to Verifier when alternatives exist)
#[must_use]
pub fn role_affinity_tier(node_role: WorkRole, worker_role: WorkRole) -> u8 {
    if worker_role == WorkRole::Unset || worker_role == WorkRole::General {
        if node_role == WorkRole::Unset || node_role == WorkRole::General {
            0
        } else {
            1
        }
    } else if node_role == worker_role {
        0
    } else if node_role == WorkRole::Unset || node_role == WorkRole::General {
        1
    } else {
        2
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkKind {
    Unset,
    Goal,
    Step,
    Task,
    Molecule,
}

impl WorkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Goal => "goal",
            Self::Step => "step",
            Self::Task => "task",
            Self::Molecule => "molecule",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "unset" => Some(Self::Unset),
            "goal" => Some(Self::Goal),
            "step" => Some(Self::Step),
            "task" => Some(Self::Task),
            "molecule" => Some(Self::Molecule),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkNode {
    pub id: WorkId,
    pub kind: WorkKind,
    pub status: WorkStatus,
    pub role: WorkRole,
    pub assignee: WorkId,
    pub parent: WorkId,
    pub deps: Vec<WorkId>,
    pub cas_gen: u64,
    pub created_unix: u64,
    pub updated_unix: u64,
    pub finished_unix: u64,
    /// Sole open-content field (writer-owned length).
    pub summary: String,
    /// Soft hide. Only legal on a terminal node. Not a status.
    pub archived: bool,
}

/// One operation, as the graph saw it in this process.
///
/// The ledger lives in memory only. `work.bin` does not store it, so a
/// command-line call starts with an empty ledger and its entries are gone
/// when it exits. It is not a history of the graph.
#[derive(Debug, Clone)]
pub struct WorkLedgerEntry {
    pub seq: u64,
    pub ts_unix: u64,
    pub work_id: WorkId,
    pub actor: WorkId,
    /// Closed op tag (static str interned as &'static str in practice).
    pub op: &'static str,
}

/// The fields an upsert sets on a node.
#[derive(Debug, Clone, Copy)]
pub struct WorkFields<'a> {
    /// What sort of work it is.
    pub kind: WorkKind,
    /// The status asked for. Readiness is still derived from the dependencies.
    pub status: WorkStatus,
    /// Who the work is for.
    pub role: WorkRole,
    /// The node this one hangs under, or zero.
    pub parent: WorkId,
    /// Who is asking, for the ledger.
    pub actor: WorkId,
    /// The one open text field.
    pub summary: &'a str,
}

#[derive(Debug, Default)]
pub struct WorkGraph {
    nodes: HashMap<WorkId, WorkNode>,
    ledger: VecDeque<WorkLedgerEntry>,
    next_seq: u64,
    mint_seq: u64,
    /// `pruned.log` in the directory the graph was loaded from.
    pruned_path: Option<PathBuf>,
    /// What that file holds, read on the first question about an id the
    /// graph does not hold, so a call that asks none never reads it.
    pruned_log: std::sync::OnceLock<String>,
    /// The finished nodes this graph dropped itself, oldest first.
    dropped: Vec<(WorkId, WorkStatus)>,
}

/// Why a seat's work graph could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Absent {
    /// No directory at all: this seat has never had a graph here.
    NoDirectory(PathBuf),
    /// A directory with no snapshot in it.
    NoSnapshot(PathBuf),
    /// A snapshot that could not be parsed, and the reason given.
    Unreadable(PathBuf, String),
}

impl std::fmt::Display for Absent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDirectory(dir) => write!(
                f,
                "no work graph at {}: the directory does not exist, so nothing has been claimed on this seat. \
                 Set CLAIMDAG_DIR, pass --dir, or claim something to create it",
                dir.display()
            ),
            Self::NoSnapshot(dir) => write!(
                f,
                "no work graph at {}: the directory holds no {}. \
                 Claim something to create it",
                dir.display(),
                crate::snap::SNAP_BIN
            ),
            Self::Unreadable(dir, why) => write!(
                f,
                "the work graph at {} could not be read: {why}. No write saves over it: \
                 restore {} from a copy, or move it aside to start an empty graph",
                dir.display(),
                crate::snap::SNAP_BIN
            ),
        }
    }
}

impl std::error::Error for Absent {}

impl WorkGraph {
    pub fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Mint a new work id (xxh3 family). Deterministic from seq + salt.
    pub fn mint_id(&mut self, kind: WorkKind, parent: WorkId, summary: &str) -> WorkId {
        self.mint_seq = self.mint_seq.saturating_add(1);
        let salt = Self::now().to_le_bytes();
        let role = match kind {
            WorkKind::Goal => "work-goal",
            WorkKind::Step => "work-step",
            WorkKind::Task => "work-task",
            WorkKind::Molecule => "work-molecule",
            WorkKind::Unset => "work",
        };
        mint_work_id(parent, WorkId::ZERO, role, summary, self.mint_seq, &salt)
    }

    pub fn get(&self, id: WorkId) -> Option<&WorkNode> {
        self.nodes.get(&id)
    }

    /// The status a node finished with before prune dropped it. `None` for a
    /// node the graph holds, never had, or pruned too long ago to remember.
    #[must_use]
    pub fn pruned(&self, id: WorkId) -> Option<WorkStatus> {
        if self.nodes.contains_key(&id) {
            return None;
        }
        if let Some((_, status)) = self.dropped.iter().rev().find(|(gone, _)| *gone == id) {
            return Some(*status);
        }
        let hex = id.to_hex();
        let log = self.pruned_log.get_or_init(|| {
            self.pruned_path
                .as_deref()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .unwrap_or_default()
        });
        log.lines().rev().find_map(|line| {
            let (gone, status) = line.split_once(' ')?;
            (gone == hex)
                .then(|| WorkStatus::parse_str(status))?
                .filter(|s| s.is_terminal())
        })
    }

    pub fn list(&self) -> Vec<&WorkNode> {
        self.list_view(true, true)
    }

    /// Default live view hides terminal and archived nodes.
    pub fn list_view(&self, show_terminal: bool, show_archived: bool) -> Vec<&WorkNode> {
        let mut v: Vec<_> = self
            .nodes
            .values()
            .filter(|n| {
                if n.archived {
                    show_archived
                } else if n.status.is_terminal() {
                    show_terminal
                } else {
                    true
                }
            })
            .collect();
        v.sort_by(|a, b| b.updated_unix.cmp(&a.updated_unix).then(a.id.cmp(&b.id)));
        v
    }

    fn push_ledger(&mut self, work_id: WorkId, actor: WorkId, op: &'static str) {
        self.next_seq = self.next_seq.saturating_add(1);
        self.ledger.push_back(WorkLedgerEntry {
            seq: self.next_seq,
            ts_unix: Self::now(),
            work_id,
            actor,
            op,
        });
        while self.ledger.len() > MAX_LEDGER {
            self.ledger.pop_front();
        }
    }

    fn prune(&mut self) {
        if self.nodes.len() <= MAX_WORK_NODES {
            return;
        }
        let overflow = self.nodes.len() - MAX_WORK_NODES;
        let mut terminal: Vec<(WorkId, u8, u64)> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.status.is_terminal())
            .map(|(k, n)| {
                let archived_first = if n.archived { 0 } else { 1 };
                (*k, archived_first, n.finished_unix.max(n.updated_unix))
            })
            .collect();
        terminal.sort_by_key(|(_, arch, t)| (*arch, *t));
        for (id, _, _) in terminal.into_iter().take(overflow) {
            if let Some(gone) = self.nodes.remove(&id) {
                self.dropped.push((id, gone.status));
            }
        }
    }

    /// A parent edge that would close a cycle, and the command that does not.
    ///
    /// A zero parent does not clear a stored one, so the follow-up omits
    /// `--parent`: that is the write that leaves the chain as it is. Upsert
    /// also refuses a live claim and a terminal node, because the command
    /// line sends a status on every upsert, so those name release or reopen.
    fn parent_cycle(
        &self,
        id: WorkId,
        parent: WorkId,
        kind: WorkKind,
        status: WorkStatus,
    ) -> String {
        let id_hex = id.to_hex();
        let parent_hex = parent.to_hex();
        match self.nodes.get(&id) {
            Some(node)
                if matches!(node.status, WorkStatus::Claimed | WorkStatus::Running)
                    && !node.assignee.is_zero() =>
            {
                format!(
                    "upsert: parent cycle. node {id_hex} is held by {} since unix {} and would have parent {parent_hex}. claimdag release {id_hex} --actor {}",
                    node.assignee.to_hex(),
                    node.updated_unix,
                    node.assignee.to_hex()
                )
            }
            Some(node) if matches!(node.status, WorkStatus::Claimed | WorkStatus::Running) => {
                format!(
                    "upsert: parent cycle. node {id_hex} would have parent {parent_hex}. claimdag release {id_hex} --force"
                )
            }
            Some(node) if node.status.is_terminal() => format!(
                "upsert: parent cycle. node {id_hex} would have parent {parent_hex}. claimdag reopen {id_hex}"
            ),
            Some(node) => format!(
                "upsert: parent cycle. node {id_hex} would have parent {parent_hex}. claimdag upsert --id {id_hex} --kind {} --status {}",
                node.kind.as_str(),
                node.status.as_str()
            ),
            None => format!(
                "upsert: parent cycle. node {id_hex} would have parent {parent_hex}. claimdag upsert --id {id_hex} --kind {} --status {}",
                kind.as_str(),
                status.as_str()
            ),
        }
    }

    /// A release or complete that names nobody.
    ///
    /// The holder, when there is one, is who `--actor` has to name.
    /// `updated_unix` is the lease clock, so it is the second that holder was
    /// last recorded. `--force` is the logged way to act for nobody; the
    /// command line appends that call to `forced.log`.
    fn no_actor(&self, verb: &str, id: WorkId) -> String {
        let Some(node) = self.nodes.get(&id) else {
            return format!(
                "{verb}: no actor. unknown id {}. claimdag list --all",
                id.to_hex()
            );
        };
        let id_hex = node.id.to_hex();
        if !node.assignee.is_zero() {
            let holder = node.assignee.to_hex();
            return format!(
                "{verb}: no actor. node {id_hex} is held by {holder} since unix {}. claimdag {verb} {id_hex} --actor {holder}. claimdag {verb} {id_hex} --force acts for nobody and is logged to forced.log",
                node.updated_unix
            );
        }
        if verb == "complete" {
            // Nobody holds it, and it can still be finished. `--force` is the
            // command that does that without inventing an actor.
            return format!(
                "{verb}: no actor. node {id_hex} has no holder. claimdag {verb} {id_hex} --force acts for nobody and is logged to forced.log"
            );
        }
        // Release applies to a live claim. With no holder the status check
        // refuses the call either way, so the next step is to read the node.
        format!("{verb}: no actor. node {id_hex} has no holder. claimdag get {id_hex}")
    }

    /// What an upsert is asserting about a node.
    pub fn upsert(&mut self, mut id: WorkId, fields: WorkFields<'_>) -> Result<WorkId, String> {
        let WorkFields {
            kind,
            status,
            role,
            parent,
            actor,
            summary,
        } = fields;
        if id.is_zero() {
            id = self.mint_id(kind, parent, summary);
        }
        // The parent chain is a forest: a parent edge is loop-checked as a
        // dependency edge is.
        if !parent.is_zero() && (parent == id || self.parent_reaches(parent, id)) {
            return Err(self.parent_cycle(id, parent, kind, status));
        }
        let now = Self::now();
        let entry = self.nodes.entry(id).or_insert_with(|| WorkNode {
            id,
            kind,
            status: WorkStatus::Todo,
            role,
            assignee: WorkId::ZERO,
            parent,
            deps: Vec::new(),
            // Start at 1 so wire expectedGen=0 means "ignore", not "assert gen 0".
            cas_gen: 1,
            created_unix: now,
            updated_unix: now,
            finished_unix: 0,
            summary: String::new(),
            archived: false,
        });
        if entry.status.is_terminal() {
            // Sticky terminal: reject reopen/status forge (client sees error).
            return Err(upsert_terminal(entry));
        }
        // A live claim is claim's to end, not upsert's. Demoting a Claimed node
        // to Todo leaves the assignee in place and lets a second agent claim
        // work the first is holding, with neither claim ever failing.
        let live = matches!(entry.status, WorkStatus::Claimed | WorkStatus::Running);
        if live && status != entry.status {
            return Err(upsert_claimed(entry));
        }
        // Lifecycle authority: claim/complete own moves into Claimed, Running
        // or a terminal status. Upsert writes metadata, and Todo, Ready or
        // Blocked when the node is not held. Repeating the status a held
        // node already has is not a new status: a summary or role edit has
        // to land while the holder still has the node.
        match status {
            WorkStatus::Todo | WorkStatus::Ready | WorkStatus::Blocked => {
                if !live {
                    entry.status = status;
                }
            }
            WorkStatus::Claimed | WorkStatus::Running if status == entry.status => {}
            WorkStatus::Claimed
            | WorkStatus::Running
            | WorkStatus::Done
            | WorkStatus::Failed
            | WorkStatus::Cancelled => {
                return Err(upsert_lifecycle(entry, status));
            }
        }
        if kind != WorkKind::Unset {
            entry.kind = kind;
        }
        if role != WorkRole::Unset {
            entry.role = role;
        }
        if !parent.is_zero() {
            entry.parent = parent;
        }
        if !summary.is_empty() {
            entry.summary = summary.to_string();
        }
        entry.updated_unix = now;
        let wid = entry.id;
        // Readiness is derived, never asserted: the dependencies decide.
        self.recompute_ready(wid);
        self.push_ledger(wid, actor, "upsert");
        self.prune();
        Ok(wid)
    }

    pub fn link_dep(&mut self, parent: WorkId, child: WorkId, actor: WorkId) -> Result<(), String> {
        if parent.is_zero() || child.is_zero() {
            return Err("link: zero id".into());
        }
        if parent == child {
            return Err("link: self-dep".into());
        }
        if !self.nodes.contains_key(&parent) {
            return Err(missing_endpoint("link", "parent", parent));
        }
        if !self.nodes.contains_key(&child) {
            return Err(missing_endpoint("link", "child", child));
        }
        if self.would_cycle(parent, child) {
            return Err("link: would create cycle".into());
        }
        let node = self.nodes.get_mut(&child).expect("child present");
        if !node.deps.contains(&parent) {
            node.deps.push(parent);
            node.updated_unix = Self::now();
        }
        self.recompute_ready(child);
        self.push_ledger(child, actor, "link");
        Ok(())
    }

    /// Drop a boolean hard dep. Missing edge is Ok.
    pub fn unlink_dep(
        &mut self,
        parent: WorkId,
        child: WorkId,
        actor: WorkId,
    ) -> Result<(), String> {
        if parent.is_zero() || child.is_zero() {
            return Err("unlink: zero id".into());
        }
        let node = self
            .nodes
            .get_mut(&child)
            .ok_or_else(|| missing_endpoint("unlink", "child", child))?;
        let before = node.deps.len();
        node.deps.retain(|d| *d != parent);
        if node.deps.len() != before {
            node.updated_unix = Self::now();
        }
        self.recompute_ready(child);
        self.push_ledger(child, actor, "unlink");
        Ok(())
    }

    fn would_cycle(&self, parent: WorkId, child: WorkId) -> bool {
        let mut stack = vec![parent];
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            if id == child {
                return true;
            }
            if !seen.insert(id) {
                continue;
            }
            if let Some(n) = self.nodes.get(&id) {
                stack.extend(n.deps.iter().copied());
            }
        }
        false
    }

    /// Whether walking up from `from` reaches `target`; bounded by a visited set.
    fn parent_reaches(&self, from: WorkId, target: WorkId) -> bool {
        let mut at = from;
        let mut seen = HashSet::new();
        while !at.is_zero() {
            if at == target {
                return true;
            }
            if !seen.insert(at) {
                return false;
            }
            let Some(node) = self.nodes.get(&at) else {
                return false;
            };
            at = node.parent;
        }
        false
    }

    /// The longest chain of unfinished work below each node, unit cost: the
    /// list-scheduling priority (Hu, doi:10.1287/opre.9.6.841; Graham,
    /// doi:10.1137/0117039). Finished and archived nodes count for nothing.
    #[must_use]
    pub fn critical_depth(&self) -> HashMap<WorkId, u32> {
        // Successors, built once; the nodes carry their parents.
        let mut below: HashMap<WorkId, Vec<WorkId>> = HashMap::new();
        for node in self.nodes.values() {
            if node.archived || node.status.is_terminal() {
                continue;
            }
            for dep in &node.deps {
                below.entry(*dep).or_default().push(node.id);
            }
        }

        let mut depth: HashMap<WorkId, u32> = HashMap::new();
        for id in self.nodes.keys() {
            Self::depth_below(*id, &below, &mut depth, &mut Vec::new());
        }
        depth
    }

    /// One node's depth, memoised; a cycle in a corrupt file terminates at
    /// zero rather than hanging.
    fn depth_below(
        id: WorkId,
        below: &HashMap<WorkId, Vec<WorkId>>,
        depth: &mut HashMap<WorkId, u32>,
        walking: &mut Vec<WorkId>,
    ) -> u32 {
        if let Some(held) = depth.get(&id) {
            return *held;
        }
        if walking.contains(&id) {
            return 0;
        }
        walking.push(id);
        let deepest = below
            .get(&id)
            .map(|kids| {
                kids.iter()
                    .map(|kid| 1 + Self::depth_below(*kid, below, depth, walking))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        walking.pop();
        depth.insert(id, deepest);
        deepest
    }

    /// The nodes a seat may take right now: unblocked and unfinished, the
    /// predicate `claim` enforces. Deepest chain first, then most recently
    /// touched, then id.
    #[must_use]
    pub fn ready_view(&self) -> Vec<&WorkNode> {
        let depth = self.critical_depth();
        let mut open: Vec<&WorkNode> = self
            .nodes
            .values()
            .filter(|n| !n.archived)
            .filter(|n| matches!(n.status, WorkStatus::Ready | WorkStatus::Todo))
            .filter(|n| self.deps_satisfied(n))
            .collect();
        open.sort_by(|a, b| {
            let (here, there) = (
                depth.get(&a.id).copied().unwrap_or(0),
                depth.get(&b.id).copied().unwrap_or(0),
            );
            there
                .cmp(&here)
                .then_with(|| b.updated_unix.cmp(&a.updated_unix))
                .then_with(|| a.id.cmp(&b.id))
        });
        open
    }

    /// Ready nodes ordered by balanced suitability for `(role, assignee)`:
    /// 1. Critical depth slack bands: candidates within `slack` of the maximum
    ///    critical depth form Band 0, preserving critical-path progress.
    /// 2. Role affinity: exact match (tier 0) before general/unset (tier 1)
    ///    before mismatched specialization (tier 2).
    /// 3. Worker dispersion / Power-of-Two choices: pseudo-random hash ranking
    ///    seeded by worker identity so concurrent workers naturally inspect
    ///    distinct candidates without central lock contention.
    /// 4. Starvation prevention: older ready nodes (updated_unix) are prioritized.
    /// 5. Deterministic tie-break by node id.
    #[must_use]
    pub fn ready_view_balanced(
        &self,
        role: WorkRole,
        assignee: WorkId,
        slack: usize,
    ) -> Vec<&WorkNode> {
        let depth = self.critical_depth();
        let mut open: Vec<&WorkNode> = self
            .nodes
            .values()
            .filter(|n| !n.archived)
            .filter(|n| matches!(n.status, WorkStatus::Ready | WorkStatus::Todo))
            .filter(|n| self.deps_satisfied(n))
            .collect();

        if open.is_empty() {
            return open;
        }

        let max_depth = open
            .iter()
            .map(|n| depth.get(&n.id).copied().unwrap_or(0))
            .max()
            .unwrap_or(0);

        let slack_u32 = slack as u32;
        let seed = if assignee.is_zero() {
            0x9e3779b97f4a7c15
        } else {
            assignee.hi ^ assignee.lo.rotate_left(17)
        };

        open.sort_by(|a, b| {
            let da = depth.get(&a.id).copied().unwrap_or(0);
            let db = depth.get(&b.id).copied().unwrap_or(0);

            let band_a = if da + slack_u32 >= max_depth {
                0
            } else {
                max_depth.saturating_sub(da + slack_u32)
            };
            let band_b = if db + slack_u32 >= max_depth {
                0
            } else {
                max_depth.saturating_sub(db + slack_u32)
            };

            let tier_a = role_affinity_tier(a.role, role);
            let tier_b = role_affinity_tier(b.role, role);

            let disp_a = xxh3_64_with_seed(&a.id.to_be_bytes(), seed);
            let disp_b = xxh3_64_with_seed(&b.id.to_be_bytes(), seed);

            band_a
                .cmp(&band_b)
                .then_with(|| tier_a.cmp(&tier_b))
                .then_with(|| db.cmp(&da))
                .then_with(|| disp_a.cmp(&disp_b))
                .then_with(|| a.updated_unix.cmp(&b.updated_unix))
                .then_with(|| a.id.cmp(&b.id))
        });

        open
    }

    /// Pick the single best candidate ready node for `(role, assignee)` using
    /// Mitzenmacher's Power of Two Random Choices over the top critical-slack
    /// candidate pool.
    #[must_use]
    pub fn select_balanced_ready(
        &self,
        role: WorkRole,
        assignee: WorkId,
        slack: usize,
    ) -> Option<&WorkNode> {
        let depth = self.critical_depth();
        let open: Vec<&WorkNode> = self
            .nodes
            .values()
            .filter(|n| !n.archived)
            .filter(|n| matches!(n.status, WorkStatus::Ready | WorkStatus::Todo))
            .filter(|n| self.deps_satisfied(n))
            .collect();

        if open.is_empty() {
            return None;
        }

        let max_depth = open
            .iter()
            .map(|n| depth.get(&n.id).copied().unwrap_or(0))
            .max()
            .unwrap_or(0);

        let slack_u32 = slack as u32;

        let band_0: Vec<&WorkNode> = open
            .into_iter()
            .filter(|n| {
                let d = depth.get(&n.id).copied().unwrap_or(0);
                d + slack_u32 >= max_depth
            })
            .collect();

        if band_0.is_empty() {
            return None;
        }

        let min_tier = band_0
            .iter()
            .map(|n| role_affinity_tier(n.role, role))
            .min()
            .unwrap_or(2);

        // Sorted because `nodes` is a HashMap, whose order differs in every
        // process: the probes below index into this list, and the same
        // worker on the same graph has to land on the same two nodes.
        let mut candidates: Vec<&WorkNode> = band_0
            .into_iter()
            .filter(|n| role_affinity_tier(n.role, role) == min_tier)
            .collect();
        candidates.sort_by_key(|n| n.id);

        if candidates.len() == 1 {
            return Some(candidates[0]);
        }

        let n = candidates.len();
        let (c1, c2) = if assignee.is_zero() {
            (candidates[0], candidates[1])
        } else {
            let seed1 = assignee.hi ^ 0x9e3779b97f4a7c15;
            let seed2 = assignee.lo ^ 0x517cc1b727220a95;
            let h1 = xxh3_64_with_seed(&assignee.to_be_bytes(), seed1) as usize;
            let h2 = xxh3_64_with_seed(&assignee.to_be_bytes(), seed2) as usize;
            let idx1 = h1 % n;
            let mut idx2 = h2 % n;
            if idx2 == idx1 {
                idx2 = (idx1 + 1) % n;
            }
            (candidates[idx1], candidates[idx2])
        };

        let d1 = depth.get(&c1.id).copied().unwrap_or(0);
        let d2 = depth.get(&c2.id).copied().unwrap_or(0);

        let c1_is_better = d1
            .cmp(&d2)
            .then_with(|| c2.updated_unix.cmp(&c1.updated_unix))
            .then_with(|| c1.id.cmp(&c2.id))
            .is_gt();

        if c1_is_better {
            Some(c1)
        } else {
            Some(c2)
        }
    }

    fn deps_satisfied(&self, node: &WorkNode) -> bool {
        node.deps.iter().all(|d| {
            self.nodes
                .get(d)
                .map(|p| p.status == WorkStatus::Done)
                .unwrap_or(false)
        })
    }

    /// The dependencies in `deps` that are not done, as `HEX (status)`, in
    /// the order given. A dependency missing from the graph reads `missing`.
    fn unfinished_deps(&self, deps: &[WorkId]) -> Vec<String> {
        deps.iter()
            .filter_map(|d| match self.nodes.get(d) {
                Some(p) if p.status == WorkStatus::Done => None,
                Some(p) => Some(format!("{} ({})", d.to_hex(), p.status.as_str())),
                None => Some(format!("{} (missing)", d.to_hex())),
            })
            .collect()
    }

    fn recompute_ready(&mut self, id: WorkId) {
        let ready = {
            let Some(n) = self.nodes.get(&id) else {
                return;
            };
            matches!(
                n.status,
                WorkStatus::Todo | WorkStatus::Blocked | WorkStatus::Ready
            ) && self.deps_satisfied(n)
        };
        if let Some(n) = self.nodes.get_mut(&id) {
            if ready && matches!(n.status, WorkStatus::Todo | WorkStatus::Blocked) {
                n.status = WorkStatus::Ready;
                n.updated_unix = Self::now();
            } else if !ready && n.status == WorkStatus::Ready {
                n.status = WorkStatus::Todo;
                n.updated_unix = Self::now();
            }
        }
    }

    fn promote_dependents(&mut self, done_id: WorkId) {
        let children: Vec<WorkId> = self
            .nodes
            .values()
            .filter(|n| n.deps.contains(&done_id))
            .map(|n| n.id)
            .collect();
        for c in children {
            self.recompute_ready(c);
        }
    }

    pub fn claim(
        &mut self,
        id: WorkId,
        assignee: WorkId,
        expected_gen: Option<u64>,
    ) -> Result<u64, String> {
        if id.is_zero() {
            return Err("claim: zero id".into());
        }
        if assignee.is_zero() {
            return Err("claim: zero assignee".into());
        }
        if let Some(was) = self.pruned(id) {
            return Err(format!("claim: status {}, pruned", was.as_str()));
        }
        let deps: Vec<WorkId> = {
            let node = self.nodes.get(&id).ok_or_else(|| unknown_id("claim", id))?;
            if let Some(g) = expected_gen {
                if node.cas_gen != g {
                    return Err(claim_gen_mismatch(node));
                }
            }
            if matches!(node.status, WorkStatus::Claimed | WorkStatus::Running) {
                return Err(claim_held(node));
            }
            if !matches!(node.status, WorkStatus::Ready | WorkStatus::Todo) {
                return Err(format!("claim: status {}", node.status.as_str()));
            }
            node.deps.clone()
        };
        // The same predicate as `deps_satisfied`: done counts, anything else,
        // including a dependency no longer in the graph, does not.
        let open = self.unfinished_deps(&deps);
        if !open.is_empty() {
            return Err(format!("claim: deps unsatisfied {}", open.join(" ")));
        }
        // Occupancy is a graph predicate: one Claimed|Running node per assignee.
        // Same mutex as the gen bump (caller holds work.lock()). Skip `id`.
        let busy: Vec<WorkId> = self
            .nodes
            .values()
            .filter(|n| {
                n.id != id
                    && n.assignee == assignee
                    && matches!(n.status, WorkStatus::Claimed | WorkStatus::Running)
            })
            .map(|n| n.id)
            .collect();
        if !busy.is_empty() {
            return Err(assignee_busy(assignee, busy));
        }
        let now = Self::now();
        let node = self.nodes.get_mut(&id).unwrap();
        node.status = WorkStatus::Claimed;
        node.assignee = assignee;
        node.cas_gen = node.cas_gen.saturating_add(1);
        node.updated_unix = now;
        let cas_gen = node.cas_gen;
        self.push_ledger(id, assignee, "claim");
        Ok(cas_gen)
    }

    /// Atomically find and claim the best ready work node for `assignee` matching
    /// `role` within critical path `slack`.
    ///
    /// Avoids CAS stampedes and two-phase check-then-act race conditions by
    /// combining candidate selection and state transition into one atomic step
    /// under the graph lock.
    pub fn claim_next(
        &mut self,
        assignee: WorkId,
        role: WorkRole,
        slack: usize,
    ) -> Result<(WorkId, u64), String> {
        if assignee.is_zero() {
            return Err("claim: zero assignee".into());
        }

        // Occupancy guard: one live claim per assignee
        let busy: Vec<WorkId> = self
            .nodes
            .values()
            .filter(|n| {
                n.assignee == assignee
                    && matches!(n.status, WorkStatus::Claimed | WorkStatus::Running)
            })
            .map(|n| n.id)
            .collect();
        if !busy.is_empty() {
            return Err(assignee_busy(assignee, busy));
        }

        let winner_id = {
            let candidate = self
                .select_balanced_ready(role, assignee, slack)
                .ok_or_else(|| "claim-next: no ready work".to_string())?;
            candidate.id
        };

        let now = Self::now();
        let node = self.nodes.get_mut(&winner_id).unwrap();
        node.status = WorkStatus::Claimed;
        node.assignee = assignee;
        node.cas_gen = node.cas_gen.saturating_add(1);
        node.updated_unix = now;
        let cas_gen = node.cas_gen;
        self.push_ledger(winner_id, assignee, "claim");
        Ok((winner_id, cas_gen))
    }

    /// Return every claim quiet for longer than `lease_secs` to `Ready`,
    /// bumping the generation so a holder that wakes later is fenced.
    /// Returns the nodes handed back, oldest first.
    pub fn reclaim(&mut self, lease_secs: u64) -> Vec<WorkId> {
        let now = Self::now();
        let mut stale: Vec<(u64, WorkId)> = self
            .nodes
            .values()
            .filter(|n| matches!(n.status, WorkStatus::Claimed | WorkStatus::Running))
            .filter(|n| now.saturating_sub(n.updated_unix) >= lease_secs)
            .map(|n| (n.updated_unix, n.id))
            .collect();
        stale.sort_unstable();
        let mut handed = Vec::with_capacity(stale.len());
        for (_, id) in stale {
            let Some(node) = self.nodes.get_mut(&id) else {
                continue;
            };
            let holder = node.assignee;
            node.status = WorkStatus::Ready;
            node.assignee = WorkId::ZERO;
            node.cas_gen = node.cas_gen.saturating_add(1);
            node.updated_unix = now;
            // The ledger records the holder that lost it rather than a
            // reclaiming actor, because there is no actor: the lease ran out.
            self.push_ledger(id, holder, "reclaim");
            handed.push(id);
        }
        handed
    }

    /// Hand one claim back on purpose. The node returns to `ready`, the
    /// assignee clears, and the generation moves, exactly as a reclaim does:
    /// the holder chose to stop before the lease ran out. Only the holder may
    /// do it. A zero actor is refused; [`WorkGraph::release_forced`] is the
    /// explicit way to hand back a claim for nobody.
    ///
    /// A node prune dropped after it finished is held by nobody already, so
    /// releasing it succeeds, changes nothing, and answers generation 0;
    /// [`WorkGraph::pruned`] says which status it finished with.
    pub fn release(&mut self, id: WorkId, actor: WorkId) -> Result<u64, String> {
        if actor.is_zero() {
            return Err(self.no_actor("release", id));
        }
        self.release_as(id, actor, false)
            .map(|(generation, _)| generation)
    }

    /// Hand a claim back whoever holds it. The ledger records the op as
    /// `release-forced`. Returns the new generation and the holder it was
    /// taken from, so the caller can log both.
    pub fn release_forced(&mut self, id: WorkId) -> Result<(u64, WorkId), String> {
        self.release_as(id, WorkId::ZERO, true)
    }

    fn release_as(
        &mut self,
        id: WorkId,
        actor: WorkId,
        forced: bool,
    ) -> Result<(u64, WorkId), String> {
        if self.pruned(id).is_some() {
            return Ok((0, WorkId::ZERO));
        }
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| unknown_id("release", id))?;
        if !matches!(node.status, WorkStatus::Claimed | WorkStatus::Running) {
            return Err(format!("release: status {}", node.status.as_str()));
        }
        if !forced && !node.assignee.is_zero() && node.assignee != actor {
            return Err(not_assignee("release", node));
        }
        let holder = node.assignee;
        node.status = WorkStatus::Ready;
        node.assignee = WorkId::ZERO;
        node.cas_gen = node.cas_gen.saturating_add(1);
        node.updated_unix = Self::now();
        let generation = node.cas_gen;
        let op = if forced { "release-forced" } else { "release" };
        self.push_ledger(id, holder, op);
        Ok((generation, holder))
    }

    /// Bring a terminal node back to `ready`, generation moved, so work on
    /// the same tracker id can be taken again: a new sitting on old work.
    /// The snapshot keeps no history of operations, so nothing on disk
    /// records the earlier completion; see [`WorkLedgerEntry`].
    pub fn reopen(&mut self, id: WorkId, actor: WorkId) -> Result<u64, String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| unknown_id("reopen", id))?;
        if !node.status.is_terminal() {
            return Err(format!(
                "reopen: status {} is not terminal",
                node.status.as_str()
            ));
        }
        node.status = WorkStatus::Ready;
        node.assignee = WorkId::ZERO;
        node.archived = false;
        node.cas_gen = node.cas_gen.saturating_add(1);
        node.updated_unix = Self::now();
        let generation = node.cas_gen;
        self.push_ledger(id, actor, "reopen");
        Ok(generation)
    }

    /// Move the lease forward. The generation stays: a renewal is not a change
    /// of ownership.
    pub fn renew(&mut self, id: WorkId, actor: WorkId) -> Result<u64, String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| unknown_id("renew", id))?;
        if !matches!(node.status, WorkStatus::Claimed | WorkStatus::Running) {
            return Err(format!("renew: status {}", node.status.as_str()));
        }
        if !node.assignee.is_zero() && !actor.is_zero() && node.assignee != actor {
            return Err(not_assignee("renew", node));
        }
        node.updated_unix = Self::now();
        Ok(node.cas_gen)
    }

    pub fn set_running(&mut self, id: WorkId, actor: WorkId) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| unknown_id("running", id))?;
        if !matches!(node.status, WorkStatus::Claimed | WorkStatus::Running) {
            return Err(format!("running: status {}", node.status.as_str()));
        }
        // The same question complete asks, and it has to answer the same way,
        // or the ledger records a stranger as having started somebody's work.
        // A zero actor is the command line's escape, as it is there.
        if !node.assignee.is_zero() && !actor.is_zero() && node.assignee != actor {
            return Err(not_assignee("running", node));
        }
        node.status = WorkStatus::Running;
        node.updated_unix = Self::now();
        self.push_ledger(id, actor, "running");
        Ok(())
    }

    /// Finish a node. `expected_gen` is the fencing token from the claim: the
    /// generation moves on every claim and reclaim, so a stale holder is
    /// refused. `None` skips the check. A zero actor is refused;
    /// [`WorkGraph::complete_forced`] is the explicit way to finish a node
    /// for nobody.
    ///
    /// A node prune dropped after it finished is as finished as it gets, so
    /// completing it again succeeds and changes nothing, as it does for a
    /// terminal node still in the graph; [`WorkGraph::pruned`] says which
    /// status it finished with.
    pub fn complete(
        &mut self,
        id: WorkId,
        status: WorkStatus,
        summary: &str,
        actor: WorkId,
        expected_gen: Option<u64>,
    ) -> Result<(), String> {
        if actor.is_zero() {
            return Err(self.no_actor("complete", id));
        }
        self.complete_as(id, status, summary, actor, expected_gen, false)
            .map(|_| ())
    }

    /// Finish a node whoever holds it. Dependencies and `expected_gen` are
    /// still checked. The ledger records the op as `complete-forced`.
    /// Returns the holder the node had, so the caller can log it.
    pub fn complete_forced(
        &mut self,
        id: WorkId,
        status: WorkStatus,
        summary: &str,
        expected_gen: Option<u64>,
    ) -> Result<WorkId, String> {
        self.complete_as(id, status, summary, WorkId::ZERO, expected_gen, true)
    }

    fn complete_as(
        &mut self,
        id: WorkId,
        status: WorkStatus,
        summary: &str,
        actor: WorkId,
        expected_gen: Option<u64>,
        forced: bool,
    ) -> Result<WorkId, String> {
        if !status.is_terminal() {
            return Err("complete: status not terminal".into());
        }
        if id.is_zero() {
            return Err("complete: zero id".into());
        }
        if self.pruned(id).is_some() {
            return Ok(WorkId::ZERO);
        }
        let now = Self::now();
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| unknown_id("complete", id))?;
        if let Some(g) = expected_gen {
            if node.cas_gen != g {
                return Err("complete: gen mismatch".into());
            }
        }
        // Whoever held it may still speak for it; checked before the terminal
        // case so a stranger cannot rewrite a finished node.
        if !forced && !node.assignee.is_zero() && node.assignee != actor {
            return Err(not_assignee("complete", node));
        }
        let holder = node.assignee;
        if node.status.is_terminal() {
            if !summary.is_empty() {
                node.summary = summary.to_string();
            }
            return Ok(holder);
        }
        // Done says the work is finished, and work that waits on unfinished
        // work is not. Failed and cancelled stay open: giving up on a node
        // does not need what it waits on.
        if status == WorkStatus::Done {
            let deps = node.deps.clone();
            let open = self.unfinished_deps(&deps);
            if !open.is_empty() {
                return Err(format!("complete: deps unfinished {}", open.join(" ")));
            }
        }
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| unknown_id("complete", id))?;
        node.status = status;
        if !summary.is_empty() {
            node.summary = summary.to_string();
        }
        node.finished_unix = now;
        node.updated_unix = now;
        let op = if forced {
            "complete-forced"
        } else {
            "complete"
        };
        self.push_ledger(id, actor, op);
        self.promote_dependents(id);
        Ok(holder)
    }

    pub fn archive(&mut self, id: WorkId, actor: WorkId) -> Result<(), String> {
        if id.is_zero() {
            return Err("archive: zero id".into());
        }
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| unknown_id("archive", id))?;
        if !node.status.is_terminal() {
            return Err(archive_not_terminal(node));
        }
        if node.archived {
            return Ok(());
        }
        node.archived = true;
        node.updated_unix = Self::now();
        node.cas_gen = node.cas_gen.saturating_add(1);
        self.push_ledger(id, actor, "archive");
        Ok(())
    }

    pub fn unarchive(&mut self, id: WorkId, actor: WorkId) -> Result<(), String> {
        if id.is_zero() {
            return Err("unarchive: zero id".into());
        }
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| unknown_id("unarchive", id))?;
        if !node.archived {
            return Ok(());
        }
        node.archived = false;
        node.updated_unix = Self::now();
        node.cas_gen = node.cas_gen.saturating_add(1);
        self.push_ledger(id, actor, "unarchive");
        Ok(())
    }

    pub fn verify(&self) -> Result<(), String> {
        for (id, n) in &self.nodes {
            for d in &n.deps {
                if !self.nodes.contains_key(d) {
                    return Err(format!(
                        "verify: {} deps missing {}",
                        id.to_hex(),
                        d.to_hex()
                    ));
                }
            }
            let mut stack = n.deps.clone();
            let mut seen = HashSet::new();
            while let Some(cur) = stack.pop() {
                if cur == *id {
                    return Err(format!("verify: cycle involving {}", id.to_hex()));
                }
                if !seen.insert(cur) {
                    continue;
                }
                if let Some(nn) = self.nodes.get(&cur) {
                    stack.extend(nn.deps.iter().copied());
                }
            }
            // A parent that is not in the graph is not an error: prune drops
            // finished work and leaves its children as roots. A parent chain
            // that comes back to where it started is.
            if !n.parent.is_zero() && self.parent_reaches(n.parent, *id) {
                return Err(format!("verify: parent cycle involving {}", id.to_hex()));
            }
        }
        Ok(())
    }

    /// The graph in a directory, or why there is none: no directory, an
    /// unreadable snapshot, or an empty graph. Readers need the difference.
    pub fn open_dir(dir: &Path) -> Result<Self, Absent> {
        if !dir.is_dir() {
            return Err(Absent::NoDirectory(dir.to_path_buf()));
        }
        if !dir.join(crate::snap::SNAP_BIN).is_file() {
            return Err(Absent::NoSnapshot(dir.to_path_buf()));
        }
        match crate::snap::read_bin(dir) {
            Ok(_) => Ok(Self::load_dir(dir)),
            Err(why) => Err(Absent::Unreadable(dir.to_path_buf(), why)),
        }
    }

    /// The graph a writer starts from: empty when the seat has none yet, and
    /// refused when a snapshot is there but cannot be read.
    ///
    /// [`WorkGraph::load_dir`] answers an unreadable snapshot with an empty
    /// graph, so the next save replaces every node with the one being
    /// written. A torn or truncated `work.bin` has to stop the writer instead.
    ///
    /// # Errors
    ///
    /// [`Absent::Unreadable`] when `work.bin` exists and does not parse.
    pub fn load_for_write(dir: &Path) -> Result<Self, Absent> {
        match Self::open_dir(dir) {
            Ok(graph) => Ok(graph),
            Err(Absent::NoDirectory(_) | Absent::NoSnapshot(_)) => Ok(Self::default()),
            Err(unreadable) => Err(unreadable),
        }
    }

    /// The graph in a directory, empty when there is none or it cannot be
    /// read. A writer wants [`WorkGraph::load_for_write`], a reader
    /// [`WorkGraph::open_dir`].
    pub fn load_dir(dir: &Path) -> Self {
        match crate::snap::read_bin(dir) {
            Ok((next_seq, mint_seq, nodes)) => {
                let mut g = Self {
                    nodes: HashMap::new(),
                    ledger: VecDeque::new(),
                    next_seq,
                    mint_seq,
                    pruned_path: Some(dir.join(PRUNED_FILE)),
                    pruned_log: std::sync::OnceLock::new(),
                    dropped: Vec::new(),
                };
                for node in nodes {
                    g.nodes.insert(node.id, node);
                }
                g
            }
            Err(_) => Self::default(),
        }
    }

    /// Atomic replace of `$dir/work.bin` (unpacked Cap'n), then the nodes
    /// prune dropped onto `pruned.log`.
    pub fn save_dir(&self, dir: &Path) -> Result<(), String> {
        crate::snap::write_bin(dir, self.next_seq, self.mint_seq, &self.list())?;
        if !self.dropped.is_empty() {
            // Best effort: a list that is lost or behind only makes a dropped
            // node answer as one never seen, which is no reason to fail a
            // save that has already happened.
            self.write_pruned(dir).ok();
        }
        Ok(())
    }

    /// Append what this graph dropped, since a rewrite adds about 0.3 ms to
    /// a writing call and an append nothing measurable
    /// (measurements/node-cap.org). Past twice [`MAX_PRUNED`] lines the file
    /// is rewritten to the newest [`MAX_PRUNED`], so a rewrite comes once in
    /// that many drops and the file never holds more than twice the list.
    fn write_pruned(&self, dir: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let fresh: Vec<String> = self
            .dropped
            .iter()
            .map(|(id, status)| format!("{} {}\n", id.to_hex(), status.as_str()))
            .collect();
        let path = dir.join(PRUNED_FILE);
        let named = std::fs::read_to_string(&path).unwrap_or_default();
        let every: Vec<&str> = named
            .lines()
            .chain(fresh.iter().map(|line| line.trim_end()))
            .collect();
        if every.len() <= 2 * MAX_PRUNED {
            return std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?
                .write_all(fresh.concat().as_bytes());
        }
        let kept = every[every.len() - MAX_PRUNED..].join("\n") + "\n";
        let tmp = dir.join("pruned.log.tmp");
        std::fs::write(&tmp, kept)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> WorkId {
        WorkId {
            hi: n,
            lo: n.wrapping_mul(3),
        }
    }

    #[test]
    fn claim_cas_and_deps() {
        let mut g = WorkGraph::default();
        let a = id(1);
        let b = id(2);
        g.upsert(
            a,
            WorkFields {
                kind: WorkKind::Step,
                status: WorkStatus::Ready,
                role: WorkRole::Explore,
                parent: WorkId::ZERO,
                actor: id(9),
                summary: "A",
            },
        )
        .unwrap();
        g.upsert(
            b,
            WorkFields {
                kind: WorkKind::Step,
                status: WorkStatus::Todo,
                role: WorkRole::Implementor,
                parent: WorkId::ZERO,
                actor: id(9),
                summary: "B",
            },
        )
        .unwrap();
        g.link_dep(a, b, id(9)).unwrap();
        assert!(g.claim(b, id(10), None).is_err());
        g.complete(a, WorkStatus::Done, "ok", id(10), None).unwrap();
        assert_eq!(g.get(b).unwrap().status, WorkStatus::Ready);
        assert_eq!(g.get(b).unwrap().cas_gen, 1);
        assert_eq!(
            g.claim(b, id(10), Some(99)).unwrap_err(),
            format!("claim: gen mismatch. node {} is generation 1", b.to_hex())
        );
        let cas_gen = g.claim(b, id(10), None).unwrap();
        assert_eq!(
            g.claim(b, id(11), Some(1)).unwrap_err(),
            format!("claim: gen mismatch. node {} is generation 2", b.to_hex())
        );
        assert!(g.claim(b, id(11), Some(cas_gen)).is_err());
        g.set_running(b, id(10)).unwrap();
        let since = g.get(b).unwrap().updated_unix;
        assert_eq!(
            g.claim(b, id(11), None).unwrap_err(),
            format!(
                "claim: status running. node {} is held by {} since unix {since}. claimdag release {} --actor {}",
                b.to_hex(),
                id(10).to_hex(),
                b.to_hex(),
                id(10).to_hex()
            )
        );
        g.complete(b, WorkStatus::Done, "done", id(10), None)
            .unwrap();
        assert!(g.verify().is_ok());
    }

    #[test]
    fn complete_preserves_summary_cite_when_empty() {
        let mut g = WorkGraph::default();
        let a = id(1);
        upsert_ready(&mut g, a, "cite:TICKET-compwrite do the work");
        g.complete(a, WorkStatus::Done, "", id(9), None).unwrap();
        let n = g.get(a).unwrap();
        assert_eq!(n.status, WorkStatus::Done);
        assert_eq!(n.summary, "cite:TICKET-compwrite do the work");
    }

    fn upsert_ready(g: &mut WorkGraph, node: WorkId, summary: &str) {
        g.upsert(
            node,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: id(9),
                summary,
            },
        )
        .unwrap();
    }

    #[test]
    fn claim_rejects_assignee_already_holding_live_node() {
        let mut g = WorkGraph::default();
        let a = id(1);
        let b = id(2);
        let x = id(20);
        let y = id(21);
        upsert_ready(&mut g, a, "A");
        upsert_ready(&mut g, b, "B");
        assert_eq!(g.get(a).unwrap().cas_gen, 1);
        assert_eq!(g.get(b).unwrap().cas_gen, 1);
        g.claim(a, x, None).unwrap();
        assert_eq!(g.get(a).unwrap().status, WorkStatus::Claimed);
        let err = g.claim(b, x, None).unwrap_err();
        assert!(
            err.starts_with("claim: assignee busy"),
            "expected busy, got {err}"
        );
        assert!(
            err.contains(&a.to_hex()),
            "busy error must list held id {}; got {err}",
            a.to_hex()
        );
        let b_after = g.get(b).unwrap();
        assert_eq!(b_after.status, WorkStatus::Ready);
        assert_eq!(b_after.cas_gen, 1);
        assert!(b_after.assignee.is_zero());
        g.claim(b, y, None).unwrap();
        assert_eq!(g.get(b).unwrap().status, WorkStatus::Claimed);
        assert_eq!(g.get(b).unwrap().assignee, y);
        g.set_running(a, x).unwrap();
        let c = id(3);
        upsert_ready(&mut g, c, "C");
        let running_err = g.claim(c, x, None).unwrap_err();
        assert!(
            running_err.starts_with("claim: assignee busy"),
            "Running still occupies; got {running_err}"
        );
        assert!(running_err.contains(&a.to_hex()), "{running_err}");
        g.complete(a, WorkStatus::Done, "ok", x, None).unwrap();
        g.claim(c, x, None).unwrap();
        assert_eq!(g.get(c).unwrap().status, WorkStatus::Claimed);
        assert_eq!(g.get(c).unwrap().assignee, x);
    }

    /// A holder that stops on purpose hands the node back: ready, no
    /// assignee, generation moved, and the same holder is free to claim again.
    #[test]
    fn release_hands_a_claim_back_and_frees_the_holder() {
        let mut g = WorkGraph::default();
        let a = id(1);
        let b = id(2);
        let x = id(20);
        let y = id(21);
        upsert_ready(&mut g, a, "A");
        upsert_ready(&mut g, b, "B");
        g.claim(a, x, None).unwrap();
        assert!(g
            .claim(b, x, None)
            .unwrap_err()
            .starts_with("claim: assignee busy"));
        // A stranger cannot hand back somebody else's work.
        let refused = g.release(a, y).unwrap_err();
        assert!(
            refused.starts_with("release: not assignee")
                && refused.contains(&a.to_hex())
                && refused.contains(&x.to_hex()),
            "{refused}"
        );
        let generation = g.release(a, x).unwrap();
        let after = g.get(a).unwrap();
        assert_eq!(after.status, WorkStatus::Ready);
        assert!(after.assignee.is_zero());
        assert_eq!(after.cas_gen, generation);
        assert_eq!(generation, 3, "claim moved it to 2, release to 3");
        // The holder is free, and the released node is claimable by anyone.
        g.claim(b, x, None).unwrap();
        g.claim(a, y, None).unwrap();
        assert!(g.release(b, x).is_ok());
        assert!(g
            .release(b, x)
            .unwrap_err()
            .starts_with("release: status ready"));
        assert_eq!(g.ledger.back().unwrap().op, "release");
    }

    /// A finished node can be reopened and claimed again; a live one cannot
    /// be reopened.
    #[test]
    fn reopen_brings_a_terminal_node_back_to_ready() {
        let mut g = WorkGraph::default();
        let a = id(1);
        let x = id(20);
        upsert_ready(&mut g, a, "A");
        assert!(g.reopen(a, x).unwrap_err().contains("not terminal"));
        g.claim(a, x, None).unwrap();
        g.complete(a, WorkStatus::Cancelled, "paused", x, None)
            .unwrap();
        assert!(
            g.claim(a, x, None).is_err(),
            "a terminal node is not claimable"
        );
        let generation = g.reopen(a, x).unwrap();
        let node = g.get(a).unwrap();
        assert_eq!(node.status, WorkStatus::Ready);
        assert!(node.assignee.is_zero());
        assert_eq!(node.cas_gen, generation);
        g.claim(a, x, None).unwrap();
        assert_eq!(g.get(a).unwrap().status, WorkStatus::Claimed);
        assert_eq!(g.ledger.iter().filter(|e| e.op == "reopen").count(), 1);
    }

    #[test]
    fn claim_concurrent_same_assignee_one_live() {
        use std::sync::{Arc, Mutex};
        let mut g = WorkGraph::default();
        let a = id(1);
        let b = id(2);
        let x = id(20);
        upsert_ready(&mut g, a, "A");
        upsert_ready(&mut g, b, "B");
        let g = Arc::new(Mutex::new(g));
        let g_a = Arc::clone(&g);
        let g_b = Arc::clone(&g);
        let t_a = std::thread::spawn(move || g_a.lock().unwrap().claim(a, x, None));
        let t_b = std::thread::spawn(move || g_b.lock().unwrap().claim(b, x, None));
        let r_a = t_a.join().expect("claim A thread");
        let r_b = t_b.join().expect("claim B thread");
        assert_eq!(
            u8::from(r_a.is_ok()) + u8::from(r_b.is_ok()),
            1,
            "exactly one of X-on-A / X-on-B succeeds; A={r_a:?} B={r_b:?}"
        );
        let busy = match (r_a, r_b) {
            (Err(losing), _) | (_, Err(losing)) => losing,
            (Ok(_), Ok(_)) => unreachable!("the assertion above ruled this out"),
        };
        assert!(
            busy.starts_with("claim: assignee busy"),
            "loser must be occupancy; got {busy}"
        );
        let g = g.lock().unwrap();
        let live: Vec<_> = g
            .list()
            .into_iter()
            .filter(|n| {
                n.assignee == x && matches!(n.status, WorkStatus::Claimed | WorkStatus::Running)
            })
            .collect();
        assert_eq!(live.len(), 1, "exactly one Claimed|Running for X");
    }

    #[test]
    fn reject_cycle() {
        let mut g = WorkGraph::default();
        let a = id(1);
        let b = id(2);
        g.upsert(
            a,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: id(9),
                summary: "",
            },
        )
        .unwrap();
        g.upsert(
            b,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: id(9),
                summary: "",
            },
        )
        .unwrap();
        g.link_dep(a, b, id(9)).unwrap();
        assert!(g.link_dep(b, a, id(9)).is_err());
    }

    #[test]
    fn mint_nonzero() {
        let mut g = WorkGraph::default();
        let id = g
            .upsert(
                WorkId::ZERO,
                WorkFields {
                    kind: WorkKind::Goal,
                    status: WorkStatus::Ready,
                    role: WorkRole::Orchestrator,
                    parent: WorkId::ZERO,
                    actor: WorkId::ZERO,
                    summary: "mint me",
                },
            )
            .unwrap();
        assert!(!id.is_zero());
    }

    /// The head of a chain outranks a leaf touched later.
    #[test]
    fn a_deep_chain_outranks_a_leaf_touched_later() {
        let mut g = WorkGraph::default();
        let make = |g: &mut WorkGraph, at: u64| {
            let node = id(at);
            g.upsert(
                node,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Ready,
                    role: WorkRole::Unset,
                    parent: WorkId::ZERO,
                    actor: id(99),
                    summary: "x",
                },
            )
            .unwrap();
            node
        };

        // A chain of four, head first, and a leaf beside it.
        let chain: Vec<WorkId> = (1..=4).map(|at| make(&mut g, at)).collect();
        for pair in chain.windows(2) {
            g.link_dep(pair[0], pair[1], id(99)).unwrap();
        }
        let leaf = make(&mut g, 9);

        if let Some(node) = g.nodes.get_mut(&leaf) {
            node.updated_unix = u64::MAX;
        }

        let depth = g.critical_depth();
        assert_eq!(depth.get(&chain[0]).copied(), Some(3));
        assert_eq!(depth.get(&chain[3]).copied(), Some(0));
        assert_eq!(depth.get(&leaf).copied(), Some(0));

        let ready: Vec<WorkId> = g.ready_view().into_iter().map(|n| n.id).collect();
        assert_eq!(ready, vec![chain[0], leaf], "recency won over the chain");

        g.claim(chain[0], id(99), None).unwrap();
        g.complete(chain[0], WorkStatus::Done, "", id(99), None)
            .unwrap();
        let depth = g.critical_depth();
        assert_eq!(depth.get(&chain[1]).copied(), Some(2));
        let ready: Vec<WorkId> = g.ready_view().into_iter().map(|n| n.id).collect();
        assert_eq!(ready, vec![chain[1], leaf]);
    }

    /// Everything the ready view offers, claim accepts.
    #[test]
    fn everything_ready_can_actually_be_claimed() {
        let mut g = WorkGraph::default();
        let mut made = Vec::new();
        for at in 1..=5u64 {
            let node = id(at);
            g.upsert(
                node,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Ready,
                    role: WorkRole::Unset,
                    parent: WorkId::ZERO,
                    actor: id(50),
                    summary: "x",
                },
            )
            .unwrap();
            made.push(node);
        }
        g.link_dep(made[0], made[1], id(50)).unwrap();
        g.link_dep(made[2], made[3], id(50)).unwrap();

        let ready: Vec<WorkId> = g.ready_view().into_iter().map(|n| n.id).collect();
        assert!(!ready.contains(&made[1]), "an unblocked node was offered");
        assert!(!ready.contains(&made[3]), "an unblocked node was offered");
        for (nth, node) in ready.iter().enumerate() {
            let taker = id(100 + nth as u64);
            g.claim(*node, taker, None)
                .unwrap_or_else(|e| panic!("ready offered {node:?} and claim refused it: {e}"));
        }
    }

    #[test]
    fn upsert_rejects_claim_status() {
        let mut g = WorkGraph::default();
        let a = id(7);
        g.upsert(
            a,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: id(9),
                summary: "x",
            },
        )
        .unwrap();
        assert!(g
            .upsert(
                a,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Claimed,
                    role: WorkRole::Unset,
                    parent: WorkId::ZERO,
                    actor: id(9),
                    summary: "",
                },
            )
            .is_err());
        // Initial cas_gen is 1 (wire 0 = ignore).
        assert_eq!(g.get(a).unwrap().cas_gen, 1);
    }

    #[test]
    fn snap_roundtrip_keeps_done_node() {
        let dir = std::env::temp_dir().join(format!("claimdag-work-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut g = WorkGraph::default();
        let a = id(42);
        g.upsert(
            a,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Implementor,
                parent: WorkId::ZERO,
                actor: id(9),
                summary: "persist me",
            },
        )
        .unwrap();
        g.complete(a, WorkStatus::Done, "done on disk", id(9), None)
            .unwrap();
        g.save_dir(&dir).unwrap();
        let loaded = WorkGraph::load_dir(&dir);
        let n = loaded.get(a).expect("done node reloads");
        assert_eq!(n.status, WorkStatus::Done);
        assert_eq!(n.kind, WorkKind::Task);
        assert_eq!(n.summary, "done on disk");
        assert!(n.finished_unix > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_requires_terminal_and_hides_from_live_view() {
        let mut g = WorkGraph::default();
        let a = id(7);
        g.upsert(
            a,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: id(9),
                summary: "live",
            },
        )
        .unwrap();
        assert_eq!(
            g.archive(a, id(9)).unwrap_err(),
            format!(
                "archive: not terminal. node {} is ready. claimdag complete {} --force. claimdag list --all",
                a.to_hex(),
                a.to_hex()
            )
        );
        g.complete(a, WorkStatus::Done, "done", id(9), None)
            .unwrap();
        assert_eq!(g.list_view(false, false).len(), 0);
        assert_eq!(g.list_view(true, false).len(), 1);
        g.archive(a, id(9)).unwrap();
        assert!(g.get(a).unwrap().archived);
        assert!(g.list_view(true, false).is_empty());
        assert_eq!(g.list_view(true, true).len(), 1);
        g.unarchive(a, id(9)).unwrap();
        assert!(!g.get(a).unwrap().archived);
        assert_eq!(g.get(a).unwrap().status, WorkStatus::Done);

        let dir = std::env::temp_dir().join(format!("claimdag-arch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        g.archive(a, id(9)).unwrap();
        g.save_dir(&dir).unwrap();
        let loaded = WorkGraph::load_dir(&dir);
        assert!(loaded.get(a).unwrap().archived);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The three ways there is no graph, which a caller cannot tell apart from
    /// an empty one and which mean different things.
    #[test]
    fn an_absent_graph_says_which_kind_of_absent() {
        let base = std::env::temp_dir().join(format!("claimdag-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        let missing = base.join("never-existed");
        assert!(matches!(
            WorkGraph::open_dir(&missing),
            Err(Absent::NoDirectory(ref at)) if *at == missing
        ));
        assert!(
            WorkGraph::open_dir(&missing)
                .unwrap_err()
                .to_string()
                .contains("nothing has been claimed"),
            "the message has to say what to do about it"
        );
        // A writer still starts cold there, which is the whole reason the two
        // views exist.
        assert!(WorkGraph::load_dir(&missing).list().is_empty());

        let empty = base.join("no-snapshot");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(matches!(
            WorkGraph::open_dir(&empty),
            Err(Absent::NoSnapshot(ref at)) if *at == empty
        ));

        let broken = base.join("corrupt");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(crate::snap::SNAP_BIN), b"not a snapshot").unwrap();
        // The worst of the three: a graph that exists and cannot be read used
        // to report as a graph with nothing in it.
        assert!(matches!(
            WorkGraph::open_dir(&broken),
            Err(Absent::Unreadable(_, _))
        ));

        let good = base.join("real");
        std::fs::create_dir_all(&good).unwrap();
        WorkGraph::default().save_dir(&good).unwrap();
        assert!(WorkGraph::open_dir(&good).unwrap().list().is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn load_dir_missing_is_empty() {
        let dir =
            std::env::temp_dir().join(format!("claimdag-work-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = WorkGraph::load_dir(&dir);
        assert!(g.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snap_roundtrip_work_bin() {
        let dir = std::env::temp_dir().join(format!("claimdag-work-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut g = WorkGraph::default();
        let a = id(7);
        upsert_ready(&mut g, a, "bin");
        g.save_dir(&dir).unwrap();
        assert!(dir.join(SNAP_FILE).is_file());
        let loaded = WorkGraph::load_dir(&dir);
        assert_eq!(loaded.get(a).unwrap().summary, "bin");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unlink_drops_edge_and_readies() {
        let mut g = WorkGraph::default();
        let a = id(1);
        let b = id(2);
        upsert_ready(&mut g, a, "A");
        g.upsert(
            b,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: id(9),
                summary: "B",
            },
        )
        .unwrap();
        g.link_dep(a, b, id(9)).unwrap();
        assert_eq!(g.get(b).unwrap().status, WorkStatus::Todo);
        g.unlink_dep(a, b, id(9)).unwrap();
        assert!(g.get(b).unwrap().deps.is_empty());
        assert_eq!(g.get(b).unwrap().status, WorkStatus::Ready);
        // Missing edge is Ok; recompute_ready stays Ready.
        g.unlink_dep(a, b, id(9)).unwrap();
        assert!(g.get(b).unwrap().deps.is_empty());
        assert_eq!(g.get(b).unwrap().status, WorkStatus::Ready);
    }

    #[test]
    fn unlink_missing_edge_is_ok() {
        let mut g = WorkGraph::default();
        let a = id(1);
        let b = id(2);
        upsert_ready(&mut g, a, "A");
        g.upsert(
            b,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: id(9),
                summary: "B",
            },
        )
        .unwrap();
        g.unlink_dep(a, b, id(9)).unwrap();
        assert!(g.get(b).unwrap().deps.is_empty());
        assert_eq!(g.get(b).unwrap().status, WorkStatus::Ready);
    }

    #[test]
    fn sticky_terminal_rejects_reopen() {
        let mut g = WorkGraph::default();
        let a = id(1);
        upsert_ready(&mut g, a, "A");
        g.complete(a, WorkStatus::Done, "done", id(9), None)
            .unwrap();
        assert_eq!(g.get(a).unwrap().status, WorkStatus::Done);
        let err = g
            .upsert(
                a,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Ready,
                    role: WorkRole::Unset,
                    parent: WorkId::ZERO,
                    actor: id(9),
                    summary: "reopen",
                },
            )
            .unwrap_err();
        assert_eq!(
            err,
            format!(
                "upsert: node is terminal. node {} is done. claimdag reopen {}",
                a.to_hex(),
                a.to_hex()
            )
        );
        let after_upsert = g.get(a).unwrap();
        assert_eq!(after_upsert.status, WorkStatus::Done);
        assert_eq!(after_upsert.summary, "done");
        assert_eq!(g.claim(a, id(10), None).unwrap_err(), "claim: status done");
        g.complete(a, WorkStatus::Failed, "still done", id(9), None)
            .unwrap();
        let after_complete = g.get(a).unwrap();
        assert_eq!(after_complete.status, WorkStatus::Done);
        assert_eq!(after_complete.summary, "still done");

        let b = id(2);
        upsert_ready(&mut g, b, "B");
        g.complete(b, WorkStatus::Cancelled, "nope", id(9), None)
            .unwrap();
        assert_eq!(
            g.upsert(
                b,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Todo,
                    role: WorkRole::Unset,
                    parent: WorkId::ZERO,
                    actor: id(9),
                    summary: "",
                },
            )
            .unwrap_err(),
            format!(
                "upsert: node is terminal. node {} is cancelled. claimdag reopen {}",
                b.to_hex(),
                b.to_hex()
            )
        );
        assert_eq!(g.get(b).unwrap().status, WorkStatus::Cancelled);
        assert_eq!(
            g.claim(b, id(10), None).unwrap_err(),
            "claim: status cancelled"
        );
    }

    #[test]
    fn test_role_affinity_tiers() {
        assert_eq!(role_affinity_tier(WorkRole::Explore, WorkRole::Explore), 0);
        assert_eq!(role_affinity_tier(WorkRole::Unset, WorkRole::Explore), 1);
        assert_eq!(role_affinity_tier(WorkRole::General, WorkRole::Explore), 1);
        assert_eq!(
            role_affinity_tier(WorkRole::Implementor, WorkRole::Explore),
            2
        );
        assert_eq!(role_affinity_tier(WorkRole::General, WorkRole::General), 0);
        assert_eq!(role_affinity_tier(WorkRole::Unset, WorkRole::General), 0);
        assert_eq!(role_affinity_tier(WorkRole::Explore, WorkRole::General), 1);
    }

    #[test]
    fn ready_view_balanced_prioritizes_role_affinity() {
        let mut g = WorkGraph::default();
        let a = id(1); // Explore
        let b = id(2); // Implementor
        let c = id(3); // Verifier
        let actor = id(99);

        g.upsert(
            a,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Explore,
                parent: WorkId::ZERO,
                actor,
                summary: "explore task",
            },
        )
        .unwrap();
        g.upsert(
            b,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Implementor,
                parent: WorkId::ZERO,
                actor,
                summary: "implementor task",
            },
        )
        .unwrap();
        g.upsert(
            c,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Verifier,
                parent: WorkId::ZERO,
                actor,
                summary: "verifier task",
            },
        )
        .unwrap();

        // Implementor worker should see Implementor task first
        let worker_imp = id(10);
        let ready_imp = g.ready_view_balanced(WorkRole::Implementor, worker_imp, 1);
        assert_eq!(ready_imp[0].id, b);

        // Verifier worker should see Verifier task first
        let worker_ver = id(20);
        let ready_ver = g.ready_view_balanced(WorkRole::Verifier, worker_ver, 1);
        assert_eq!(ready_ver[0].id, c);

        // Explorer worker should see Explore task first
        let worker_exp = id(30);
        let ready_exp = g.ready_view_balanced(WorkRole::Explore, worker_exp, 1);
        assert_eq!(ready_exp[0].id, a);
    }

    #[test]
    fn ready_view_balanced_respects_critical_slack() {
        let mut g = WorkGraph::default();
        let a = id(1); // Critical chain head (depth 2)
        let a_dep = id(11);
        let a_dep2 = id(12);
        let b = id(2); // Leaf task (depth 0), role = Implementor
        let actor = id(99);

        g.upsert(
            a,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::General,
                parent: WorkId::ZERO,
                actor,
                summary: "critical chain head",
            },
        )
        .unwrap();
        g.upsert(
            a_dep,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::General,
                parent: WorkId::ZERO,
                actor,
                summary: "critical chain mid",
            },
        )
        .unwrap();
        g.upsert(
            a_dep2,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::General,
                parent: WorkId::ZERO,
                actor,
                summary: "critical chain tail",
            },
        )
        .unwrap();
        g.link_dep(a, a_dep, actor).unwrap();
        g.link_dep(a_dep, a_dep2, actor).unwrap();

        g.upsert(
            b,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Implementor,
                parent: WorkId::ZERO,
                actor,
                summary: "leaf implementor task",
            },
        )
        .unwrap();

        // With slack = 0, critical path task 'a' (depth 2) MUST outrank leaf 'b' (depth 0)
        // even for an Implementor worker who would prefer 'b' in role affinity
        let worker_imp = id(10);
        let ready_strict = g.ready_view_balanced(WorkRole::Implementor, worker_imp, 0);
        assert_eq!(ready_strict[0].id, a, "slack 0 preserves critical chain");

        // With slack = 2, 'b' is within slack of 'a' (depth 2 - 2 <= 0), so role affinity wins!
        let ready_slack = g.ready_view_balanced(WorkRole::Implementor, worker_imp, 2);
        assert_eq!(
            ready_slack[0].id, b,
            "slack 2 allows role affinity matching"
        );
    }

    #[test]
    fn claim_next_atomically_claims_and_fences() {
        let mut g = WorkGraph::default();
        let a = id(1);
        let b = id(2);
        let actor = id(99);

        g.upsert(
            a,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Explore,
                parent: WorkId::ZERO,
                actor,
                summary: "task A",
            },
        )
        .unwrap();
        g.upsert(
            b,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Implementor,
                parent: WorkId::ZERO,
                actor,
                summary: "task B",
            },
        )
        .unwrap();

        let worker1 = id(10);
        let (claimed_id, gen) = g
            .claim_next(worker1, WorkRole::Implementor, 1)
            .expect("claims next work");
        assert_eq!(claimed_id, b, "claimed matching implementor task");
        assert_eq!(gen, 2, "generation bumped from 1 to 2");

        let node = g.get(b).unwrap();
        assert_eq!(node.status, WorkStatus::Claimed);
        assert_eq!(node.assignee, worker1);
        assert_eq!(node.cas_gen, 2);

        // Worker1 cannot claim another node while holding B
        let busy_err = g
            .claim_next(worker1, WorkRole::Explore, 1)
            .expect_err("assignee busy");
        assert!(busy_err.contains("assignee busy"), "{busy_err}");

        // Worker2 can claim remaining task A
        let worker2 = id(20);
        let (claimed_id2, gen2) = g
            .claim_next(worker2, WorkRole::Explore, 1)
            .expect("claims task A");
        assert_eq!(claimed_id2, a);
        assert_eq!(gen2, 2);

        // No more ready tasks
        let worker3 = id(30);
        let no_work_err = g
            .claim_next(worker3, WorkRole::Unset, 1)
            .expect_err("no ready work");
        assert_eq!(no_work_err, "claim-next: no ready work");
    }
}
