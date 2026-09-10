//! What each tool takes.
//!
//! Ids are 32 hex characters, the way every other surface spells them. The
//! generation appears wherever a caller holds one, and is optional in the same
//! places the command line makes it optional: omitting it is the escape for
//! speaking about a node nobody holds.

use schemars::JsonSchema;
use serde::Deserialize;

/// Whether to include finished and archived work.
#[derive(Deserialize, JsonSchema)]
pub struct ListArgs {
    /// Include terminal and archived nodes. Live work only by default.
    #[serde(default)]
    pub all: Option<bool>,
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
