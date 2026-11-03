//! What a call costs against the node count, to set the node cap.
//!
//! Every call loads `work.bin` whole and a writing call saves it whole, so
//! a call pays a fixed part and a part per node. The sweep holds live nodes,
//! which prune never drops, so it can go past the cap. `finished` is a full
//! graph in steady state: every node finished, at least a graph's worth
//! named in `pruned.log`, and each new node dropping the oldest. `compact`
//! is the same graph when `pruned.log` names twice the cap and the next
//! drop rewrites it.
//!
//! ```console
//! $ cargo build --release --workspace
//! $ cargo run --release -p claimdag --example node_cap -- target/release/claimdag [sizes] [summary bytes]
//! ```

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus, SNAP_BIN};

/// Runs per figure. Odd, so the median is one of the runs.
const REPS: usize = 21;

/// Median milliseconds of `call`, with `setup` run untimed before each.
fn median_ms(mut setup: impl FnMut(), mut call: impl FnMut()) -> f64 {
    let mut times: Vec<f64> = (0..REPS)
        .map(|_| {
            setup();
            let start = Instant::now();
            call();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[REPS / 2]
}

fn run(bin: &Path, dir: &Path, args: &[&str]) {
    let out = Command::new(bin)
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .expect("run claimdag");
    assert!(out.status.success(), "{args:?}: {out:?}");
}

fn add(graph: &mut WorkGraph, summary: &str) -> WorkId {
    graph
        .upsert(
            WorkId::ZERO,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Ready,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: WorkId::ZERO,
                summary,
            },
        )
        .expect("upsert")
}

/// One row: snapshot bytes, in-process load and save, and a read and a
/// writing call through the binary, `reset` run untimed before each write.
fn row(
    label: &str,
    graph: &WorkGraph,
    node: WorkId,
    (bin, dir): (&Path, &Path),
    write: &[&str],
    reset: impl FnMut(),
) {
    let save = median_ms(|| {}, || graph.save_dir(dir).expect("save"));
    let bytes = std::fs::metadata(dir.join(SNAP_BIN))
        .expect("work.bin")
        .len();
    let load = median_ms(
        || {},
        || {
            std::hint::black_box(WorkGraph::open_dir(dir).expect("open").list().len());
        },
    );
    let read = median_ms(|| {}, || run(bin, dir, &["get", &node.to_hex()]));
    let writing = median_ms(reset, || run(bin, dir, write));
    let nodes = graph.list().len();
    println!(
        "| {label:<8} | {nodes:>6} | {bytes:>9} | {:>6.0} | {load:>7.2} | {save:>7.2} | {read:>7.2} | {writing:>8.2} |",
        bytes as f64 / nodes as f64
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let bin = std::fs::canonicalize(args.next().expect("path to the claimdag binary"))
        .expect("the claimdag binary");
    let sizes: Vec<usize> = args
        .next()
        .map(|raw| raw.split(',').filter_map(|s| s.parse().ok()).collect())
        .unwrap_or_else(|| vec![1024, 2048, 4096, 8192, 16384, 32768, 65536]);
    let pad = "x".repeat(args.next().and_then(|s| s.parse().ok()).unwrap_or(0));
    let summary = |n: usize| format!("work item {n}{pad}");

    let dir = std::env::temp_dir().join(format!("claimdag-node-cap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    println!("| graph    |  nodes |     bytes | B/node | load ms | save ms | read ms | write ms |");
    println!("|----------+--------+-----------+--------+---------+---------+---------+----------|");

    let mut graph = WorkGraph::default();
    let mut node = WorkId::ZERO;
    let mut made = 0;
    for target in sizes {
        while made < target {
            node = add(&mut graph, &summary(made));
            made += 1;
        }
        let hex = node.to_hex();
        let write = ["upsert", "--id", &hex, "--summary", "work item"];
        row("live", &graph, node, (&bin, &dir), &write, || {});
    }

    // Finish nodes until prune drops the first, which is the cap, then as
    // many again, which fills pruned.log.
    let actor = WorkId { hi: 1, lo: 1 };
    let finish = |graph: &mut WorkGraph, n: usize| {
        let id = add(graph, &summary(n));
        graph
            .complete(id, WorkStatus::Done, "", actor, None)
            .expect("complete");
        id
    };
    let mut graph = WorkGraph::default();
    let first = finish(&mut graph, 0);
    let mut made = 1;
    while graph.get(first).is_some() {
        node = finish(&mut graph, made);
        made += 1;
    }
    for _ in 0..graph.list().len() {
        node = finish(&mut graph, made);
        made += 1;
    }
    let steady = dir.join("finished");
    graph.save_dir(&steady).expect("save");
    let graph = WorkGraph::load_dir(&steady);
    let write = ["upsert", "--summary", "work item"];
    row("finished", &graph, node, (&bin, &steady), &write, || {});

    // The same graph once pruned.log holds twice the cap, so the next drop
    // rewrites it: the longest load, and the write that compacts.
    let log = steady.join("pruned.log");
    let named = std::fs::read_to_string(&log).expect("pruned.log");
    let full: String = named
        .lines()
        .cycle()
        .take(2 * graph.list().len())
        .map(|line| format!("{line}\n"))
        .collect();
    let reset = || std::fs::write(&log, &full).expect("pruned.log");
    reset();
    row("compact", &graph, node, (&bin, &steady), &write, reset);
    std::fs::remove_dir_all(&dir).ok();
}
