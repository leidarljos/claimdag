//! What the work graph costs as it fills toward its node cap.
//!
//! The cap is why this is short: the graph cannot grow without bound, so what
//! matters is that nothing degrades on the way to it. The snapshot is rewritten
//! whole on every mutation, so the interesting column is not the mutation but
//! the save and load around it.
//!
//! ```console
//! $ cargo run --release -p claimdag --example bench -- [sizes]
//! ```
//!
//! Sizes default to 100,1000,4000.

use std::time::Instant;

use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};

/// Median milliseconds over `reps` runs.
fn timed(mut call: impl FnMut(), reps: usize) -> f64 {
    let mut times: Vec<f64> = Vec::with_capacity(reps);
    for _ in 0..reps {
        let start = Instant::now();
        call();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).expect("no nan"));
    times[times.len() / 2]
}

/// Add one node and return its id.
fn add(graph: &mut WorkGraph, index: usize) -> WorkId {
    let summary = format!("work item {index}");
    let id = graph.mint_id(WorkKind::Task, WorkId::ZERO, &summary);
    graph
        .upsert(
            id,
            WorkFields {
                kind: WorkKind::Task,
                status: WorkStatus::Todo,
                role: WorkRole::Unset,
                parent: WorkId::ZERO,
                actor: WorkId::ZERO,
                summary: &summary,
            },
        )
        .expect("upsert");
    id
}

fn main() -> Result<(), String> {
    let sizes: Vec<usize> = std::env::args()
        .nth(1)
        .map(|raw| raw.split(',').filter_map(|s| s.parse().ok()).collect())
        .unwrap_or_else(|| vec![100, 1000, 4000]);

    let dir = std::env::temp_dir().join(format!("claimdag-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let header = format!(
        "{:>7} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "nodes", "upsert", "save", "load", "list", "claim", "link"
    );
    println!("{header}");
    println!("{}", "-".repeat(header.len()));

    let mut graph = WorkGraph::default();
    let mut made = 0usize;
    let assignee = WorkId::from_hex("0000000000000000000000000000000f").expect("id");

    for target in sizes {
        while made < target {
            add(&mut graph, made);
            made += 1;
        }

        let upsert = timed(
            || {
                add(&mut graph, made);
                made += 1;
            },
            5,
        );

        let save = timed(|| graph.save_dir(&dir).expect("save"), 5);
        let load = timed(
            || {
                let read = WorkGraph::load_dir(&dir);
                std::hint::black_box(read.list().len());
            },
            5,
        );
        let list = timed(
            || {
                std::hint::black_box(graph.list_view(false, false).len());
            },
            5,
        );

        // A fresh node each time, because a claim is only a claim once: the
        // repeat would otherwise measure the status check that rejects it. The
        // release is inside the timing because occupancy is a graph predicate,
        // so a held claim changes what the next one costs.
        let claim = timed(
            || {
                let id = add(&mut graph, made);
                made += 1;
                graph.claim(id, assignee, None).expect("claim");
                graph
                    .complete(id, WorkStatus::Done, "", assignee)
                    .expect("complete");
            },
            3,
        );

        let first = add(&mut graph, made);
        let second = add(&mut graph, made + 1);
        made += 2;
        let link = timed(
            || {
                graph.link_dep(first, second, WorkId::ZERO).expect("link");
                graph
                    .unlink_dep(first, second, WorkId::ZERO)
                    .expect("unlink");
            },
            3,
        );

        println!(
            "{:>7} {upsert:>8.2}m {save:>8.2}m {load:>8.2}m {list:>8.2}m {claim:>8.2}m {link:>8.2}m",
            graph.list().len()
        );
    }
    std::fs::remove_dir_all(&dir).ok();
    Ok(())
}
