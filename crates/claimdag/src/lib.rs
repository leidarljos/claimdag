//! claimdag: CAS claim/complete on a DAG.
//!
//! The host process is the sole mutator. Snapshot is unpacked Cap'n
//! `work.bin` (mmap).

mod claimdag_capnp;
mod graph;
mod id;
mod label;
mod seat;
mod snap;

pub use graph::DEFAULT_LEASE_SECS;
pub use graph::{
    role_affinity_tier, Absent, WorkFields, WorkGraph, WorkKind, WorkLedgerEntry, WorkNode,
    WorkRole, WorkStatus, SNAP_FILE,
};
pub use id::{mint_work_id, WorkId};
pub use label::{append as append_warn, emit as emit_warn, line as warn_line, FILE as WARN_LOG};
pub use seat::{locate, resolve_dir, Located};
pub use snap::SNAP_BIN;

pub mod lock;
pub use lock::lock_dir;
