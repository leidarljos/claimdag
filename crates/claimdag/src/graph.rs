//! Claim/complete DAG. Host process is the sole mutator.
//!
//! Identity is [`WorkId`] (xxh3-128). Closed enums for kind/status/role.
//! One open text field: summary. Snapshot is unpacked Cap'n `work.bin`
//! (mmap).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::id::{mint_work_id, WorkId};

/// How long a claim stands before anybody may take it back, in seconds.
///
/// Every seat that can reclaim has to mean the same thing by "quiet", or one
/// of them takes back a node another still considers held. Fifteen minutes is
/// long enough that a unit of work in progress is not swept out from under
/// somebody, and short enough that a crashed worker does not hold a node and
/// an identity for a sitting.
pub const DEFAULT_LEASE_SECS: u64 = 900;

/// Snapshot format written by this crate.
pub const FORMAT_V1: &str = "claimdag/v1";

/// Snapshot filename under the host state dir (unpacked Cap'n, mmap).
pub const SNAP_FILE: &str = crate::snap::SNAP_BIN;

const MAX_WORK_NODES: usize = 4096;
const MAX_LEDGER: usize = 8192;

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
                "the work graph at {} could not be read: {why}",
                dir.display()
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
            self.nodes.remove(&id);
        }
    }

    /// What an upsert is asserting about a node.
    ///
    /// Eight positional arguments where five of them describe the same node is
    /// a call nobody can read at the site, and two ids of the same type next to
    /// each other is a swap waiting to happen.
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
        // The parent chain is a forest and nothing else enforced that. A
        // dependency edge is checked for a loop and a parent edge was not, so
        // `upsert --id X --parent X` was accepted and any reader that walks up
        // from a node, or draws the tree, ran forever on it.
        if !parent.is_zero() && (parent == id || self.parent_reaches(parent, id)) {
            return Err("upsert: parent cycle".into());
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
            return Err("upsert: node is terminal".into());
        }
        // A live claim is claim's to end, not upsert's. Demoting a Claimed node
        // to Todo leaves the assignee in place and lets a second agent claim
        // work the first is holding, with neither claim ever failing.
        let live = matches!(entry.status, WorkStatus::Claimed | WorkStatus::Running);
        if live && status != entry.status {
            return Err("upsert: node is claimed; use completeWork".into());
        }
        // Lifecycle authority: claim/complete own Claimed/Running/terminal.
        // Upsert only metadata + Todo|Ready|Blocked.
        match status {
            WorkStatus::Todo | WorkStatus::Ready | WorkStatus::Blocked => {
                if !live {
                    entry.status = status;
                }
            }
            WorkStatus::Claimed
            | WorkStatus::Running
            | WorkStatus::Done
            | WorkStatus::Failed
            | WorkStatus::Cancelled => {
                return Err(
                    "upsert: status claim/run/terminal requires claimWork or completeWork".into(),
                );
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
        // Readiness is derived, never asserted: a caller may ask for Ready and
        // the dependencies decide. Otherwise the status lies to every reader
        // who uses it to pick up workable work, and the claim being refused
        // later does not help the one who believed it.
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
            return Err("link: parent missing".into());
        }
        if !self.nodes.contains_key(&child) {
            return Err("link: child missing".into());
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
            .ok_or_else(|| "unlink: child missing".to_string())?;
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

    /// Whether walking up from `from` reaches `target`.
    ///
    /// Bounded by the visited set rather than by depth, because a chain that
    /// is already circular is exactly the input this has to survive.
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

    /// How long a chain still waits below each node.
    ///
    /// A scheduler handing out one ready node at a time has to answer which
    /// one, and the answer since Hu (doi:10.1287/opre.9.6.841) is the head of
    /// the longest chain of remaining work. Starting a leaf nothing waits on,
    /// while the head of a chain of nine sits ready, delays all nine by
    /// however long the leaf takes; Graham (doi:10.1137/0117039) is what
    /// bounds how bad a list schedule can be, and every priority in the DAG
    /// scheduling literature since (doi:10.1145/344588.344618, and HEFT at
    /// doi:10.1109/71.993206) is a refinement of this one.
    ///
    /// The cost model is unit, because the graph carries no durations. Every
    /// remaining node counts one, which is the case Hu's result covers and the
    /// honest model for what this stores. A node with nothing unfinished below
    /// it has depth zero.
    ///
    /// Done and archived nodes are not below anything: what is already
    /// finished cannot delay what waits on it.
    #[must_use]
    pub fn critical_depth(&self) -> HashMap<WorkId, u32> {
        // Successors, since the nodes carry their parents. Built once rather
        // than scanned per node, which would make this quadratic on a graph
        // that is usually shallow and wide.
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

    /// One node's depth, memoised, with the path held so a cycle terminates.
    ///
    /// The graph refuses a cycle on the way in, so this cannot happen through
    /// the API. It is still written to terminate on one, because a walk that
    /// diverges on a corrupt file is a scheduler that hangs rather than one
    /// that reports a corrupt file.
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

    /// The nodes a seat may take right now, deepest chain first.
    ///
    /// Ready is unblocked and unfinished, which is the same predicate `claim`
    /// enforces, so a surface that filters for itself can drift from what the
    /// graph will actually accept. This is the one definition.
    ///
    /// The order is the scheduling decision: critical depth first, then the
    /// most recently touched, then the id. Recency is a tie-break rather than
    /// the sort, because the most recently touched ready node is usually the
    /// one somebody just created or just unblocked, which says nothing about
    /// what waits on it.
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

    fn deps_satisfied(&self, node: &WorkNode) -> bool {
        node.deps.iter().all(|d| {
            self.nodes
                .get(d)
                .map(|p| p.status == WorkStatus::Done)
                .unwrap_or(false)
        })
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
        let deps: Vec<WorkId> = {
            let node = self
                .nodes
                .get(&id)
                .ok_or_else(|| "claim: not found".to_string())?;
            if let Some(g) = expected_gen {
                if node.cas_gen != g {
                    return Err("claim: gen mismatch".into());
                }
            }
            if !matches!(node.status, WorkStatus::Ready | WorkStatus::Todo) {
                return Err(format!("claim: status {}", node.status.as_str()));
            }
            node.deps.clone()
        };
        let deps_satisfied = deps.iter().all(|d| {
            self.nodes
                .get(d)
                .map(|p| p.status == WorkStatus::Done)
                .unwrap_or(false)
        });
        if !deps_satisfied {
            return Err("claim: deps unsatisfied".into());
        }
        // Occupancy is a graph predicate: one Claimed|Running node per assignee.
        // Same mutex as the gen bump (caller holds work.lock()). Skip `id`.
        let mut busy: Vec<WorkId> = self
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
            busy.sort();
            let listed = busy
                .iter()
                .map(|held| held.to_hex())
                .collect::<Vec<_>>()
                .join(" ");
            return Err(format!("claim: assignee busy {listed}"));
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

    /// Hand back every claim that has gone quiet for longer than the lease.
    ///
    /// A claim with no expiry is a claim a crashed worker keeps. The node
    /// stays `Claimed` with nobody working it, and because occupancy allows
    /// one held node per assignee, that worker's id can never claim anything
    /// again either. One process dying takes a node and an identity with it,
    /// permanently, which is not a property a scheduler can have.
    ///
    /// So a claim is a lease. `renew` says the holder is alive; going quiet
    /// past `lease_secs` returns the node to `Ready` for somebody else. The
    /// generation moves on the way out, which is what fences the holder that
    /// wakes up later still believing it owns the node.
    ///
    /// Leases rather than heartbeat-and-evict because the graph has one
    /// mutator and no way to ask whether a worker is alive: the only evidence
    /// available is whether it said so recently. Chubby's lease and
    /// ZooKeeper's ephemeral node answer the same question the same way.
    ///
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

    /// The holder says it is still working.
    ///
    /// Moves the lease forward and nothing else. The generation deliberately
    /// stays put: a renewal is not a change of ownership, and bumping it would
    /// invalidate the token the holder is about to complete with.
    pub fn renew(&mut self, id: WorkId, actor: WorkId) -> Result<u64, String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| "renew: not found".to_string())?;
        if !matches!(node.status, WorkStatus::Claimed | WorkStatus::Running) {
            return Err(format!("renew: status {}", node.status.as_str()));
        }
        if !node.assignee.is_zero() && !actor.is_zero() && node.assignee != actor {
            return Err("renew: not assignee".into());
        }
        node.updated_unix = Self::now();
        Ok(node.cas_gen)
    }

    pub fn set_running(&mut self, id: WorkId, actor: WorkId) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| "running: not found".to_string())?;
        if !matches!(node.status, WorkStatus::Claimed | WorkStatus::Running) {
            return Err(format!("running: status {}", node.status.as_str()));
        }
        // The same question complete asks, and it has to answer the same way,
        // or the ledger records a stranger as having started somebody's work.
        // A zero actor is the command line's escape, as it is there.
        if !node.assignee.is_zero() && !actor.is_zero() && node.assignee != actor {
            return Err("running: not assignee".into());
        }
        node.status = WorkStatus::Running;
        node.updated_unix = Self::now();
        self.push_ledger(id, actor, "running");
        Ok(())
    }

    /// Finish a node, optionally fencing on the generation held at claim time.
    ///
    /// `expected_gen` is a fencing token. A holder that stalled long enough to
    /// be reclaimed wakes up believing it still owns the node, and the
    /// assignee check alone does not stop it: a reclaim clears the assignee,
    /// and a cleared assignee is what lets anybody finish an unheld node. The
    /// generation moves on every claim and every reclaim, so a caller that
    /// passes the one it was given is refused exactly when the world moved
    /// underneath it.
    ///
    /// `None` skips the check, which is the command line's escape and the only
    /// way to speak for a node nobody claimed.
    pub fn complete(
        &mut self,
        id: WorkId,
        status: WorkStatus,
        summary: &str,
        actor: WorkId,
        expected_gen: Option<u64>,
    ) -> Result<(), String> {
        if !status.is_terminal() {
            return Err("complete: status not terminal".into());
        }
        if id.is_zero() {
            return Err("complete: zero id".into());
        }
        let now = Self::now();
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| "complete: not found".to_string())?;
        if let Some(g) = expected_gen {
            if node.cas_gen != g {
                return Err("complete: gen mismatch".into());
            }
        }
        // Whoever held it is who may still speak for it. The guard runs before
        // the terminal case, not after: a finished node is exactly the one a
        // stranger would otherwise be able to rewrite, since the early return
        // used to be reached first.
        if !node.assignee.is_zero() && !actor.is_zero() && node.assignee != actor {
            return Err("complete: not assignee".into());
        }
        if node.status.is_terminal() {
            if !summary.is_empty() {
                node.summary = summary.to_string();
            }
            return Ok(());
        }
        node.status = status;
        if !summary.is_empty() {
            node.summary = summary.to_string();
        }
        node.finished_unix = now;
        node.updated_unix = now;
        self.push_ledger(id, actor, "complete");
        self.promote_dependents(id);
        Ok(())
    }

    pub fn archive(&mut self, id: WorkId, actor: WorkId) -> Result<(), String> {
        if id.is_zero() {
            return Err("archive: zero id".into());
        }
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| "archive: not found".to_string())?;
        if !node.status.is_terminal() {
            return Err("archive: not terminal".into());
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
            .ok_or_else(|| "unarchive: not found".to_string())?;
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

    /// Load `$dir/work.bin` (mmap Cap'n), or empty if missing.
    /// Why a directory holds no graph to read.
    ///
    /// The three cases read identically to a caller that gets an empty list,
    /// and they mean different things: nothing claimed yet, no seat here at
    /// all, and a snapshot that cannot be parsed. A writer treats all three as
    /// a cold start, which is right. A reader answering "nothing is claimable"
    /// from the second or third is answering a question it did not check.
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

    /// The graph in a directory, empty when there is none.
    ///
    /// A writer's view: a seat with nothing claimed and a seat with no
    /// directory are the same cold start to something about to write. Readers
    /// want [`WorkGraph::open_dir`], which says which of the two it found.
    pub fn load_dir(dir: &Path) -> Self {
        match crate::snap::read_bin(dir) {
            Ok((next_seq, mint_seq, nodes)) => {
                let mut g = Self {
                    nodes: HashMap::new(),
                    ledger: VecDeque::new(),
                    next_seq,
                    mint_seq,
                };
                for node in nodes {
                    g.nodes.insert(node.id, node);
                }
                g
            }
            Err(_) => Self::default(),
        }
    }

    /// Atomic replace of `$dir/work.bin` (unpacked Cap'n).
    pub fn save_dir(&self, dir: &Path) -> Result<(), String> {
        crate::snap::write_bin(dir, self.next_seq, self.mint_seq, &self.list())
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
            "claim: gen mismatch"
        );
        let cas_gen = g.claim(b, id(10), None).unwrap();
        assert_eq!(
            g.claim(b, id(11), Some(1)).unwrap_err(),
            "claim: gen mismatch"
        );
        assert!(g.claim(b, id(11), Some(cas_gen)).is_err());
        g.set_running(b, id(10)).unwrap();
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

    /// The head of a long chain outranks a leaf nothing waits on, however
    /// recently the leaf was touched.
    ///
    /// This is the whole reason the order is not recency. Handing out the leaf
    /// first delays every node in the chain by however long the leaf takes,
    /// and nothing anywhere would report that it happened.
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

        // The leaf is the most recently touched, which is exactly what the old
        // order sorted by.
        if let Some(node) = g.nodes.get_mut(&leaf) {
            node.updated_unix = u64::MAX;
        }

        let depth = g.critical_depth();
        assert_eq!(depth.get(&chain[0]).copied(), Some(3));
        assert_eq!(depth.get(&chain[3]).copied(), Some(0));
        assert_eq!(depth.get(&leaf).copied(), Some(0));

        // Only the head and the leaf are unblocked, and the head goes first.
        let ready: Vec<WorkId> = g.ready_view().into_iter().map(|n| n.id).collect();
        assert_eq!(ready, vec![chain[0], leaf], "recency won over the chain");

        // Finishing the head moves the frontier down the chain, and what is
        // already done stops counting toward anybody's depth.
        g.claim(chain[0], id(99), None).unwrap();
        g.complete(chain[0], WorkStatus::Done, "", id(99), None)
            .unwrap();
        let depth = g.critical_depth();
        assert_eq!(depth.get(&chain[1]).copied(), Some(2));
        let ready: Vec<WorkId> = g.ready_view().into_iter().map(|n| n.id).collect();
        assert_eq!(ready, vec![chain[1], leaf]);
    }

    /// The ready view is the predicate `claim` enforces, not a second one.
    ///
    /// Two definitions of ready is one that drifts, and the surface that
    /// drifts offers work the graph then refuses.
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
        // Each one a fresh assignee, since occupancy allows one held node per
        // identity and that is a different refusal from an unsatisfied dep.
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
        assert_eq!(g.archive(a, id(9)).unwrap_err(), "archive: not terminal");
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
        assert_eq!(err, "upsert: node is terminal");
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
            "upsert: node is terminal"
        );
        assert_eq!(g.get(b).unwrap().status, WorkStatus::Cancelled);
        assert_eq!(
            g.claim(b, id(10), None).unwrap_err(),
            "claim: status cancelled"
        );
    }
}
