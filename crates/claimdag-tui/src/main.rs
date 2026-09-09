//! `claimdag-tui`: the work graph, drawn.

use std::path::PathBuf;

use claimdag::WorkGraph;
use claimdag_tui::{forest, Handles};
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "claimdag-tui",
    version,
    about = "Terminal work graph over a claimdag work.bin"
)]
struct Cli {
    /// Directory that holds work.bin (else CLAIMDAG_DIR, else the runtime dir).
    #[arg(long)]
    dir: Option<PathBuf>,
    /// Print the parent tree as text and exit.
    #[arg(long)]
    dump: bool,
    /// With --dump, include finished and archived work.
    #[arg(long)]
    all: bool,
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let dir = claimdag::resolve_dir(cli.dir);
    if cli.dump {
        // `--dump` is a reading verb like the command line's, so it says when
        // there is nothing at the directory rather than printing an empty
        // forest that looks like a quiet seat.
        let graph = match WorkGraph::open_dir(&dir) {
            Ok(graph) => graph,
            Err(absent) => {
                eprintln!("{absent}");
                std::process::exit(1);
            }
        };
        let nodes = graph.list_view(cli.all, cli.all);
        print!("{}", forest::dump(&nodes, &Handles::load(&dir)));
        return Ok(());
    }
    claimdag_tui::run(dir)
}
