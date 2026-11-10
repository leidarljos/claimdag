#!/usr/bin/env python3
"""Write docs/orgmode/changelog.org from CHANGELOG.md."""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "CHANGELOG.md"
OUT = ROOT / "docs" / "orgmode" / "changelog.org"

_CODE = re.compile(r"`([^`]+)`")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = render(SRC.read_text(encoding="utf-8"))
    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.is_file() else ""
        if current != text:
            print("sync-changelog: changelog.org drifted from CHANGELOG.md", file=sys.stderr)
            return 1
        print("sync-changelog: changelog.org matches CHANGELOG.md")
        return 0
    OUT.write_text(text, encoding="utf-8")
    print(f"sync-changelog: wrote {OUT.relative_to(ROOT)}")
    return 0


def convert_inline(text: str) -> str:
    def code(match: re.Match[str]) -> str:
        body = match.group(1)
        if "=" in body and "~" not in body:
            return f"~{body}~"
        if "=" in body:
            return body
        return f"={body}="

    return _CODE.sub(code, text)


def render(markdown: str) -> str:
    lines = [
        "#+title: Changelog",
        "#+description: What each claimdag version changed, from CHANGELOG.md.",
        "#+options: toc:nil num:nil",
        "",
        "This page is =CHANGELOG.md=. The version line in the workspace",
        "=Cargo.toml= is the version the crates publish.",
        "",
        "The [[file:reference.org][reference]] is the current command line. The",
        "[[file:getting-started.org][tutorial]] runs against that command line.",
        "",
    ]
    for raw in markdown.splitlines():
        if raw.strip() == "<!-- towncrier release notes start -->":
            continue
        if raw.startswith("# "):
            continue
        if raw.startswith("## "):
            lines.append("")
            lines.append(f"* {convert_inline(raw[3:].strip())}")
            lines.append("")
            continue
        if raw.startswith("- "):
            lines.append("- " + convert_inline(raw[2:].strip()))
            continue
        if raw.strip() == "":
            lines.append("")
            continue
        lines.append(convert_inline(raw))
    text = "\n".join(lines).rstrip() + "\n"
    text = re.sub(r"\n{3,}", "\n\n", text)
    return text


if __name__ == "__main__":
    sys.exit(main())
