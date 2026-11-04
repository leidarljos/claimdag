//! The claim graph over MCP, on the snapshot the pane and the CLI use.
//! `claim` returns a generation; `complete` requires it; `renew` leaves it.

use std::path::PathBuf;

use claimdag::{Absent, WorkFields, WorkGraph, WorkId, WorkKind, WorkNode, WorkRole, WorkStatus};
use rmcp::{
    handler::server::tool::IntoCallToolResult, handler::server::wrapper::Parameters,
    handler::server::ServerHandler, model::*, prompt_handler, tool, tool_handler, tool_router,
    ErrorData as McpError,
};
use serde::Serialize;

use crate::args::*;

#[derive(Clone)]
pub struct ClaimdagServer {
    /// Set when `CLAIMDAG_DIR` named the directory, or a test opened one.
    /// A named directory stays fixed. `None` means the defaults, which
    /// `dir` resolves again on each call.
    dir: Option<PathBuf>,
}

/// One node, as an agent reads it.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct NodeRow {
    /// 32 hex characters.
    pub id: String,
    /// `todo`, `ready`, `claimed`, `running`, `blocked`, or a terminal.
    pub status: String,
    /// What the work is.
    pub summary: String,
    /// Who holds it, when anybody does.
    pub assignee: Option<String>,
    /// The generation to pass back when finishing it.
    pub generation: u64,
    /// Ids that have to finish first.
    pub blocked_by: Vec<String>,
    /// How long a held node has gone without word, in seconds.
    ///
    /// The number the lease is measured against. A caller deciding whether to
    /// reclaim needs it, and a status of `claimed` reads the same whether the
    /// holder is working or gone.
    pub quiet_seconds: Option<u64>,
    /// Unfinished work still waiting below this node; the ready order.
    pub waiting_below: u32,
}

/// A node the graph holds, or the status prune kept after dropping it.
///
/// The two answers share no extra flag. A row has the fields a live node
/// carries. A pruned node is only its id and the status it finished with.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ReadNode {
    /// The graph still holds this node.
    Row(NodeRow),
    /// Prune dropped it.
    Pruned(PrunedNode),
}

/// The status a node finished with before prune dropped it.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct PrunedNode {
    /// 32 hex characters.
    pub id: String,
    /// `done`, `failed` or `cancelled`.
    pub status: String,
}

/// What a claim gave out.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ClaimRow {
    /// The node now held.
    pub id: String,
    /// The token to carry. Pass it to `complete`, and a reclaim while you were
    /// away will refuse the finish rather than let it through.
    pub generation: u64,
}

fn row(node: &WorkNode, now: u64, waiting_below: u32) -> NodeRow {
    let held = matches!(node.status, WorkStatus::Claimed | WorkStatus::Running);
    NodeRow {
        id: node.id.to_hex(),
        status: node.status.as_str().to_string(),
        summary: node.summary.clone(),
        assignee: (!node.assignee.is_zero()).then(|| node.assignee.to_hex()),
        generation: node.cas_gen,
        blocked_by: node.deps.iter().map(|d| d.to_hex()).collect(),
        quiet_seconds: held.then(|| now.saturating_sub(node.updated_unix)),
        waiting_below,
    }
}

fn shown(graph: &WorkGraph, id: WorkId) -> Result<NodeRow, Fail> {
    let node = graph.get(id).ok_or_else(|| {
        bad(format!(
            "get: unknown id {}. claimdag list --all",
            id.to_hex()
        ))
    })?;
    let below = graph.critical_depth().get(&node.id).copied().unwrap_or(0);
    Ok(row(node, now_unix(), below))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A tool result: the row, then any warning lines as further text blocks.
///
/// The row stays the structured content, so its schema is unchanged. Each
/// warning is the same `allow`, `warn`, rule line the command line writes,
/// and it is part of the result a seat reads. Stderr and `warn.log` keep a
/// copy.
struct Json<T>(pub T, Vec<String>);

impl<T> Json<T> {
    fn plain(value: T) -> Self {
        Self(value, Vec::new())
    }

    fn noted(value: T, warnings: Vec<String>) -> Self {
        Self(value, warnings)
    }
}

impl<T: Serialize + 'static> IntoCallToolResult for Json<T> {
    fn into_call_tool_result(self) -> Result<CallToolResponse, McpError> {
        let value = serde_json::to_value(self.0).map_err(|e| {
            McpError::internal_error(format!("Failed to serialize structured content: {e}"), None)
        })?;
        let mut result = CallToolResult::structured(value);
        for line in self.1 {
            result.content.push(ContentBlock::text(line));
        }
        Ok(result.into())
    }
}

/// Why a tool call did not return a row.
///
/// A refusal is text the seat has to read, so it is a tool result with
/// `isError` set. `Err` of [`McpError`] is a JSON-RPC error, and clients
/// show that as a transport failure, some of them retrying it, so the seat
/// never reads "claim: status claimed". An id that is not 32 hex characters,
/// or a status other than done, failed or cancelled, stays invalid params:
/// the call never reached the graph. rmcp 3.3.0 keeps `Result<Json<T>, _>`
/// as the output schema and turns an error-side `CallToolResult` into that
/// tool result, while `Err(ErrorData)` stays a protocol error, so the two
/// are different variants.
#[derive(Debug)]
enum Fail {
    Refusal(String),
    Params(McpError),
}

