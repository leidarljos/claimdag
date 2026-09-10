//! claimdag: CAS claim/complete on a DAG.
//!
//! The host process is the sole mutator. Snapshot is unpacked Cap'n
//! `work.bin` (mmap).

mod claimdag_capnp;
mod graph;
mod id;
mod seat;
mod snap;

pub use graph::DEFAULT_LEASE_SECS;
pub use graph::{
    Absent, WorkFields, WorkGraph, WorkKind, WorkLedgerEntry, WorkNode, WorkRole, WorkStatus,
    SNAP_FILE,
};
pub use id::{mint_work_id, WorkId};
pub use seat::resolve_dir;
pub use snap::SNAP_BIN;
