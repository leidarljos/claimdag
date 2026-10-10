//! Unpacked Cap'n snapshot. Hosts mmap `work.bin`.

use std::fs::File;
use std::path::Path;

use capnp::message::{Builder, HeapAllocator, ReaderOptions};
use capnp::serialize;
use memmap2::Mmap;

use crate::claimdag_capnp;
use crate::graph::{WorkKind, WorkNode, WorkRole, WorkStatus, FORMAT_V1};
use crate::id::WorkId;

pub const SNAP_BIN: &str = "work.bin";

pub fn write_bin(
    dir: &Path,
    next_seq: u64,
    mint_seq: u64,
    nodes: &[&WorkNode],
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(SNAP_BIN);
    let tmp = dir.join("work.bin.tmp");
    let mut message = Builder::new(HeapAllocator::new());
    {
        let mut snap = message.init_root::<claimdag_capnp::snap::Builder>();
        snap.set_format(FORMAT_V1);
        snap.set_next_seq(next_seq);
        snap.set_mint_seq(mint_seq);
        let mut list = snap.init_nodes(nodes.len() as u32);
        for (i, n) in nodes.iter().enumerate() {
            fill_node(list.reborrow().get(i as u32), n);
        }
    }
    let mut f = File::create(&tmp).map_err(|e| e.to_string())?;
    serialize::write_message(&mut f, &message).map_err(|e| e.to_string())?;
    // The bytes reach the disk before the name moves, and the directory
    // entry after, so a crash leaves the old snapshot or the new one and
    // never a renamed file that is still empty.
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn read_bin(dir: &Path) -> Result<(u64, u64, Vec<WorkNode>), String> {
    let path = dir.join(SNAP_BIN);
    let file = File::open(&path).map_err(|e| e.to_string())?;
    let map = unsafe { Mmap::map(&file).map_err(|e| e.to_string())? };
    let mut slice: &[u8] = &map;
    let reader = serialize::read_message_from_flat_slice(&mut slice, ReaderOptions::new())
        .map_err(|e| e.to_string())?;
    let snap = reader
        .get_root::<claimdag_capnp::snap::Reader>()
        .map_err(|e| e.to_string())?;
    let next_seq = snap.get_next_seq();
    let mint_seq = snap.get_mint_seq();
    let list = snap.get_nodes().map_err(|e| e.to_string())?;
    let mut nodes = Vec::with_capacity(list.len() as usize);
    for n in list.iter() {
        nodes.push(node_from_reader(n)?);
    }
    Ok((next_seq, mint_seq, nodes))
}

fn fill_id(mut b: claimdag_capnp::work_id::Builder<'_>, id: WorkId) {
    b.set_hi(id.hi);
    b.set_lo(id.lo);
}

fn fill_node(mut b: claimdag_capnp::node::Builder<'_>, n: &WorkNode) {
    fill_id(b.reborrow().init_id(), n.id);
    b.set_kind(kind_to_cap(n.kind));
    b.set_status(status_to_cap(n.status));
    b.set_role(role_to_cap(n.role));
    fill_id(b.reborrow().init_assignee(), n.assignee);
    fill_id(b.reborrow().init_parent(), n.parent);
    {
        let mut deps = b.reborrow().init_deps(n.deps.len() as u32);
        for (i, d) in n.deps.iter().enumerate() {
            fill_id(deps.reborrow().get(i as u32), *d);
        }
    }
    b.set_cas_gen(n.cas_gen);
    b.set_created_unix(n.created_unix);
    b.set_updated_unix(n.updated_unix);
    b.set_finished_unix(n.finished_unix);
    b.set_summary(n.summary.as_str());
    b.set_archived(n.archived);
}

fn id_from_reader(r: claimdag_capnp::work_id::Reader<'_>) -> WorkId {
    WorkId {
        hi: r.get_hi(),
        lo: r.get_lo(),
    }
}

fn node_from_reader(r: claimdag_capnp::node::Reader<'_>) -> Result<WorkNode, String> {
    let id = id_from_reader(r.get_id().map_err(|e| e.to_string())?);
    let assignee = id_from_reader(r.get_assignee().map_err(|e| e.to_string())?);
    let parent = id_from_reader(r.get_parent().map_err(|e| e.to_string())?);
    let deps_r = r.get_deps().map_err(|e| e.to_string())?;
    let mut deps = Vec::with_capacity(deps_r.len() as usize);
    for d in deps_r.iter() {
        deps.push(id_from_reader(d));
    }
    let summary = r
        .get_summary()
        .map_err(|e| e.to_string())?
        .to_string()
        .map_err(|e| e.to_string())?;
    Ok(WorkNode {
        id,
        kind: kind_from_cap(r.get_kind().map_err(|e| e.to_string())?),
        status: status_from_cap(r.get_status().map_err(|e| e.to_string())?),
        role: role_from_cap(r.get_role().map_err(|e| e.to_string())?),
        assignee,
        parent,
        deps,
        cas_gen: r.get_cas_gen(),
        created_unix: r.get_created_unix(),
        updated_unix: r.get_updated_unix(),
        finished_unix: r.get_finished_unix(),
        summary,
        archived: r.get_archived(),
    })
}

fn kind_to_cap(k: WorkKind) -> claimdag_capnp::Kind {
    match k {
        WorkKind::Unset => claimdag_capnp::Kind::Unset,
        WorkKind::Goal => claimdag_capnp::Kind::Goal,
        WorkKind::Step => claimdag_capnp::Kind::Step,
        WorkKind::Task => claimdag_capnp::Kind::Task,
        WorkKind::Molecule => claimdag_capnp::Kind::Molecule,
    }
}

fn kind_from_cap(k: claimdag_capnp::Kind) -> WorkKind {
    match k {
        claimdag_capnp::Kind::Unset => WorkKind::Unset,
        claimdag_capnp::Kind::Goal => WorkKind::Goal,
        claimdag_capnp::Kind::Step => WorkKind::Step,
        claimdag_capnp::Kind::Task => WorkKind::Task,
        claimdag_capnp::Kind::Molecule => WorkKind::Molecule,
    }
}

fn status_to_cap(s: WorkStatus) -> claimdag_capnp::Status {
    match s {
        WorkStatus::Todo => claimdag_capnp::Status::Todo,
        WorkStatus::Ready => claimdag_capnp::Status::Ready,
        WorkStatus::Claimed => claimdag_capnp::Status::Claimed,
        WorkStatus::Running => claimdag_capnp::Status::Running,
        WorkStatus::Blocked => claimdag_capnp::Status::Blocked,
        WorkStatus::Done => claimdag_capnp::Status::Done,
        WorkStatus::Failed => claimdag_capnp::Status::Failed,
        WorkStatus::Cancelled => claimdag_capnp::Status::Cancelled,
    }
}

fn status_from_cap(s: claimdag_capnp::Status) -> WorkStatus {
    match s {
        claimdag_capnp::Status::Todo => WorkStatus::Todo,
        claimdag_capnp::Status::Ready => WorkStatus::Ready,
        claimdag_capnp::Status::Claimed => WorkStatus::Claimed,
        claimdag_capnp::Status::Running => WorkStatus::Running,
        claimdag_capnp::Status::Blocked => WorkStatus::Blocked,
        claimdag_capnp::Status::Done => WorkStatus::Done,
        claimdag_capnp::Status::Failed => WorkStatus::Failed,
        claimdag_capnp::Status::Cancelled => WorkStatus::Cancelled,
    }
}

fn role_to_cap(r: WorkRole) -> claimdag_capnp::Role {
    match r {
        WorkRole::Unset => claimdag_capnp::Role::Unset,
        WorkRole::Explore => claimdag_capnp::Role::Explore,
        WorkRole::Architect => claimdag_capnp::Role::Architect,
        WorkRole::Implementor => claimdag_capnp::Role::Implementor,
        WorkRole::Verifier => claimdag_capnp::Role::Verifier,
        WorkRole::Orchestrator => claimdag_capnp::Role::Orchestrator,
        WorkRole::General => claimdag_capnp::Role::General,
    }
}

fn role_from_cap(r: claimdag_capnp::Role) -> WorkRole {
    match r {
        claimdag_capnp::Role::Unset => WorkRole::Unset,
        claimdag_capnp::Role::Explore => WorkRole::Explore,
        claimdag_capnp::Role::Architect => WorkRole::Architect,
        claimdag_capnp::Role::Implementor => WorkRole::Implementor,
        claimdag_capnp::Role::Verifier => WorkRole::Verifier,
        claimdag_capnp::Role::Orchestrator => WorkRole::Orchestrator,
        claimdag_capnp::Role::General => WorkRole::General,
    }
}
