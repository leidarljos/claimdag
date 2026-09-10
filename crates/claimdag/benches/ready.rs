//! What asking for work costs at the graph's cap.
//!
//! `ready` walks every node and, since the order became the critical path,
//! computes the depth of unfinished work below each one on every call. The cap
//! is 4096 nodes, so this is the worst case a seat can hit and the number a
//! reader should expect at the pane's refresh rate.

use std::time::Duration;

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn id(n: u64) -> WorkId {
    WorkId {
        hi: n,
        lo: n.wrapping_mul(3),
    }
}

/// A graph of `n` nodes in chains of `depth`, every third node finished, so
/// the depth walk has both finished nodes to skip and chains to follow.
fn graph(n: usize, depth: usize) -> WorkGraph {
    let mut g = WorkGraph::default();
    let actor = id(999_999);
    let mut previous: Option<WorkId> = None;
    for at in 0..n {
        let node = id(at as u64 + 1);
        g.upsert(
            node,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor,
                summary: "bench",
            },
        )
        .expect("upsert");
        if at % depth != 0 {
            if let Some(prev) = previous {
                g.link_dep(prev, node, actor).expect("link");
            }
        }
        previous = Some(node);
    }
    // Finish the heads of every third chain, so some depth is done work.
    for at in (0..n).step_by(depth * 3) {
        let node = id(at as u64 + 1);
        if g.claim(node, id(7), None).is_ok() {
            g.complete(node, WorkStatus::Done, "", id(7), None)
                .expect("complete");
        }
    }
    g
}

fn ready(c: &mut Criterion) {
    let mut group = c.benchmark_group("ready over n nodes");
    group.measurement_time(Duration::from_secs(6));
    for (n, depth) in [(256usize, 8usize), (1_024, 8), (4_096, 8), (4_096, 64)] {
        let g = graph(n, depth);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(
            BenchmarkId::new(format!("chains of {depth}"), n),
            &n,
            |b, _| b.iter(|| g.ready_view().len()),
        );
    }
    group.finish();
}

fn depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("critical depth over n nodes");
    group.measurement_time(Duration::from_secs(6));
    for n in [1_024usize, 4_096] {
        let g = graph(n, 8);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| g.critical_depth().len())
        });
    }
    group.finish();
}

criterion_group!(benches, ready, depth);
criterion_main!(benches);
