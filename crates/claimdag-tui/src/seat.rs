//! Who this pane claims as.
//!
//! Derived rather than random, so the same seat is the same actor across
//! restarts: a claim it holds is one it can still complete, and the assignee
//! guard in the graph is what makes that matter.

use claimdag::WorkId;
use sha2::{Digest, Sha256};

/// The actor id this pane writes.
///
/// `CLAIMDAG_ACTOR` when it names a 32-hex id, else the leading half of the
/// SHA-256 of `user@host`.
#[must_use]
pub fn actor() -> WorkId {
    let named = std::env::var("CLAIMDAG_ACTOR").unwrap_or_default();
    if let Some(id) = WorkId::from_hex(named.trim()) {
        return id;
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "actor".to_string());
    derive(&user, &hostname())
}

/// The rule, with its inputs passed in so it can be checked.
fn derive(user: &str, host: &str) -> WorkId {
    let digest = Sha256::digest(format!("{user}@{host}").as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    WorkId::from_hex(&hex).unwrap_or(WorkId::ZERO)
}

/// This machine's name, or `localhost` when the kernel will not say.
fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seat_derives_the_same_actor() {
        assert_eq!(
            derive("someone", "somewhere"),
            derive("someone", "somewhere")
        );
    }

    #[test]
    fn a_different_seat_derives_a_different_actor() {
        assert_ne!(
            derive("someone", "somewhere"),
            derive("someone", "elsewhere")
        );
    }

    #[test]
    fn a_derived_actor_is_never_the_escape_hatch() {
        // Zero actor bypasses the assignee guard in the graph, so a pane must
        // never claim as zero by accident.
        assert!(!derive("someone", "somewhere").is_zero());
    }
}
