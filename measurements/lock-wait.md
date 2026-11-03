# The lock's load-to-save section at the node cap

`WAIT` and `RETRY` in `crates/claimdag/src/lock.rs` come from these numbers.

The section is what one command-line write holds the graph lock for: take
the lock, `load_for_write`, one upsert, `save_dir`, let the lock go. It is
timed on a graph filled with finished work until prune holds it at the
4096-node cap (`work.bin` 786,528 bytes). The upsert adds a node, so every
section also runs a prune.

```console
$ cargo test --release -p claimdag --lib section_at_the_cap -- --ignored --nocapture
```

Machine: Intel Xeon (family 6, model 207), 8 cores, one thread per core,
Linux 6.12, ext4. CPU only. rustc 1.97.1, release profile.

Before is `e0f79d7ee343a74819ccb7421bc9ac644c1d8479` (v0.8.0) with only the
measurement test added; its lock blocks for as long as the holder holds.
After is the commit that adds this file, whose lock also writes the pid in
and empties the file on release. Ten runs of 1000 sections each, before
and after taken in turn.

| run | before median | before p99 | before max | after median | after p99 | after max |
|----:|--------------:|-----------:|-----------:|-------------:|----------:|----------:|
|   1 |       4.51 ms |    7.05 ms |   11.29 ms |      4.64 ms |   7.30 ms |  16.92 ms |
|   2 |       4.75 ms |   15.73 ms |   24.78 ms |      4.62 ms |   8.32 ms |   9.30 ms |
|   3 |       4.10 ms |    6.63 ms |   21.01 ms |      4.23 ms |   7.73 ms |   8.79 ms |
|   4 |       4.54 ms |    7.56 ms |   14.53 ms |      4.56 ms |   7.20 ms |   9.37 ms |
|   5 |       4.59 ms |    6.90 ms |    9.19 ms |      4.11 ms |   6.98 ms |   8.64 ms |
|   6 |       4.66 ms |    6.75 ms |    8.01 ms |      4.60 ms |   6.82 ms |   8.57 ms |
|   7 |       4.75 ms |    7.35 ms |    8.28 ms |      4.76 ms |   7.20 ms |   7.89 ms |
|   8 |       4.46 ms |    7.30 ms |    8.33 ms |      4.54 ms |   6.81 ms |   8.14 ms |
|   9 |       4.87 ms |    7.79 ms |    9.00 ms |      4.70 ms |  11.90 ms |  17.04 ms |
|  10 |       4.70 ms |    6.77 ms |    9.04 ms |      4.39 ms |   6.91 ms |   7.51 ms |

Writing the pid costs nothing the runs can tell apart: the medians are
4.10 to 4.87 ms before and 4.11 to 4.76 ms after.

Runs taken on the same machine and disk while the change was being
written, 15,000 sections more, had one section at 51.5 ms and none slower.
That is the 52 ms `WAIT` is set against: one second is 19 of those back to
back, and 200 median sections.

On tmpfs, where a sync is free, the median was 2.9 to 3.0 ms (two runs,
before): most of the section is reading and writing the snapshot, and the
two syncs in `save_dir` add about 1.5 ms on this disk. A slower disk
lengthens that part.
