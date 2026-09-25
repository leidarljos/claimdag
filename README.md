<p align="center">
  <img src="docs/logo/icon.svg" width="120" height="120" alt="claimdag">
</p>

# claimdag

**What is claimable right now?** CAS claim and complete on a DAG. One writer. Unpacked Cap'n on disk.

The host process is the sole mutator. Snapshot is `work.bin` (mmap).
This crate has no RPC surface.

Docs: https://leidarljos.github.io/claimdag/

| Page | What it answers |
|---|---|
| [Getting started](https://leidarljos.github.io/claimdag/getting-started.html) | Upsert, claim, complete on a scratch graph |
| [How-to](https://leidarljos.github.io/claimdag/howto.html) | Leases, release, ready |
| [Reference](https://leidarljos.github.io/claimdag/reference.html) | Verbs, generation, snapshot |
| [Explanation](https://leidarljos.github.io/claimdag/explanation.html) | Why completing does not close a ticket |

The seat that claims through this graph is documented at https://leidarljos.github.io.

```console
$ cargo add claimdag
$ cargo install --path crates/claimdag-cli
```

## First minute

```console
$ export CLAIMDAG_DIR=/tmp/demo-claims
$ a=$(claimdag upsert --summary "parse the manifest header")
$ b=$(claimdag upsert --summary "reject a manifest with no header" --parent $a)
$ claimdag link $a $b
$ claimdag list
092dca29...  ready  task  gen=1  parse the manifest header
71368334...  todo   task  gen=1  reject a manifest with no header
```

```console
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

## WorkGraph pane

`claimdag-tui` draws the parent forest and claims, completes and archives
through the same `WorkGraph` calls the command line makes, so an action a
stranger may not take fails in the pane too. Bindings, the actor id and
the handle names are in the [how-to](https://leidarljos.github.io/claimdag/howto.html).

```console
$ CLAIMDAG_DIR=/var/lib/seat claimdag-tui
```

Docs: [docs/orgmode/architecture.org](docs/orgmode/architecture.org).
Schema: [schema/claimdag.capnp](schema/claimdag.capnp).

License: Apache-2.0 OR MIT.
