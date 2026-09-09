#!/usr/bin/env python3
"""What the work graph costs as it fills toward its node cap.

The cap is why this is short: the graph cannot grow without bound, so what
matters is that nothing degrades on the way to it.

    python3 scripts/bench_graph.py target/release/claimdag
"""
from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SIZES = (100, 1000, 4000)
ASSIGNEE = "0" * 31 + "1"


class Graph:
    """The command line, against one directory."""

    def __init__(self, binary: Path, root: Path) -> None:
        self.binary = binary
        self.root = root
        self.ids: list[str] = []

    def run(self, *args: str) -> str:
        return subprocess.run(
            [str(self.binary), "--dir", str(self.root), *args],
            capture_output=True,
            text=True,
            check=False,
        ).stdout

    def upsert(self) -> None:
        summary = f"work item {len(self.ids)}"
        self.ids.append(self.run("upsert", "--summary", summary).strip())

    def listing(self) -> str:
        return self.run("list")

    def claim(self) -> str:
        return self.run("claim", self.ids[0], "--assignee", ASSIGNEE)

    def link(self) -> str:
        return self.run("link", self.ids[1], self.ids[2])


def timed(call, reps: int = 5) -> float:
    """Median milliseconds over `reps` runs."""
    times = []
    for _ in range(reps):
        start = time.perf_counter()
        call()
        times.append(time.perf_counter() - start)
    times.sort()
    return times[len(times) // 2] * 1000


def main(argv: list[str]) -> int:
    binary = Path(argv[0]) if argv else Path("target/release/claimdag")
    root = Path(tempfile.mkdtemp(prefix="claimdag-bench-"))
    graph = Graph(binary, root)
    try:
        header = f"{'nodes':>7} {'upsert':>9} {'list':>9} {'claim':>9} {'link':>9}"
        print(header)
        print("-" * len(header))
        for target in SIZES:
            while len(graph.ids) < target:
                graph.upsert()
            upsert = timed(graph.upsert)
            listing = timed(graph.listing)
            claim = timed(graph.claim, reps=3)
            link = timed(graph.link, reps=3)
            print(
                f"{len(graph.ids):>7} {upsert:>8.1f}m {listing:>8.1f}m "
                f"{claim:>8.1f}m {link:>8.1f}m"
            )
        return 0
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