impl IntoCallToolResult for Fail {
    fn into_call_tool_result(self) -> Result<CallToolResponse, McpError> {
        match self {
            Self::Params(err) => Err(err),
            Self::Refusal(text) => Ok(CallToolResult::error(vec![ContentBlock::text(text)]).into()),
        }
    }
}

fn parse(raw: &str) -> Result<WorkId, Fail> {
    WorkId::from_hex(raw).ok_or_else(|| invalid(format!("bad id {raw}")))
}

/// The graph, the snapshot or the lock said no. The text is what the seat reads.
fn bad(why: String) -> Fail {
    Fail::Refusal(why)
}

/// An id or a status the tool cannot take. The protocol answers invalid params.
fn invalid(why: String) -> Fail {
    Fail::Params(McpError::invalid_params(why, None))
}

#[tool_router]
impl ClaimdagServer {
    /// Open on the graph this seat keeps.
    ///
    /// `CLAIMDAG_DIR` fixes the directory. Otherwise the defaults are
    /// resolved again on each call, and this startup resolve only says,
    /// once, when another location holds work.
    #[must_use]
    pub fn from_env() -> Self {
        let named = std::env::var_os("CLAIMDAG_DIR").is_some_and(|raw| !raw.is_empty());
        // Said once at startup when another location holds work. A call uses
        // this path only when the directory was named; the defaults are
        // resolved again in `dir`.
        let resolved = claimdag::resolve_dir(None);
        Self {
            dir: named.then_some(resolved),
        }
    }

    /// Open on a named directory, for a test.
    #[cfg(test)]
    #[must_use]
    pub fn at(dir: PathBuf) -> Self {
        Self { dir: Some(dir) }
    }

    /// The directory this call reads and writes.
    ///
    /// A named directory stays the one given. The defaults are resolved
    /// again: `claimdag where --move` can put the graph in an earlier
    /// default while this process is still up, and a path kept from startup
    /// would keep reading the old one.
    ///
    /// `locate` stats at most five paths. On `claimdag_ready` with the graph
    /// in `/tmp/claimdag-UID` and the home state directory empty, 21 calls
    /// an arm across six rounds (126 an arm), a cached directory took a
    /// median of 12 µs (IQR 0 µs) and resolving each call took 15 µs
    /// (IQR 0 µs). The calls are `measurements/j78q/ready.tsv`. Resolving
    /// adds 3 µs, under a millisecond, so an unnamed call resolves every
    /// time. Intel Xeon (family 6, model 207), 8 cores, one thread per
    /// core, Linux 6.12, release.
    fn dir(&self) -> PathBuf {
        if let Some(dir) = &self.dir {
            return dir.clone();
        }
        claimdag::locate(None).dir().to_path_buf()
    }

    /// A graph to read, or the reason there is none.
    ///
    /// Reading verbs refuse an absent graph rather than answering "nothing
    /// claimable", because those are opposite answers: one is a quiet seat and
    /// the other is a caller pointed at the wrong place.
    fn reading(&self) -> Result<WorkGraph, Fail> {
        WorkGraph::open_dir(&self.dir()).map_err(|absent: Absent| bad(absent.to_string()))
    }

    /// The directory, the lock on it, and the graph a writing verb starts
    /// from. The lock is taken before the load and held by the caller across
    /// the save. A snapshot that does not parse is refused, never replaced
    /// by an empty graph and saved over.
    ///
    /// An unnamed directory is resolved again after the lock is held.
    /// `claimdag where --move` holds both locks while it moves `work.bin`,
    /// so a call that resolved the old directory and waited out that move
    /// would load no snapshot and save a new graph in the empty directory.
    /// When the defaults now name a different directory, this drops the lock
    /// and retries there. A named directory is not moved, so it is locked once.
    fn writing(&self) -> Result<(PathBuf, claimdag::lock::Lock, WorkGraph), Fail> {
        if self.dir.is_some() {
            let dir = self.dir();
            let lock = claimdag::lock_dir(&dir).map_err(bad)?;
            let graph = WorkGraph::load_for_write(&dir)
                .map_err(|absent: Absent| bad(absent.to_string()))?;
            return Ok((dir, lock, graph));
        }
        loop {
            let dir = claimdag::locate(None).dir().to_path_buf();
            let lock = claimdag::lock_dir(&dir).map_err(bad)?;
            let current = claimdag::locate(None).dir().to_path_buf();
            if current != dir {
                drop(lock);
                continue;
            }
            let graph = WorkGraph::load_for_write(&dir)
                .map_err(|absent: Absent| bad(absent.to_string()))?;
            return Ok((dir, lock, graph));
        }
    }

    /// Save, and return the warnings the call recorded.
    ///
    /// Each line is also written to stderr and `warn.log`. The caller puts
    /// the same lines in the tool result.
    fn stored(&self, dir: &std::path::Path, graph: &WorkGraph) -> Result<Vec<String>, Fail> {
        graph.save_dir(dir).map_err(bad)?;
        let warnings = graph.warnings().to_vec();
        for line in &warnings {
            claimdag::emit_warn(dir, line);
        }
        Ok(warnings)
    }

