# Changelog

Versions follow semver at 0.x: a minor bump is a feature, a patch is a fix.

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
