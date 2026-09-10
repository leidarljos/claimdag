//! Working a lease, as a sequence rather than four unrelated verbs.
//!
//! A prompt carries what a tool description cannot: an order, and the reason
//! for it. The order here is set by the one thing this store is about. A claim
//! is not a fact, it is a statement that expires, so every step after the
//! claim carries the token the claim handed back, and an agent that drops it
//! between calls is one that will finish work it no longer holds.
//!
//! What is scheduled here does not outlive the session. Nothing in these
//! prompts tells an agent what the work means or what it produced; those are
//! the tracker's and the deed store's, and they are deliberately outside this
//! join.

use rmcp::{
    handler::server::wrapper::Parameters, model::*, prompt, prompt_router, ErrorData as McpError,
};
use schemars::JsonSchema;
use serde::Deserialize;

use claimdag::DEFAULT_LEASE_SECS as DEFAULT_LEASE;

use crate::server::ClaimdagServer;

/// Who is picking work up, and for how long.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TakeArgs {
    /// The actor to claim as. Defaults to the seat's own identity.
    pub actor: Option<String>,
    /// How long a claim should stand before it can be taken back, in seconds.
    pub lease_seconds: Option<u64>,
}

/// How quiet a held node has to be before it is worth taking back.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SweepArgs {
    /// Seconds of silence after which a claim is treated as abandoned.
    pub lease_seconds: Option<u64>,
}

fn asked(text: String) -> Vec<PromptMessage> {
    vec![PromptMessage::new_text(Role::User, text)]
}


#[prompt_router(vis = pub(crate))]
impl ClaimdagServer {
    /// Take the next claimable node and work it under a lease, carrying the
    /// generation from the claim through to the finish.
    #[prompt(name = "take_the_next_node")]
    pub async fn take_the_next_node_prompt(
        &self,
        Parameters(args): Parameters<TakeArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let actor = args.actor.filter(|a| !a.trim().is_empty()).map_or_else(
            || "the seat's own identity".to_string(),
            |a| format!("actor {a}"),
        );
        let lease = args.lease_seconds.unwrap_or(DEFAULT_LEASE);
        Ok(asked(format!(
            "Take the next claimable node as {actor}, on a {lease} second lease.\n\
             \n\
             `claimdag_ready` first. Ready is unblocked and unheld, which is not the\n\
             same as open: a node somebody else holds is not yours to start, and a\n\
             node whose inputs are unfinished is work that will be redone.\n\
             \n\
             `claimdag_claim` hands back a generation. That number is the token for\n\
             everything after it. Carry it. An agent that drops it between calls is an\n\
             agent that will finish work it no longer holds.\n\
             \n\
             While working, `claimdag_renew` says the holder is alive. A renewal does\n\
             not change the generation, because it is not a change of ownership.\n\
             \n\
             `claimdag_complete` takes the generation back. If the lease was reclaimed\n\
             while you were away, the finish is refused rather than overwriting\n\
             somebody else's claim, and that refusal is information: the work was\n\
             taken up elsewhere and what you have may be a duplicate.\n\
             \n\
             This graph is for one session. What has to survive it belongs in the\n\
             tracker, cited to what it produced."
        )))
    }

    /// Take back the claims that went quiet, and say what was taken and from
    /// whom.
    #[prompt(name = "sweep_stale_claims")]
    pub async fn sweep_stale_claims_prompt(
        &self,
        Parameters(args): Parameters<SweepArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let lease = args.lease_seconds.unwrap_or(DEFAULT_LEASE);
        Ok(asked(format!(
            "Take back the claims that have been quiet longer than {lease} seconds.\n\
             \n\
             `claimdag_list` shows who holds what and how long each held node has\n\
             been quiet. Read that before reclaiming: quiet is not the same as\n\
             abandoned, and a node held by somebody still renewing is not stale no\n\
             matter how long the work is taking.\n\
             \n\
             `claimdag_reclaim` with the lease takes back what is past it. It moves\n\
             the generation, which is what makes the old holder's finish fail rather\n\
             than land silently on work somebody else now has.\n\
             \n\
             Report what was reclaimed and who held it. A reclaim is visible to the\n\
             seat that lost it only as a refused completion, so saying so here is the\n\
             only warning anyone gets."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every declared prompt renders, from the arguments it says it takes.
    #[tokio::test]
    async fn every_prompt_renders_from_what_it_declares() {
        let declared = ClaimdagServer::prompt_router().list_all();
        let names: Vec<&str> = declared.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["take_the_next_node", "sweep_stale_claims"]);
        for prompt in &declared {
            assert!(
                prompt.description.as_ref().is_some_and(|d| !d.is_empty()),
                "{} carries no description",
                prompt.name
            );
            assert!(
                prompt.arguments.as_ref().is_some_and(|a| !a.is_empty()),
                "{} declares no arguments",
                prompt.name
            );
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let server = ClaimdagServer::at(dir.path().to_path_buf());

        let took = server
            .take_the_next_node_prompt(Parameters(TakeArgs {
                actor: Some("reader".into()),
                lease_seconds: Some(60),
            }))
            .await
            .expect("renders");
        let said = format!("{:?}", took[0].content);
        assert!(said.contains("actor reader"), "{said}");
        assert!(said.contains("60 second lease"), "{said}");
        // The property the whole surface is shaped by survives into the text.
        assert!(said.contains("generation"), "{said}");

        let default = server
            .take_the_next_node_prompt(Parameters(TakeArgs {
                actor: None,
                lease_seconds: None,
            }))
            .await
            .expect("renders");
        let said = format!("{:?}", default[0].content);
        assert!(said.contains("the seat's own identity"), "{said}");
        assert!(said.contains(&format!("{DEFAULT_LEASE} second lease")), "{said}");

        let swept = server
            .sweep_stale_claims_prompt(Parameters(SweepArgs {
                lease_seconds: None,
            }))
            .await
            .expect("renders");
        let said = format!("{:?}", swept[0].content);
        assert!(said.contains(&format!("{DEFAULT_LEASE} seconds")), "{said}");
        assert!(said.contains("quiet is not the same as"), "{said}");
    }
}
