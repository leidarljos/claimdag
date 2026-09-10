//! The claim graph as an agent surface.
//!
//! An agent picking up work asks one question before it does anything else:
//! what is unblocked and unheld. That answer lived behind a shell, which meant
//! the first thing an agent needed was the thing it could not see.
//!
//! The shape of this surface is set by the lease. A claim is not a fact, it is
//! a statement that expires, so every verb here either takes the generation it
//! was given or says why it is not taking one:
//!
//! - `claim` hands back a generation, which is the token for everything after.
//! - `renew` says the holder is alive and leaves the generation alone, because
//!   a renewal is not a change of ownership.
//! - `complete` takes the token, so a holder that stalled long enough to be
//!   reclaimed is refused rather than finishing work somebody else now holds.
//!
//! An agent that drops the generation between calls is an agent that will
//! finish work it no longer holds, so the tools that return one say what it is
//! for rather than leaving it as a number in a field.
//!
//! One process holds the graph, so this is a reader and a writer over the same
//! snapshot the pane and the command line use, not a second store.

use std::path::PathBuf;

use claimdag::{Absent, WorkGraph, WorkId, WorkNode, WorkStatus};
use rmcp::{
    handler::server::wrapper::Json, handler::server::wrapper::Parameters,
    handler::server::ServerHandler, model::*, prompt_handler, tool, tool_handler, tool_router,
    ErrorData as McpError,
};
use serde::Serialize;

use crate::args::*;

#[derive(Clone)]
pub struct ClaimdagServer {
    dir: PathBuf,
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

fn row(node: &WorkNode, now: u64) -> NodeRow {
    let held = matches!(node.status, WorkStatus::Claimed | WorkStatus::Running);
    NodeRow {
        id: node.id.to_hex(),
        status: node.status.as_str().to_string(),
        summary: node.summary.clone(),
        assignee: (!node.assignee.is_zero()).then(|| node.assignee.to_hex()),
        generation: node.cas_gen,
        blocked_by: node.deps.iter().map(|d| d.to_hex()).collect(),
        quiet_seconds: held.then(|| now.saturating_sub(node.updated_unix)),
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse(raw: &str) -> Result<WorkId, McpError> {
    WorkId::from_hex(raw).ok_or_else(|| McpError::invalid_params(format!("bad id {raw}"), None))
}

fn bad(why: String) -> McpError {
    McpError::internal_error(why, None)
}

#[tool_router]
impl ClaimdagServer {
    /// Open on the graph this seat keeps.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            dir: claimdag::resolve_dir(None),
        }
    }

    /// Open on a named directory, for a test.
    #[cfg(test)]
    #[must_use]
    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// A graph to read, or the reason there is none.
    ///
    /// Reading verbs refuse an absent graph rather than answering "nothing
    /// claimable", because those are opposite answers: one is a quiet seat and
    /// the other is a caller pointed at the wrong place.
    fn reading(&self) -> Result<WorkGraph, McpError> {
        WorkGraph::open_dir(&self.dir).map_err(|absent: Absent| bad(absent.to_string()))
    }

    #[tool(
        description = "Work that can be taken right now: unblocked, unheld, and not finished. Ask this before anything else. An empty answer means the seat is quiet; a failure means there is no graph here, which is a different thing.",
        annotations(
            title = "Claimable work",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_ready(&self) -> Result<Json<Vec<NodeRow>>, McpError> {
        let graph = self.reading()?;
        let now = now_unix();
        let done: std::collections::HashSet<WorkId> = graph
            .list()
            .iter()
            .filter(|n| n.status == WorkStatus::Done)
            .map(|n| n.id)
            .collect();
        Ok(Json(
            graph
                .list_view(false, false)
                .into_iter()
                .filter(|n| matches!(n.status, WorkStatus::Ready | WorkStatus::Todo))
                .filter(|n| n.deps.iter().all(|d| done.contains(d)))
                .map(|n| row(n, now))
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
    ) -> Result<Json<Vec<NodeRow>>, McpError> {
        let graph = self.reading()?;
        let now = now_unix();
        let all = args.all.unwrap_or(false);
        Ok(Json(
            graph
                .list_view(all, all)
                .into_iter()
                .map(|n| row(n, now))
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
    ) -> Result<Json<ClaimRow>, McpError> {
        let id = parse(&args.id)?;
        let assignee = parse(&args.assignee)?;
        let mut graph = WorkGraph::load_dir(&self.dir);
        let generation = graph.claim(id, assignee, args.generation).map_err(bad)?;
        graph.save_dir(&self.dir).map_err(bad)?;
        Ok(Json(ClaimRow {
            id: id.to_hex(),
            generation,
        }))
    }

    #[tool(
        description = "Say the holder is still working, which moves the lease and leaves the generation alone. A renewal is not a change of ownership, so the token you hold stays good.",
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
    ) -> Result<Json<ClaimRow>, McpError> {
        let id = parse(&args.id)?;
        let actor = parse(&args.actor)?;
        let mut graph = WorkGraph::load_dir(&self.dir);
        let generation = graph.renew(id, actor).map_err(bad)?;
        graph.save_dir(&self.dir).map_err(bad)?;
        Ok(Json(ClaimRow {
            id: id.to_hex(),
            generation,
        }))
    }

    #[tool(
        description = "Finish a node. Pass the generation claim gave you: without it the finish goes through even if your lease was reclaimed and somebody else has the work. Terminal is sticky.",
        annotations(
            title = "Finish work",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn claimdag_complete(
        &self,
        Parameters(args): Parameters<CompleteArgs>,
    ) -> Result<Json<NodeRow>, McpError> {
        let id = parse(&args.id)?;
        let actor = parse(&args.actor)?;
        let status = match args.status.as_deref().unwrap_or("done") {
            "done" => WorkStatus::Done,
            "failed" => WorkStatus::Failed,
            "cancelled" => WorkStatus::Cancelled,
            other => {
                return Err(McpError::invalid_params(
                    format!("status is done, failed or cancelled, not {other}"),
                    None,
                ));
            }
        };
        let mut graph = WorkGraph::load_dir(&self.dir);
        graph
            .complete(
                id,
                status,
                args.summary.as_deref().unwrap_or(""),
                actor,
                args.generation,
            )
            .map_err(bad)?;
        graph.save_dir(&self.dir).map_err(bad)?;
        let now = now_unix();
        let node = graph
            .get(id)
            .ok_or_else(|| bad("complete: the node went missing".into()))?;
        Ok(Json(row(node, now)))
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
    ) -> Result<Json<Vec<String>>, McpError> {
        let mut graph = WorkGraph::load_dir(&self.dir);
        let handed = graph.reclaim(args.lease_seconds);
        if !handed.is_empty() {
            graph.save_dir(&self.dir).map_err(bad)?;
        }
        Ok(Json(handed.iter().map(|id| id.to_hex()).collect()))
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

    /// Every tool says what it does, and the three that write say whether they
    /// are destructive and whether repeating them changes anything more.
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
                "claimdag_complete".to_string(),
                "claimdag_reclaim".to_string(),
                "claimdag_renew".to_string(),
            ]
        );
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
}
