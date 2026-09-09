<p align="center">
  <img src="docs/logo/icon.svg" width="120" height="120" alt="claimdag">
</p>

# claimdag

**CAS claim and complete on a DAG. One writer. Unpacked Cap'n on disk.**

The host process is the sole mutator. Snapshot is `work.bin` (mmap).
This crate has no RPC surface.

```console
$ cargo add claimdag
$ cargo install --path crates/claimdag-cli
$ claimdag --dir /var/lib/seat upsert --summary "land the adapter"
$ claimdag --dir /var/lib/seat list
$ claimdag --dir /var/lib/seat list --json --all
$ claimdag --dir /var/lib/seat claim <id> --assignee <actor>
$ claimdag --dir /var/lib/seat complete <id>
$ claimdag --dir /var/lib/seat archive <id>
$ claimdag --dir /var/lib/seat link <parent> <child>
```

## Law

- One in-process graph. The host is the sole mutator.
- Snapshot is unpacked Cap'n `work.bin`. Hosts mmap it.
- Ids are 128-bit `WorkId`. Kind, status, and role are closed enums.
- Summary is the only open text field.
- `claim` is CAS on `gen` (omit `--gen` to ignore). Terminal is sticky.
- `archive` is a flag on a terminal node, not a new status and not a delete.
- Default `list` is live work only (`--terminal`, `--archived`, `--all`).
- `link` is a boolean hard dependency.

```toml
claimdag = { git = "https://github.com/HaoZeke/claimdag", tag = "v0.1.3" }
```

## WorkGraph pane

The graph is a DAG over dependencies and a forest over parents. The pane draws
the forest, and mutates through the same `WorkGraph` calls the command line
makes, so an action a stranger may not take fails in the pane too.

```console
$ CLAIMDAG_DIR=/var/lib/seat claimdag-tui
$ claimdag-tui --dir /var/lib/seat --dump
```

Bindings: `c` claim, `d` complete, `a` archive, `u` unlink, `h` show finished,
`A` show archived, `r` reload, `q` quit. Colours come from the terminal.

`CLAIMDAG_ACTOR` pins who the pane claims as; without it the id is derived
from user and host, so the same seat is the same actor across restarts.
`handles.json` in the graph directory, or `CLAIMDAG_HANDLES`, maps actor ids
to names a person recognises.

Docs: [docs/orgmode/architecture.org](docs/orgmode/architecture.org).
Schema: [schema/claimdag.capnp](schema/claimdag.capnp).

License: Apache-2.0 OR MIT.
