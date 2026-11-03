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

/// POSIX `_POSIX_HOST_NAME_MAX` is 255. `gethostname` needs one more byte
/// for the trailing NUL. Linux's own limit is 64, which fits inside this.
const HOST_NAME_CAP: usize = 256;

/// This machine's name, or `localhost` when the kernel will not say.
///
/// `gethostname` is the call Linux and macOS both answer. A read of
/// `/proc/sys/kernel/hostname` comes back empty on macOS, and the pane
/// would then claim as `user@localhost` on every Mac.
fn hostname() -> String {
    let mut buf = [0u8; HOST_NAME_CAP];
    // SAFETY: the buffer is HOST_NAME_CAP bytes and gethostname writes at
    // most that many. It does not keep the pointer.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return "localhost".to_string();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..end]).trim().to_string();
    if name.is_empty() {
        "localhost".to_string()
    } else {
        name
    }
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

    /// The name is this machine's. On macOS there is no `/proc`, and
    /// `localhost` would make two Macs the same seat.
    #[test]
    fn the_seat_takes_this_machines_name() {
        let out = std::process::Command::new("hostname")
            .output()
            .expect("hostname");
        assert!(
            out.status.success(),
            "hostname: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let expected = String::from_utf8(out.stdout).expect("hostname is utf-8");
        let expected = expected.trim();
        assert!(!expected.is_empty());
        assert_eq!(hostname(), expected);
    }

    #[test]
    fn a_derived_actor_is_never_the_escape_hatch() {
        // Zero actor bypasses the assignee guard in the graph, so a pane must
        // never claim as zero by accident.
        assert!(!derive("someone", "somewhere").is_zero());
    }
}
