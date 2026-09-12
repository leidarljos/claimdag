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
$ claimdag --dir /var/lib/seat release <id> --actor <actor>   # hand it back unfinished
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
- `ready` is unblocked and unheld, deepest chain first. Take the first one.

## Which ready node

`ready` orders by the longest chain of unfinished work below a node, unit
cost, then recency, then id: the list-scheduling priority (Hu, DOI
10.1287/opre.9.6.841; Graham, DOI 10.1137/0117039; HEFT, DOI 10.1109/71.993206).
Every row carries `waiting_below`, so the order is visible rather than trusted.

## What this is not

claimdag is a scheduler, not a record. It holds the work one host process is
handing out right now: who is on what, what is unblocked, what finished. The
snapshot lives in the runtime directory, is capped at 4096 nodes, and prunes
finished work to stay under the cap. Nothing here is meant to outlive the
session that wrote it.

A plain-text issue tracker models the same shape. Parent edge, hard dependency,
holder, derived workable set, sticky terminal state. The two are not
redundant. A record is edited by people, carries prose, and is kept in version
control forever. A scheduler is compare-and-swap over an mmap. Forcing either to
be the other makes the record slow or the scheduler unreadable.

So the rule for picking is the tempo, not the shape: if the answer has to
survive the session, it belongs in the tracker.

### Citations are deliberately absent

A `WorkNode` has no field that could hold a deed accession, and it is not
getting one. The accession crosses a tracker, a memory pack and a deed store.
All three are durable. A scheduler that prunes its own finished nodes cannot be
an end of a citation without lying about it later.

What a node can do is name what it is scheduling. `summary` is the only open
text field, so a node that stands for a tracked issue opens its summary with
that issue's id:

```console
$ claimdag upsert --summary "vissue-4asb land the adapter"
$ claimdag list --json | grep vissue-4asb
```

That is a convention rather than a schema change, and it is the whole route.
The tracker holds the citation. The node points at the tracker. Neither reads
the other's store.

## WorkGraph pane

The graph is a DAG over dependencies and a forest over parents. The pane draws
the forest. It mutates through the same `WorkGraph` calls the command line
makes, so an action a stranger may not take fails in the pane too.

```console
$ CLAIMDAG_DIR=/var/lib/seat claimdag-tui
$ claimdag-tui --dir /var/lib/seat --dump
```

Bindings: `c` claim, `d` complete, `a` archive, `u` unlink, `h` show finished,
`A` show archived, `r` reload, `q` quit. Colours come from the terminal.

`CLAIMDAG_ACTOR` pins who the pane claims as. Without it the id is derived from
user and host, so the same seat is the same actor across restarts.
`handles.json` in the graph directory, or `CLAIMDAG_HANDLES`, maps actor ids
to names a person recognises.

Docs: [docs/orgmode/architecture.org](docs/orgmode/architecture.org).
Schema: [schema/claimdag.capnp](schema/claimdag.capnp).

License: Apache-2.0 OR MIT.
