# Working on claimdag

A call is not stopped because of who the caller is.

`release` and `complete` with no actor, or with the zero id, still run.
`release`, `complete`, `renew` and `set_running` still run when the actor
is not the holder. Each of those writes one line, `allow`, `warn`, and
the rule (`no-actor` or `not-assignee`), separated by tabs, on stderr and
in `warn.log`.

A default graph directory another user owns is still opened. The same
line is written with the rule `not-owner`. The directory is created with
mode 0700. A directory named by `CLAIMDAG_DIR` or `--dir` is used as
given, with no such line.

`--force` still means act for nobody, and it still appends to `forced.log`.

These stay errors, because the call cannot be applied:

- an id, kind, status or role the graph does not have
- a parent edge or a dependency that would close a cycle
- `done` while a dependency is unfinished
- a claim of a node somebody already holds, or by a holder who already has one
- a generation that is not the one stored
- a snapshot that is there and does not parse
- the directory lock still held when its wait ends

The lock wait is one second. `measurements/lock-wait.md` is why.