    #[tool(
        description = "Work that can be taken right now: unblocked, unheld, and not finished, deepest chain first. Take the first one: the order is the critical path, so the head of a long chain outranks a leaf nothing waits on. Ask this before anything else. An empty answer means the seat is quiet; a failure means there is no graph here, which is a different thing.",
        annotations(
            title = "Claimable work",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_ready(&self) -> Result<Json<Vec<NodeRow>>, Fail> {
        let graph = self.reading()?;
        let now = now_unix();
        let depth = graph.critical_depth();
        Ok(Json::plain(
            graph
                .ready_view()
                .into_iter()
                .map(|n| row(n, now, depth.get(&n.id).copied().unwrap_or(0)))
                .collect(),
        ))
    }

    #[tool(
        description = "Work that can be taken right now, ordered by role suitability and worker dispersion with critical slack to prevent herd contention. Take the first one.",
        annotations(
            title = "Balanced claimable work",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_ready_balanced(
        &self,
        Parameters(args): Parameters<ReadyArgs>,
    ) -> Result<Json<Vec<NodeRow>>, Fail> {
        let graph = self.reading()?;
        let now = now_unix();
        let depth = graph.critical_depth();
        let role = WorkRole::from(args.role.unwrap_or(crate::args::Role::Unset));
        // Omitted is nobody. A string that is not an id is a bad id.
        let assignee = match args.assignee.as_deref() {
            Some(raw) => parse(raw)?,
            None => WorkId::ZERO,
        };
        let slack = args.slack.unwrap_or(1);
        let nodes = if args.balanced.unwrap_or(true) {
            graph.ready_view_balanced(role, assignee, slack)
        } else {
            graph.ready_view()
        };
        Ok(Json::plain(
            nodes
                .into_iter()
                .map(|n| row(n, now, depth.get(&n.id).copied().unwrap_or(0)))
                .collect(),
        ))
    }

    #[tool(
        description = "Every live node: what is held, by whom, and how long each held one has gone without word. The quiet seconds are what a reclaim decision is made from.",
        annotations(
            title = "The live graph",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_list(
        &self,
        Parameters(args): Parameters<ListArgs>,
    ) -> Result<Json<Vec<NodeRow>>, Fail> {
        let graph = self.reading()?;
        let now = now_unix();
        let all = args.all.unwrap_or(false);
        let depth = graph.critical_depth();
        Ok(Json::plain(
            graph
                .list_view(all, all)
                .into_iter()
                .map(|n| row(n, now, depth.get(&n.id).copied().unwrap_or(0)))
                .collect(),
        ))
    }

    #[tool(
        description = "Take a node. Returns the generation to carry: pass it to complete, and a reclaim while you were away refuses the finish rather than letting it through. Refused when somebody else holds it, when a dependency is unfinished, or when you already hold something.",
        annotations(
            title = "Claim work",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_claim(
        &self,
        Parameters(args): Parameters<ClaimArgs>,
    ) -> Result<Json<ClaimRow>, Fail> {
        let id = parse(&args.id)?;
        let assignee = parse(&args.assignee)?;
        let (dir, _lock, mut graph) = self.writing()?;
        let generation = graph.claim(id, assignee, args.generation).map_err(bad)?;
        let warnings = self.stored(&dir, &graph)?;
        Ok(Json::noted(
            ClaimRow {
                id: id.to_hex(),
                generation,
            },
            warnings,
        ))
    }

    #[tool(
        description = "Atomically claim the best available ready node using role-aware and power-of-two load balancing. Avoids CAS stampedes when multiple workers seek work simultaneously. Returns the claimed node and generation token to carry.",
        annotations(
            title = "Claim next work",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn claimdag_claim_next(
        &self,
        Parameters(args): Parameters<ClaimNextArgs>,
    ) -> Result<Json<ClaimRow>, Fail> {
        let assignee = parse(&args.assignee)?;
        let role = WorkRole::from(args.role.unwrap_or(crate::args::Role::Unset));
        let slack = args.slack.unwrap_or(1);
        let (dir, _lock, mut graph) = self.writing()?;
        let (id, generation) = graph.claim_next(assignee, role, slack).map_err(bad)?;
        let warnings = self.stored(&dir, &graph)?;
        Ok(Json::noted(
            ClaimRow {
                id: id.to_hex(),
                generation,
            },
            warnings,
        ))
    }

    #[tool(
        description = "Say the holder is still working, which moves the lease and leaves the generation alone. A renewal is not a change of ownership, so the token you hold stays good. An actor who is not the holder still moves the lease. The tool result then has a text block after the row: allow, warn, and not-assignee, separated by tabs.",
        annotations(
            title = "Renew a lease",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_renew(
        &self,
        Parameters(args): Parameters<ActorArgs>,
    ) -> Result<Json<ClaimRow>, Fail> {
        let id = parse(&args.id)?;
        let actor = parse(&args.actor)?;
        let (dir, _lock, mut graph) = self.writing()?;
        let generation = graph.renew(id, actor).map_err(bad)?;
        let warnings = self.stored(&dir, &graph)?;
        Ok(Json::noted(
            ClaimRow {
                id: id.to_hex(),
                generation,
            },
            warnings,
        ))
    }

    #[tool(
        description = "Hand one claim back on purpose before the work is terminal: the node is ready again, the assignee clears, and the generation moves so the token you held is fenced. Call this when you stop working on a node without finishing it; until you do, you stay busy and cannot claim anything else. An actor who is not the holder, or the zero id, still hands it back. The tool result then has a text block after the row: allow, warn, and not-assignee or no-actor, separated by tabs.",
        annotations(
            title = "Hand a claim back",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn claimdag_release(
        &self,
        Parameters(args): Parameters<ActorArgs>,
    ) -> Result<Json<ClaimRow>, Fail> {
        let id = parse(&args.id)?;
        let actor = parse(&args.actor)?;
        let (dir, _lock, mut graph) = self.writing()?;
        let generation = graph.release(id, actor).map_err(bad)?;
        let warnings = self.stored(&dir, &graph)?;
        Ok(Json::noted(
            ClaimRow {
                id: id.to_hex(),
                generation,
            },
            warnings,
        ))
    }

    #[tool(
        description = "Bring a finished node back to ready, generation moved, so the same work can be claimed again: a new sitting on old work. No record of the earlier completion is kept on disk. Refused on a node that is not terminal.",
        annotations(
            title = "Reopen finished work",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn claimdag_reopen(
        &self,
        Parameters(args): Parameters<ActorArgs>,
    ) -> Result<Json<ClaimRow>, Fail> {
        let id = parse(&args.id)?;
        let actor = parse(&args.actor)?;
        let (dir, _lock, mut graph) = self.writing()?;
        let generation = graph.reopen(id, actor).map_err(bad)?;
        let warnings = self.stored(&dir, &graph)?;
        Ok(Json::noted(
            ClaimRow {
                id: id.to_hex(),
                generation,
            },
            warnings,
        ))
    }

    #[tool(
        description = "Complete a node. Pass the generation claim gave you: without it the complete goes through even if your lease was reclaimed and somebody else has the work. Done is refused while a dependency is unfinished, and the error names it. An actor who is not the holder, or the zero id, still finishes the node. The tool result then has a text block after the row: allow, warn, and not-assignee or no-actor, separated by tabs. Terminal is sticky.",
        annotations(
            title = "Complete the node",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_complete(
        &self,
        Parameters(args): Parameters<CompleteArgs>,
    ) -> Result<Json<NodeRow>, Fail> {
        let id = parse(&args.id)?;
        let actor = parse(&args.actor)?;
        let status = match args.status.as_deref().unwrap_or("done") {
            "done" => WorkStatus::Done,
            "failed" => WorkStatus::Failed,
            "cancelled" => WorkStatus::Cancelled,
            other => {
                return Err(invalid(format!(
                    "status is done, failed or cancelled, not {other}"
                )));
            }
        };
        let (dir, _lock, mut graph) = self.writing()?;
        graph
            .complete(
                id,
                status,
                args.summary.as_deref().unwrap_or(""),
                actor,
                args.generation,
            )
            .map_err(bad)?;
        let warnings = self.stored(&dir, &graph)?;
        if let Some(node) = graph.get(id) {
            let now = now_unix();
            let depth = graph.critical_depth();
            // A finished node has nothing unfinished below it that it is holding
            // up, which is exactly what the depth of zero says.
            let below = depth.get(&node.id).copied().unwrap_or(0);
            return Ok(Json::noted(row(node, now, below), warnings));
        }
        // Prune drops a finished node and remembers only the status it
        // finished with. The command line answers "already terminal and
        // pruned; nothing changed". This row is that answer, so a retry is
        // not an error. The node is gone, so the summary, assignee and
        // dependencies are empty, and it is not held.
        let was = graph
            .pruned(id)
            .ok_or_else(|| bad("complete: the node went missing".into()))?;
        Ok(Json::noted(
            NodeRow {
                id: id.to_hex(),
                status: was.as_str().to_string(),
                summary: String::new(),
                assignee: None,
                // Release of a pruned node answers generation 0: the node is no
                // longer in the graph, so there is no generation to carry.
                generation: 0,
                blocked_by: Vec::new(),
                quiet_seconds: None,
                // There is no chain left to measure. A node that
                // `critical_depth` does not hold is reported as zero on the
                // live path too.
                waiting_below: 0,
            },
            warnings,
        ))
    }

    #[tool(
        description = "Hand back every claim that has gone quiet longer than the lease, in seconds. A claim with no expiry is one a crashed worker keeps, along with the identity that held it. Returns what was handed back.",
        annotations(
            title = "Reclaim stale leases",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_reclaim(
        &self,
        Parameters(args): Parameters<ReclaimArgs>,
    ) -> Result<Json<Vec<String>>, Fail> {
        let (dir, _lock, mut graph) = self.writing()?;
        let handed = graph.reclaim(args.lease_seconds);
        let warnings = if !handed.is_empty() {
            self.stored(&dir, &graph)?
        } else {
            Vec::new()
        };
        Ok(Json::noted(
            handed.iter().map(|id| id.to_hex()).collect(),
            warnings,
        ))
    }

    #[tool(
        description = "One node by its id. Returns the row the graph holds, or the status prune kept when it dropped the node. An id the graph never held is refused, which is a different answer from a node that finished and was dropped.",
        annotations(
            title = "Read one node",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_get(
        &self,
        Parameters(args): Parameters<GetArgs>,
    ) -> Result<Json<ReadNode>, Fail> {
        let id = parse(&args.id)?;
        let graph = self.reading()?;
        if graph.get(id).is_some() {
            return Ok(Json::plain(ReadNode::Row(shown(&graph, id)?)));
        }
        if let Some(was) = graph.pruned(id) {
            return Ok(Json::plain(ReadNode::Pruned(PrunedNode {
                id: id.to_hex(),
                status: was.as_str().to_string(),
            })));
        }
        Err(bad(format!(
            "get: unknown id {}. claimdag list --all",
            id.to_hex()
        )))
    }

    // Minting with no id is a new node on every call, so repeating it is not
    // the same write.
    #[tool(
        description = "Create a node, or change the summary, kind, role or parent of one. Omit the id to mint one. Omit the status and the node keeps the status it has, so a blocked node stays blocked; readiness still follows the dependencies. Returns the row.",
        annotations(
            title = "Create or update a node",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn claimdag_upsert(
        &self,
        Parameters(args): Parameters<UpsertArgs>,
    ) -> Result<Json<NodeRow>, Fail> {
        let id = match args.id.as_deref() {
            None => WorkId::ZERO,
            Some(raw) => parse(raw)?,
        };
        let actor = parse(&args.actor)?;
        let summary = args.summary.unwrap_or_default();
        let parent = match args.parent.as_deref() {
            None => WorkId::ZERO,
            Some(raw) => parse(raw)?,
        };
        let kind = match args.kind.as_deref() {
            None => None,
            Some(raw) => {
                Some(WorkKind::parse_str(raw).ok_or_else(|| invalid(format!("bad kind {raw}")))?)
            }
        };
        let role = match args.role.as_deref() {
            None => WorkRole::Unset,
            Some(raw) => {
                WorkRole::parse_str(raw).ok_or_else(|| invalid(format!("bad role {raw}")))?
            }
        };
        let status = match args.status.as_deref() {
            None => None,
            Some(raw) => Some(
                WorkStatus::parse_str(raw).ok_or_else(|| invalid(format!("bad status {raw}")))?,
            ),
        };
        let (dir, _lock, mut graph) = self.writing()?;
        let present = graph.get(id).map(|node| node.status);
        // Omitting status keeps the node's status. A minted node has none yet,
        // and todo is the status a node is created with; the dependencies then
        // decide whether it is ready. Unset kind leaves a stored kind alone,
        // and a new node is a task, the command line's default.
        let status = status.unwrap_or_else(|| present.unwrap_or(WorkStatus::Todo));
        let kind = kind.unwrap_or(if present.is_some() {
            WorkKind::Unset
        } else {
            WorkKind::Task
        });
        let id = graph
            .upsert(
                id,
                WorkFields {
                    kind,
                    status,
                    role,
                    parent,
                    actor,
                    summary: &summary,
                },
            )
            .map_err(bad)?;
        let warnings = self.stored(&dir, &graph)?;
        Ok(Json::noted(shown(&graph, id)?, warnings))
    }

    #[tool(
        description = "Record that the parent finishes before the child. The child waits until the parent is done. Returns the child. Refused when either id is missing or the edge would make a cycle.",
        annotations(
            title = "Depend on a node",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_link(
        &self,
        Parameters(args): Parameters<EdgeArgs>,
    ) -> Result<Json<NodeRow>, Fail> {
        let parent = parse(&args.parent)?;
        let child = parse(&args.child)?;
        let actor = parse(&args.actor)?;
        let (dir, _lock, mut graph) = self.writing()?;
        graph.link_dep(parent, child, actor).map_err(bad)?;
        let warnings = self.stored(&dir, &graph)?;
        Ok(Json::noted(shown(&graph, child)?, warnings))
    }

    #[tool(
        description = "Drop the dependency from parent to child. An edge that is not there is left as it is. Returns the child.",
        annotations(
            title = "Drop a dependency",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_unlink(
        &self,
        Parameters(args): Parameters<EdgeArgs>,
    ) -> Result<Json<NodeRow>, Fail> {
        let parent = parse(&args.parent)?;
        let child = parse(&args.child)?;
        let actor = parse(&args.actor)?;
        let (dir, _lock, mut graph) = self.writing()?;
        graph.unlink_dep(parent, child, actor).map_err(bad)?;
        let warnings = self.stored(&dir, &graph)?;
        Ok(Json::noted(shown(&graph, child)?, warnings))
    }
}

#[tool_handler]
#[prompt_handler(router = Self::prompt_router())]
impl ServerHandler for ClaimdagServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new("claimdag", env!("CARGO_PKG_VERSION")))
        .with_instructions(
            "Ask what is claimable before doing anything else. A claim is a lease, not \
                 a fact: claim hands back a generation, renew keeps the lease without \
                 changing it, and complete takes it so a finish is refused if the lease was \
                 reclaimed while you were away. Carry the generation between calls. This \
                 schedules work for one session and does not outlive it; what has to \
                 survive belongs in the tracker.",
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use claimdag::{WorkFields, WorkKind, WorkRole};

    fn seat() -> (tempfile::TempDir, ClaimdagServer) {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = ClaimdagServer::at(dir.path().to_path_buf());
        (dir, server)
    }

    fn work(dir: &std::path::Path, summary: &str) -> WorkId {
        let mut graph = WorkGraph::load_dir(dir);
        let id = graph
            .upsert(
                WorkId::ZERO,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Ready,
                    role: WorkRole::Implementor,
                    parent: WorkId::ZERO,
                    actor: WorkId { hi: 1, lo: 1 },
                    summary,
                },
            )
            .expect("upsert");
        graph.save_dir(dir).expect("save");
        id
    }

    /// Every tool says what it does, and each one that writes says whether it
    /// is destructive and whether repeating it changes anything more.
    #[test]
    fn every_tool_says_what_it_does_to_the_graph() {
        let tools = ClaimdagServer::tool_router().list_all();
        assert!(tools.len() >= 6, "{} tools", tools.len());
        let mut writers = Vec::new();
        for tool in &tools {
            let hints = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("{} carries no annotations", tool.name));
            assert_eq!(hints.open_world_hint, Some(false), "{}", tool.name);
            match hints.read_only_hint {
                Some(true) => {}
                Some(false) => {
                    writers.push(tool.name.to_string());
                    assert!(hints.destructive_hint.is_some(), "{}", tool.name);
                    assert!(hints.idempotent_hint.is_some(), "{}", tool.name);
                }
                None => panic!("{} does not say whether it writes", tool.name),
            }
        }
        writers.sort();
        assert_eq!(
            writers,
            vec![
                "claimdag_claim".to_string(),
                "claimdag_claim_next".to_string(),
                "claimdag_complete".to_string(),
                "claimdag_link".to_string(),
                "claimdag_reclaim".to_string(),
                "claimdag_release".to_string(),
                "claimdag_renew".to_string(),
                "claimdag_reopen".to_string(),
                "claimdag_unlink".to_string(),
                "claimdag_upsert".to_string(),
            ]
        );
    }

    /// A torn snapshot refuses a writing tool and stays on disk as it was.
    #[tokio::test]
    async fn a_writing_tool_does_not_save_over_a_torn_snapshot() {
        let (dir, server) = seat();
        let node = work(dir.path(), "the work");
        work(dir.path(), "other work");
        let snapshot = dir.path().join(claimdag::SNAP_BIN);
        let whole = std::fs::read(&snapshot).unwrap();
        let torn = whole[..whole.len() / 2].to_vec();
        std::fs::write(&snapshot, &torn).unwrap();
        let Err(err) = server
            .claimdag_claim(Parameters(ClaimArgs {
                id: node.to_hex(),
                assignee: WorkId { hi: 7, lo: 7 }.to_hex(),
                generation: None,
            }))
            .await
        else {
            panic!("claimed on a torn snapshot");
        };
        assert!(format!("{err:?}").contains("could not be read"), "{err:?}");
        assert_eq!(std::fs::read(&snapshot).unwrap(), torn);
    }

    /// An agent's whole loop: find work, take it, keep the token, finish with
    /// it.
    #[tokio::test]
    async fn an_agent_can_take_work_and_finish_it() {
        let (dir, server) = seat();
        let node = work(dir.path(), "the work");
        let me = WorkId { hi: 7, lo: 7 }.to_hex();

        let ready = server.claimdag_ready().await.expect("ready");
        assert_eq!(ready.0.len(), 1, "{:?}", ready.0);
        assert_eq!(ready.0[0].id, node.to_hex());
        assert!(ready.0[0].assignee.is_none());

        let taken = server
            .claimdag_claim(Parameters(ClaimArgs {
                id: node.to_hex(),
                assignee: me.clone(),
                generation: None,
            }))
            .await
            .expect("claims");

        // Taken work is not claimable, which is the point of asking.
        assert!(server.claimdag_ready().await.expect("ready").0.is_empty());

        let done = server
            .claimdag_complete(Parameters(CompleteArgs {
                id: node.to_hex(),
                actor: me,
                status: None,
                summary: Some("did it".into()),
                generation: Some(taken.0.generation),
            }))
            .await
            .expect("finishes");
        assert_eq!(done.0.status, "done");
        assert_eq!(done.0.summary, "did it");
    }

    /// The token is the point: a holder whose lease was reclaimed cannot
    /// finish work somebody else now holds.
    #[tokio::test]
    async fn a_reclaimed_holder_is_refused_with_the_token_it_kept() {
        let (dir, server) = seat();
        let node = work(dir.path(), "the work");
        let first = WorkId { hi: 7, lo: 7 }.to_hex();

        let taken = server
            .claimdag_claim(Parameters(ClaimArgs {
                id: node.to_hex(),
                assignee: first.clone(),
                generation: None,
            }))
            .await
            .expect("claims");

        // A zero lease is every claim past it, which is how this is tested
        // without waiting.
        let handed = server
            .claimdag_reclaim(Parameters(ReclaimArgs { lease_seconds: 0 }))
            .await
            .expect("reclaims");
        assert_eq!(handed.0, vec![node.to_hex()]);

        let stale = server
            .claimdag_complete(Parameters(CompleteArgs {
                id: node.to_hex(),
                actor: first,
                status: None,
                summary: None,
                generation: Some(taken.0.generation),
            }))
            .await;
        assert!(stale.is_err(), "a fenced holder finished the work");

        // And it is claimable again, which is what the reclaim was for.
        assert_eq!(server.claimdag_ready().await.expect("ready").0.len(), 1);
    }

    /// No graph is not an empty graph, and a reader is told which it met.
    #[tokio::test]
    async fn an_absent_graph_is_refused_rather_than_reported_as_quiet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = ClaimdagServer::at(dir.path().join("never-existed"));
        // Matched rather than unwrapped: the ok side is a `Json`, which has no
        // Debug, and the point is the message anyway.
        let Err(err) = server.claimdag_ready().await else {
            panic!("an absent graph read as quiet");
        };
        assert!(
            format!("{err:?}").contains("nothing has been claimed"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn claimdag_claim_next_and_ready_balanced_work() {
        let (dir, server) = seat();
        let node1 = work(dir.path(), "explore work");
        {
            let mut g = WorkGraph::load_dir(dir.path());
            g.upsert(
                node1,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Ready,
                    role: WorkRole::Explore,
                    parent: WorkId::ZERO,
                    actor: WorkId { hi: 1, lo: 1 },
                    summary: "explore work",
                },
            )
            .unwrap();
            g.save_dir(dir.path()).unwrap();
        }
        let node2 = work(dir.path(), "implementor work");

        let me_imp = WorkId { hi: 42, lo: 42 }.to_hex();
        let ready = server
            .claimdag_ready_balanced(Parameters(ReadyArgs {
                role: Some(crate::args::Role::Implementor),
                assignee: Some(me_imp.clone()),
                balanced: Some(true),
                slack: Some(1),
            }))
            .await
            .expect("ready balanced");
        assert_eq!(ready.0[0].id, node2.to_hex());

        let claimed = server
            .claimdag_claim_next(Parameters(ClaimNextArgs {
                assignee: me_imp,
                role: Some(crate::args::Role::Implementor),
                slack: Some(1),
            }))
            .await
            .expect("claim next");
        assert_eq!(claimed.0.id, node2.to_hex());
        assert_eq!(claimed.0.generation, 2);
    }

    /// Calls on one arm in one round.
    ///
    /// Odd, so the median of a round is one of the calls rather than the
    /// mean of two. This is the sample the `dir` comment reports.
    const CALLS_AN_ARM: usize = 21;

    /// Rounds, alternating which arm goes first.
    ///
    /// One round can be slowed by something else on the machine. Six puts
    /// that round among the 126 calls of an arm, so it cannot by itself be
    /// the median. This is the sample the `dir` comment reports.
    const ROUNDS: usize = 6;

    /// The per-call times behind the `dir` comment.
    ///
    /// Parks `/tmp/claimdag-UID`, so it is not part of the suite. Run it on
    /// its own, in release: `cargo +1.97.1 test --release -p claimdag-mcp
    /// --bin claimdag-mcp ready_locate_timings -- --ignored --nocapture`.
    #[test]
    #[ignore = "writes measurements/j78q/ready.tsv and parks /tmp/claimdag-UID"]
    fn ready_locate_timings() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(ready_locate_timings_run());
    }

    async fn ready_locate_timings_run() {
        let parked = park_uid_dir();
        let root = tempfile::tempdir().expect("private home");
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        let _env = seat_env(&home);
        let uid = parked.path.clone();
        let _seed = work(&uid, "ready timing");

        std::env::set_var("CLAIMDAG_DIR", &uid);
        let cached = ClaimdagServer::from_env();
        std::env::remove_var("CLAIMDAG_DIR");
        let resolving = ClaimdagServer::from_env();

        let mut rows: Vec<(usize, &'static str, usize, u128)> = Vec::new();
        for round in 1..=ROUNDS {
            // Odd rounds start on the cached arm, even rounds on resolving,
            // and the two arms alternate within the round.
            let cached_first = round % 2 == 1;
            for call in 1..=CALLS_AN_ARM {
                let arms = if cached_first {
                    ["cached", "resolving"]
                } else {
                    ["resolving", "cached"]
                };
                for arm in arms {
                    let server = if arm == "cached" { &cached } else { &resolving };
                    let started = std::time::Instant::now();
                    server.claimdag_ready().await.expect("ready");
                    rows.push((round, arm, call, started.elapsed().as_micros()));
                }
            }
        }

        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../measurements/j78q/ready.tsv");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("measurements dir");
        let mut out = String::new();
        out.push_str(&format!("# host: {}\n", shell(&["hostname"])));
        out.push_str(&format!("# cpu: {}\n", cpu_line()));
        out.push_str(&format!(
            "# commit: {}\n",
            shell(&["git", "-C", env!("CARGO_MANIFEST_DIR"), "rev-parse", "HEAD"])
        ));
        out.push_str(
            "# command: cargo +1.97.1 test --release -p claimdag-mcp --bin claimdag-mcp ready_locate_timings -- --ignored --nocapture\n",
        );
        out.push_str(&format!("# date: {}\n", shell(&["date", "-Iseconds"])));
        out.push_str(
            "# round order: odd rounds start with cached and alternate each call; even rounds start with resolving. 21 calls an arm, 6 rounds, 126 calls an arm.\n",
        );
        out.push_str("# column us: microseconds for one claimdag_ready call, release build\n");
        out.push_str(
            "# quartile: an even count splits in half, and each hinge is the median of that half. 126 splits into two halves of 63.\n",
        );
        out.push_str("round\tarm\tcall\tus\n");
        for (round, arm, call, us) in &rows {
            out.push_str(&format!("{round}\t{arm}\t{call}\t{us}\n"));
        }
        std::fs::write(&path, out).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));

        for arm in ["cached", "resolving"] {
            let mut samples: Vec<u128> = rows
                .iter()
                .filter(|row| row.1 == arm)
                .map(|row| row.3)
                .collect();
            let (median, iqr) = median_and_iqr(&mut samples);
            eprintln!(
                "{arm} median {median} µs  IQR {iqr} µs  n {}",
                samples.len()
            );
        }
        eprintln!("wrote {}", path.display());
    }

    /// Move `/tmp/claimdag-UID` aside for the run and put it back after.
    fn park_uid_dir() -> ParkedUid {
        let path = PathBuf::from(format!("/tmp/claimdag-{}", uid_number()));
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
        ParkedUid { path, backup }
    }

    fn uid_number() -> u32 {
        let out = std::process::Command::new("id")
            .arg("-u")
            .output()
            .expect("id -u");
        String::from_utf8(out.stdout)
            .expect("uid")
            .trim()
            .parse()
            .expect("uid number")
    }

    struct ParkedUid {
        path: PathBuf,
        backup: Option<PathBuf>,
    }

    impl Drop for ParkedUid {
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

    struct SeatEnv {
        keys: [&'static str; 4],
        saved: Vec<Option<std::ffi::OsString>>,
    }

    impl SeatEnv {
        fn keys() -> [&'static str; 4] {
            ["HOME", "XDG_STATE_HOME", "XDG_RUNTIME_DIR", "CLAIMDAG_DIR"]
        }
    }

    fn seat_env(home: &std::path::Path) -> SeatEnv {
        let keys = SeatEnv::keys();
        let saved = keys.iter().map(std::env::var_os).collect();
        std::env::set_var("HOME", home);
        std::env::remove_var("XDG_STATE_HOME");
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var("CLAIMDAG_DIR");
        SeatEnv { keys, saved }
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

    fn shell(args: &[&str]) -> String {
        let output = std::process::Command::new(args[0])
            .args(&args[1..])
            .output()
            .unwrap_or_else(|err| panic!("{}: {err}", args[0]));
        String::from_utf8(output.stdout)
            .unwrap_or_else(|err| panic!("{}: {err}", args[0]))
            .trim()
            .to_string()
    }

    /// One line: family, model, stepping, cores and threads. CPU only.
    fn cpu_line() -> String {
        let text = std::fs::read_to_string("/proc/cpuinfo").expect("cpuinfo");
        let take = |key: &str| {
            text.lines()
                .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    (k.trim() == key).then(|| v.trim().to_string())
                })
                .unwrap_or_default()
        };
        let threads = text
            .lines()
            .filter(|line| line.starts_with("processor"))
            .count();
        let cores: usize = take("cpu cores").parse().unwrap_or(0);
        // One hardware thread a core is the machine this was read on; a
        // different count is named instead of assumed.
        let threads_per = if threads == cores {
            "1 thread per core".to_string()
        } else {
            format!("{threads} threads")
        };
        format!(
            "Intel Xeon (family {}, model {}, stepping {}), {cores} cores, {threads_per}",
            take("cpu family"),
            take("model"),
            take("stepping")
        )
    }

    /// Median, and the interquartile range as the gap between the medians of
    /// the lower and upper halves.
    ///
    /// An even count splits in half. 126 calls give two halves of 63, and 63
    /// is odd, so each hinge is one of the calls rather than a mean.
    fn median_and_iqr(samples: &mut [u128]) -> (f64, f64) {
        samples.sort_unstable();
        let mid = samples.len() / 2;
        let med = hinge(samples);
        let q1 = hinge(&samples[..mid]);
        let q3 = hinge(&samples[mid..]);
        (med, q3 - q1)
    }

    fn hinge(sorted: &[u128]) -> f64 {
        let n = sorted.len();
        if n.is_multiple_of(2) {
            (sorted[n / 2 - 1] + sorted[n / 2]) as f64 / 2.0
        } else {
            sorted[n / 2] as f64
        }
    }
}
