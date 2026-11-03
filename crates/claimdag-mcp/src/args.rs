//! What each tool takes.
//!
//! Ids are 32 hex characters, the way every other surface spells them. The
//! generation appears wherever a caller holds one, and is optional in the same
//! places the command line makes it optional: omitting it is the escape for
//! speaking about a node nobody holds.

use claimdag::WorkRole;
use schemars::JsonSchema;
use serde::Deserialize;

/// A worker role, lowercase, the same words the graph stores.
///
/// The tool schema lists these. A word outside the list is refused.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Unset,
    Explore,
    Architect,
    Implementor,
    Verifier,
    Orchestrator,
    General,
}

impl From<Role> for WorkRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Unset => Self::Unset,
            Role::Explore => Self::Explore,
            Role::Architect => Self::Architect,
            Role::Implementor => Self::Implementor,
            Role::Verifier => Self::Verifier,
            Role::Orchestrator => Self::Orchestrator,
            Role::General => Self::General,
        }
    }
}

/// Whether to include finished and archived work.
#[derive(Deserialize, JsonSchema)]
pub struct ListArgs {
    /// Include terminal and archived nodes. Live work only by default.
    #[serde(default)]
    pub all: Option<bool>,
}

/// Querying claimable ready work.
#[derive(Deserialize, JsonSchema, Default)]
pub struct ReadyArgs {
    /// Filter or prioritize by worker role ('unset', 'explore', 'architect', 'implementor', 'verifier', 'orchestrator', 'general').
    #[serde(default)]
    pub role: Option<Role>,
    /// Who is asking, for worker-dispersed load balancing (32 hex characters).
    #[serde(default)]
    pub assignee: Option<String>,
    /// Order by balanced suitability and worker dispersion rather than static depth.
    #[serde(default)]
    pub balanced: Option<bool>,
    /// Allowed critical depth slack for candidate dispersion (default 1).
    #[serde(default)]
    pub slack: Option<usize>,
}

/// Atomically taking the next best ready node with load-balanced selection.
#[derive(Deserialize, JsonSchema)]
pub struct ClaimNextArgs {
    /// Who is taking it, 32 hex characters.
    pub assignee: String,
    /// Desired role affinity ('unset', 'explore', 'architect', 'implementor', 'verifier', 'orchestrator', 'general').
    #[serde(default)]
    pub role: Option<Role>,
    /// Allowed critical depth slack for candidate dispersion (default 1).
    #[serde(default)]
    pub slack: Option<usize>,
}

/// Taking a node.
#[derive(Deserialize, JsonSchema)]
pub struct ClaimArgs {
    /// The node, 32 hex characters.
    pub id: String,
    /// Who is taking it, 32 hex characters.
    pub assignee: String,
    /// Refuse unless the node is still at this generation.
    #[serde(default)]
    pub generation: Option<u64>,
}

/// Speaking for a node you hold.
#[derive(Deserialize, JsonSchema)]
pub struct ActorArgs {
    /// The node, 32 hex characters.
    pub id: String,
    /// Who is speaking, 32 hex characters.
    pub actor: String,
}

/// Finishing a node.
#[derive(Deserialize, JsonSchema)]
pub struct CompleteArgs {
    /// The node, 32 hex characters.
    pub id: String,
    /// Who is finishing it, 32 hex characters.
    pub actor: String,
    /// `done`, `failed` or `cancelled`. Done by default.
    #[serde(default)]
    pub status: Option<String>,
    /// What happened, recorded on the node.
    #[serde(default)]
    pub summary: Option<String>,
    /// The generation `claim` returned. Without it the finish goes through
    /// even when the lease was reclaimed and somebody else holds the work.
    #[serde(default)]
    pub generation: Option<u64>,
}

/// How long a claim may go quiet before it is handed back.
#[derive(Deserialize, JsonSchema)]
pub struct ReclaimArgs {
    /// Seconds of silence a claim is allowed.
    pub lease_seconds: u64,
}

/// One node to read.
#[derive(Deserialize, JsonSchema)]
pub struct GetArgs {
    /// The node, 32 hex characters.
    pub id: String,
}

/// Create a node, or change one that is already there.
#[derive(Deserialize, JsonSchema)]
pub struct UpsertArgs {
    /// The node, 32 hex characters. Omit it to mint one.
    #[serde(default)]
    pub id: Option<String>,
    /// What the work is. Omit it and an existing summary stays.
    #[serde(default)]
    pub summary: Option<String>,
    /// `goal`, `step`, `task`, `molecule` or `unset`. Omit it and a stored
    /// kind stays; a new node is a task.
    #[serde(default)]
    pub kind: Option<String>,
    /// `explore`, `architect`, `implementor`, `verifier`, `orchestrator`,
    /// `general` or `unset`. Omit it and a stored role stays.
    #[serde(default)]
    pub role: Option<String>,
    /// The node this one hangs under, 32 hex characters. Omit it to leave the
    /// parent as it is.
    #[serde(default)]
    pub parent: Option<String>,
    /// `todo`, `ready` or `blocked`. Omit it and the node keeps its status.
    #[serde(default)]
    pub status: Option<String>,
    /// Who is asking, 32 hex characters.
    pub actor: String,
}

/// A dependency edge: the parent finishes before the child.
#[derive(Deserialize, JsonSchema)]
pub struct EdgeArgs {
    /// The node that has to finish first, 32 hex characters.
    pub parent: String,
    /// The node that waits, 32 hex characters.
    pub child: String,
    /// Who is asking, 32 hex characters.
    pub actor: String,
}
