"""WorkGraph pane: fixture tree, CLI bindings, Tokyo Night Storm tokens."""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path

import textual
from textual.widgets import Tree

from claimdag_tui.theme import CSS_FILES
from claimdag_tui.app import (
    WorkGraphApp,
    default_dir,
    dump_forest,
    load_nodes,
    main,
    parent_tree,
    visible_nodes,
)

FIXTURE_DIR = Path(__file__).resolve().parent / "fixtures"


def test_textual_imports() -> None:
    assert textual.__version__


def test_load_fixture_nodes() -> None:
    nodes = load_nodes(FIXTURE_DIR)
    assert [n["summary"] for n in nodes] == [
        "land the adapter",
        "write the tree",
        "verify the tree",
    ]


def test_parent_tree_of_fixture() -> None:
    by_id, children, roots = parent_tree(load_nodes(FIXTURE_DIR))
    assert roots == ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]
    assert children["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"] == [
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "cccccccccccccccccccccccccccccccc",
    ]
    assert by_id["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]["summary"] == "write the tree"


def test_dump_forest_fixture() -> None:
    text = dump_forest(load_nodes(FIXTURE_DIR))
    assert text == (
        "ready  land the adapter\n"
        "  todo  write the tree  deps=1\n"
        "  todo  verify the tree  deps=1\n"
    )


def test_dump_cli_writes_fixture_tree(capsys) -> None:
    assert main(["--dir", str(FIXTURE_DIR), "--dump"]) == 0
    out = capsys.readouterr().out
    assert "land the adapter" in out
    assert "write the tree" in out
    assert "verify the tree" in out


def test_default_dir_claimdag_then_xdg(monkeypatch, tmp_path) -> None:
    claimed = tmp_path / "claimed"
    monkeypatch.setenv("CLAIMDAG_DIR", str(claimed))
    assert default_dir() == claimed
    monkeypatch.delenv("CLAIMDAG_DIR")
    monkeypatch.setenv("XDG_RUNTIME_DIR", str(tmp_path))
    assert default_dir() == tmp_path / "claimdag"


def test_storm_tokens() -> None:
    shared = CSS_FILES[0].read_text(encoding="utf-8")
    widgets = CSS_FILES[1].read_text(encoding="utf-8")
    assert CSS_FILES[0].name == "tokyo_night_storm.tcss"
    assert "#24283b" in shared
    assert "(36, 40, 59)" in shared
    assert "#c0caf5" in shared
    assert "#7aa2f7" in shared
    assert "$panel:" not in shared
    assert "height: 1fr" in widgets


def test_app_tree_renders_fixture(monkeypatch) -> None:
    raw = json.loads((FIXTURE_DIR / "work.json").read_text(encoding="utf-8"))
    nodes = raw["nodes"]
    monkeypatch.setattr("claimdag_tui.app.load_nodes", lambda _d: nodes)
    app = WorkGraphApp(directory=FIXTURE_DIR)

    async def run() -> list[str]:
        async with app.run_test() as _pilot:
            tree = app.query_one(Tree)
            root_kids = list(tree.root.children)
            assert root_kids
            labels = [str(root_kids[0].label)]
            labels.extend(str(child.label) for child in root_kids[0].children)
            return labels

    labels = asyncio.run(run())
    assert any("land the adapter" in lab for lab in labels)
    assert any("write the tree" in lab for lab in labels)
    assert any("verify the tree" in lab for lab in labels)


def test_claim_complete_unlink_call_cli(monkeypatch, tmp_path) -> None:
    stub = tmp_path / "claimdag"
    log = tmp_path / "cli.log"
    stub.write_text(
        "#!/bin/sh\n"
        f'{{ printf "%s " "$@"; printf "\\n"; }} >> "{log}"\n'
        "exit 0\n"
    )
    stub.chmod(0o755)
    monkeypatch.setenv("PATH", f"{tmp_path}{os.pathsep}{os.environ.get('PATH', '')}")
    monkeypatch.setenv("CLAIMDAG_BIN", "claimdag")
    monkeypatch.setenv("CLAIMDAG_ACTOR", "11111111111111111111111111111111")

    (tmp_path / "work.json").write_text(
        (FIXTURE_DIR / "work.json").read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    app = WorkGraphApp(directory=tmp_path)

    async def run() -> None:
        async with app.run_test() as pilot:
            tree = app.query_one(Tree)
            goal = tree.root.children[0]
            child = goal.children[0]
            tree.move_cursor(child)
            await pilot.press("c")
            await pilot.press("d")
            await pilot.press("a")
            await pilot.press("u")

    asyncio.run(run())
    recorded = [line.rstrip() for line in log.read_text(encoding="utf-8").splitlines()]
    child_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    parent_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    actor = "11111111111111111111111111111111"
    assert f"--dir {tmp_path} claim {child_id} --assignee {actor}" in recorded
    assert f"--dir {tmp_path} complete {child_id} --status done --actor {actor}" in recorded
    assert f"--dir {tmp_path} archive {child_id} --actor {actor}" in recorded
    assert f"--dir {tmp_path} unlink {parent_id} {child_id} --actor {actor}" in recorded


def test_poll_reloads_upsert_without_pressing_r(tmp_path) -> None:
    def write_nodes(nodes: list[dict]) -> None:
        (tmp_path / "work.json").write_text(
            json.dumps(
                {
                    "format": "claimdag/v1",
                    "next_seq": len(nodes),
                    "mint_seq": len(nodes),
                    "nodes": nodes,
                }
            ),
            encoding="utf-8",
        )

    live = {
        "id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "kind": "task",
        "status": "ready",
        "role": "general",
        "assignee": "00000000000000000000000000000000",
        "parent": "00000000000000000000000000000000",
        "deps": [],
        "cas_gen": 1,
        "created_unix": 1,
        "updated_unix": 1,
        "finished_unix": 0,
        "summary": "first-live-node",
    }
    extra = {
        **live,
        "id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "summary": "second-live-node",
        "created_unix": 2,
        "updated_unix": 2,
    }
    done = {**live, "status": "done", "finished_unix": 3, "updated_unix": 3}
    write_nodes([live])
    app = WorkGraphApp(directory=tmp_path)

    async def run() -> tuple[list[str], list[str], list[str]]:
        async with app.run_test() as _pilot:
            tree = app.query_one(Tree)

            def labels() -> list[str]:
                return [str(child.label) for child in tree.root.children]

            before = labels()
            write_nodes([live, extra])
            await asyncio.sleep(1.2)
            after_upsert = labels()
            write_nodes([done, extra])
            await asyncio.sleep(1.2)
            after_done = labels()
            return before, after_upsert, after_done

    before, after_upsert, after_done = asyncio.run(run())
    assert any("first-live-node" in lab for lab in before)
    assert any("second-live-node" in lab for lab in after_upsert)
    assert not any("first-live-node" in lab for lab in after_done)
    assert any("second-live-node" in lab for lab in after_done)


def test_visible_nodes_hides_terminal_and_archived() -> None:
    live = {"id": "aa", "status": "todo", "summary": "live"}
    done = {"id": "bb", "status": "done", "summary": "done"}
    archived = {"id": "cc", "status": "done", "archived": True, "summary": "old"}
    nodes = [live, done, archived]
    assert visible_nodes(nodes, show_terminal=False, show_archived=False) == [live]
    assert visible_nodes(nodes, show_terminal=True, show_archived=False) == [live, done]
    assert visible_nodes(nodes, show_terminal=True, show_archived=True) == nodes
