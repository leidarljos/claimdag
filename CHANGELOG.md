# Changelog

Versions follow semver at 0.x: a minor bump is a feature, a patch is a fix.

## 0.8.0 (2026-10-20)

- claimdag builds on capnp 0.27. capnp 0.20 marks two safe functions
  unsound (RUSTSEC-2025-0143), and the code generated from the schema
  calls one of them, so `cargo deny check` failed in every crate that
  depends on claimdag. The schema is unchanged and so are its type ids
  and struct layouts. A `work.bin` that 0.7.1 wrote opens, and saving the
  graph again writes the same bytes. A test reads such a file.
- Each release archive gets a GitHub build attestation from the release workflow, so `gh attestation verify` and the ljos installer can check that this repository built it. Releases before this one have none, and the installer counts them as not published.
- The docs no longer say the ledger keeps a reopened node's completion.
  The ledger is in memory only and `work.bin` does not store it, so each
  command-line call starts with it empty. `reopen` keeps no record of the
  earlier completion on disk.
- `release` and `complete` need an actor. `--actor` defaulted to the zero
  id, and the graph read zero as "anyone", so a call that named nobody
  could hand back or finish another seat's claim. A call with no `--actor`,
  or with the zero id, is refused. `--force` acts for nobody: it prints a
  line on stderr and appends one to `forced.log` in the graph directory.
  The library has `release_forced` and `complete_forced` for the same.
- `complete --status done` is refused while a dependency is unfinished,
  and the refusal names each open dependency with its status. A node could
  be marked done with its dependency still ready, and its dependents were
  then promoted. `failed` and `cancelled` still go through.
- A writer refuses a `work.bin` it cannot read. The command line, the MCP
  server and the pane loaded such a snapshot as an empty graph, and the
  next save kept only the node being written: on a truncated snapshot one
  `upsert` dropped 75 nodes to 1. The refusal names the file and says to
  restore it or move it aside. The snapshot is synced before the rename
  and the directory after it.
- The pane takes the graph lock for a claim, a completion or a reclaim, as
  the command line and the MCP server do. It loaded and saved without it,
  so a claim made elsewhere in between was overwritten.
- With no runtime directory the graph goes to `$XDG_STATE_HOME/claimdag`, else
  `~/.local/state/claimdag`, else `/tmp/claimdag-UID`. The shared
  `/tmp/claimdag` made a second user's first claim fail with `File exists`.
  A `/tmp/claimdag` this user owns that has a graph is still read while
  the new path has none. claimdag makes a default directory with mode
  0700 and refuses one another user owns, by name. A directory named by
  `CLAIMDAG_DIR` or `--dir` is used as given, whoever owns it.

## 0.7.1 (2026-10-10)

- The pane missed a second write in the same clock tick; it now compares
  a hash of the files it reads.
- The how-to says how several workers take work at once, and its list of
  tools names `claimdag_ready_balanced` and `claimdag_claim_next`.

## 0.7.0 (2026-10-09)

- `ready --balanced` and `claim-next` order ready work by role affinity
  and claim the chosen node in one step.
- The explanation names the generation a fencing token when the holder
  passes it back. Omitting `--gen` skips the check, so a reclaimed node
  can still be finished.

## 0.6.2 (2026-09-20)

- The how-to names the tools the server serves; `claimdag_complete` is
  titled as what it does; the architecture page names the ratatui pane.

## 0.6.0 (2026-09-12)

- `reopen ID [--actor ID]` and `claimdag_reopen`: a terminal node comes
  back to ready with the generation moved, so a tracker id that maps to
  one node can be sat on again. The ledger keeps the earlier completion.

## 0.5.0 (2026-09-12)

- `release ID [--actor ID]` and the MCP `claimdag_release`: hand one claim
  back before the work is terminal. Ready again, assignee cleared,
  generation moved. A holder that stops without it stays busy until the
  lease runs out.

## 0.4.0 (2026-09-12)

What a user gets:

- One mutator across processes: every command line and server call that
  changes the graph takes an advisory lock on the graph directory from load
  to save, so two workers claiming at once cannot lose each other's change.
- Leases: `reclaim --lease SECS` hands back every claim quiet for longer, and
  the generation moves so a stale holder is fenced at `complete --gen`.
- Critical path: the MCP `claimdag_ready` orders ready nodes by the work
  waiting below them, under a millisecond at four thousand nodes.
- `list --json`, `archive`, `unarchive`; the pane hands back stale claims
  with one key.
- A documentation site at https://leidarljos.github.io/claimdag/.

## 0.3.0

Compare-and-swap claims over a work graph with a Cap'n Proto snapshot.
