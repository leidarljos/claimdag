//! `claimdag-mcp`: the claim graph over the Model Context Protocol.
//!
//! One graph per seat, in the directory `CLAIMDAG_DIR` or the runtime
//! directory names, the same as the command line and the pane. Stdio, because
//! a seat runs this beside the agent rather than as a service.

mod args;
mod server;

use rmcp::{transport::stdio, ServiceExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let running = server::ClaimdagServer::from_env().serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
