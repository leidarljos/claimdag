#!/usr/bin/env python3
"""Run the README and getting-started console blocks in a scratch directory.

A line the page shows succeeding has to exit 0, and the lines under it have
to be what the program prints. A line whose shown output is a refusal has
to exit non-zero and print that refusal. Anything else is the page lying
about the command.

Ids are minted with the clock in the salt, so the tutorials abbreviate one
as eight hex digits and an ellipsis. That token stands for the 32 hex
characters the program prints. A refusal names the unix second the holder
was last written, and that second moves with the clock, so the page writes
`since unix ...` for those digits. The rest of the line is literal,
including the two spaces the list printer puts between fields.
"""

from __future__ import annotations

import os
import pathlib
import re
import shlex
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
PAGES = (
    ROOT / "README.md",
    ROOT / "docs" / "orgmode" / "getting-started.org",
)

# The directories the two pages name. CI has no /var/lib/seat, and
# /tmp/demo-claims would be shared with whatever else is on the machine.
# Both become the scratch directory this run created.
DOC_DIRS = ("/tmp/demo-claims", "/var/lib/seat")

# Eight hex digits is how these pages abbreviate an id. The program prints
# 32, which is the width of a WorkId. A shorter or longer abbreviation is
# not what the pages do, so it stays literal and the comparison fails.
_ID_TOKEN = re.compile(r"[0-9a-f]{8}\.\.\.")
# Three dots after `since unix` stand for the digits of that second. A
# shorter or longer ellipsis stays literal, as a short id abbreviation does.
_UNIX_TOKEN = re.compile(r"since unix \.\.\.")
_WILDCARD = re.compile(rf"{_ID_TOKEN.pattern}|{_UNIX_TOKEN.pattern}")
_PLACEHOLDER = re.compile(r"<[a-z]+>")


def line_regex(expected: str) -> re.Pattern[str]:
    pieces: list[str] = []
    last = 0
    for match in _WILDCARD.finditer(expected):
        pieces.append(re.escape(expected[last : match.start()]))
        if match.group(0).startswith("since unix"):
            pieces.append(r"since unix [0-9]+")
        else:
            pieces.append(r"[0-9a-f]{32}")
        last = match.end()
    pieces.append(re.escape(expected[last:]))
    return re.compile("".join(pieces))


def check_abbreviation_rule() -> None:
    """The abbreviation stands for any minted id, and spacing stays literal."""
    expected = "092dca29...  todo  task  gen=1  x"
    pattern = line_regex(expected)
    good = "829a60c1cc05f1121d0d5eb488cfd43a  todo  task  gen=1  x"
    # Three spaces is the old hand-aligned column, not what list prints.
    bad = "829a60c1cc05f1121d0d5eb488cfd43a  todo   task  gen=1  x"
    if pattern.fullmatch(good) is None or pattern.fullmatch(bad) is not None:
        raise SystemExit("run-doc-examples: id abbreviation rule is wrong")
    refusal = (
        "claim: status claimed. node 092dca29... is held by "
        "00000000000000000000000000000007 since unix .... "
        "claimdag release 092dca29... --actor 00000000000000000000000000000007"
    )
    printed = (
        "claim: status claimed. node 02745e23a376c46bd2602cc07857f2da is held by "
        "00000000000000000000000000000007 since unix 1793699591. "
        "claimdag release 02745e23a376c46bd2602cc07857f2da --actor "
        "00000000000000000000000000000007"
    )
    refusal_pattern = line_regex(refusal)
    if refusal_pattern.fullmatch(printed) is None:
        raise SystemExit("run-doc-examples: unix abbreviation rule is wrong")
    # The second is digits. A blank where they go is not that second.
    if refusal_pattern.fullmatch(printed.replace("unix 1793699591", "unix ")) is not None:
        raise SystemExit("run-doc-examples: unix abbreviation rule is wrong")


def console_blocks(path: pathlib.Path) -> list[list[str]]:
    text = path.read_text(encoding="utf-8")
    if path.suffix == ".org":
        pattern = re.compile(r"^#\+begin_src console[^\n]*\n(.*?)^#\+end_src", re.M | re.S)
    else:
        pattern = re.compile(r"^```console\n(.*?)^```", re.M | re.S)
    return [block.splitlines() for block in pattern.findall(text)]


class Step:
    def __init__(self, command: str, expected: list[str], skip: str | None) -> None:
        self.command = command
        self.expected = expected
        self.skip = skip
        self.run = rewrite(command)


def rewrite(command: str) -> str:
    out = command
    for directory in DOC_DIRS:
        out = out.replace(directory, '"$CLAIMDAG_DOCS_SCRATCH"')
    return out


def skip_reason(command: str) -> str | None:
    # Drop a leading VAR=value so CLAIMDAG_DIR=/tmp/... claimdag-tui is the
    # pane and not a graph command. Assignments like a=$(...) stay.
    body = re.sub(r"^(?:[A-Za-z_][A-Za-z0-9_]*=\S*\s+)+", "", command.strip())
    program = body.split(maxsplit=1)[0] if body else ""
    if program == "cargo":
        # cargo add needs a package, and cargo binstall fetches a release.
        # Neither is a graph session on a scratch directory.
        return "install lines are not a graph session"
    if program == "claimdag-tui" and "--dump" not in body.split():
        # The pane waits on a terminal. --dump is the form that exits.
        return "claimdag-tui without --dump keeps a terminal open"
    return None


def parse_block(lines: list[str]) -> list[Step]:
    steps: list[Step] = []
    command: str | None = None
    expected: list[str] = []

    def finish() -> None:
        nonlocal command, expected
        if command is None:
            return
        steps.append(Step(command, expected, skip_reason(command)))
        command = None
        expected = []

    for line in lines:
        if line.startswith("$ "):
            finish()
            command = line[2:]
            continue
        if line.strip() == "":
            # Blank lines in these blocks are spacing between commands,
            # not a blank line the program is claimed to print.
            continue
        if command is None:
            raise SystemExit(f"run-doc-examples: output before a command: {line}")
        expected.append(line)
    finish()
    return steps


def lines_of(text: str) -> list[str]:
    if text.endswith("\n"):
        text = text[:-1]
    if text == "":
        return []
    return text.split("\n")


def same_lines(expected: list[str], actual: list[str]) -> bool:
    # `list` sorts by updated_unix, which is whole seconds, and breaks a tie
    # on id. Two nodes written in one second have no stable order, so the
    # lines under a command have to show up, not show up in one order.
    if len(expected) != len(actual):
        return False
    unused = list(actual)
    for want in expected:
        pattern = line_regex(want)
        for index, got in enumerate(unused):
            if pattern.fullmatch(got):
                del unused[index]
                break
        else:
            return False
    return not unused


def write_script(path: pathlib.Path, steps: list[Step], scratch: str, outdir: str) -> None:
    parts = [
        "#!/usr/bin/env bash\n",
        # Statuses are recorded for every line, including a refusal the
        # tutorial shows, so a failing command must not abort the script.
        "set +e\n",
        "set -u\n",
        f"export CLAIMDAG_DOCS_SCRATCH={shlex.quote(scratch)}\n",
        'export CLAIMDAG_DIR="$CLAIMDAG_DOCS_SCRATCH"\n',
        'cd "$CLAIMDAG_DOCS_SCRATCH" || exit 1\n',
        f"outdir={shlex.quote(outdir)}\n",
    ]
    for number, step in enumerate(steps):
        parts.append(
            "eval \"$(cat <<'CLAIMDAG_DOC_CMD'\n"
            f"{step.run}\n"
            "CLAIMDAG_DOC_CMD\n"
            f')\" >"$outdir/{number}.out" 2>"$outdir/{number}.err"\n'
            f"printf '%s\\n' \"$?\" >\"$outdir/{number}.status\"\n"
        )
    parts.append("exit 0\n")
    path.write_text("".join(parts), encoding="utf-8")


def report_mismatch(page: pathlib.Path, step: Step, status: int, stdout: str, stderr: str) -> str:
    return (
        f"{page.name}: command did not do what the page shows\n"
        f"  $ {step.command}\n"
        f"  exit {status}\n"
        f"  stdout: {stdout!r}\n"
        f"  stderr: {stderr!r}\n"
        f"  page:   {step.expected!r}"
    )


def run_page(page: pathlib.Path, env: dict[str, str]) -> list[str]:
    problems: list[str] = []
    blocks = console_blocks(page)
    if not blocks:
        return [f"{page}: no console blocks"]
    steps: list[Step] = []
    for block in blocks:
        steps.extend(parse_block(block))
    ran = [step for step in steps if step.skip is None]
    if not ran:
        return [f"{page}: no commands ran"]
    for step in ran:
        for directory in DOC_DIRS:
            if directory in step.run:
                problems.append(f"{page.name}: {directory} still in: {step.command}")
        if _PLACEHOLDER.search(step.run):
            problems.append(f"{page.name}: placeholder left in: {step.command}")
    if problems:
        return problems

    with tempfile.TemporaryDirectory(prefix="claimdag-docs-") as scratch:
        with tempfile.TemporaryDirectory(prefix="claimdag-docs-out-") as outdir:
            script = pathlib.Path(outdir) / "run.sh"
            write_script(script, ran, scratch, outdir)
            proc = subprocess.run(
                ["bash", str(script)],
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )
            if proc.returncode != 0:
                return [
                    f"{page.name}: example script exited {proc.returncode}\n"
                    f"{proc.stdout}\n{proc.stderr}"
                ]
            for number, step in enumerate(ran):
                status_path = pathlib.Path(outdir) / f"{number}.status"
                if not status_path.is_file():
                    problems.append(f"{page.name}: no status for: {step.command}")
                    continue
                status = int(status_path.read_text(encoding="utf-8").strip())
                stdout = (pathlib.Path(outdir) / f"{number}.out").read_text(encoding="utf-8")
                stderr = (pathlib.Path(outdir) / f"{number}.err").read_text(encoding="utf-8")
                if step.expected:
                    # A refusal is stderr and nothing on stdout. A success
                    # the page quotes is stdout and nothing on stderr.
                    other = stdout if status != 0 else stderr
                    if status == 0 and not same_lines(step.expected, lines_of(stdout)):
                        problems.append(report_mismatch(page, step, status, stdout, stderr))
                    elif status != 0 and not same_lines(step.expected, lines_of(stderr)):
                        problems.append(report_mismatch(page, step, status, stdout, stderr))
                    elif lines_of(other):
                        problems.append(report_mismatch(page, step, status, stdout, stderr))
                elif status != 0 or lines_of(stderr):
                    problems.append(report_mismatch(page, step, status, stdout, stderr))
    skipped = [step for step in steps if step.skip is not None]
    print(
        f"run-doc-examples: {page.relative_to(ROOT)}: "
        f"{len(ran)} commands, {len(skipped)} skipped",
        flush=True,
    )
    for step in skipped:
        print(f"run-doc-examples: skip {page.name}: {step.command} ({step.skip})", flush=True)
    return problems


def main() -> int:
    check_abbreviation_rule()
    claimdag = shutil.which("claimdag")
    if claimdag is None:
        print("run-doc-examples: claimdag is not on PATH", flush=True)
        return 1
    env = os.environ.copy()
    env["NO_COLOR"] = "1"
    env["LC_ALL"] = "C"
    env.pop("CLAIMDAG_DIR", None)
    problems: list[str] = []
    for page in PAGES:
        problems.extend(run_page(page, env))
    if problems:
        for problem in problems:
            print(f"run-doc-examples: {problem}", flush=True)
        return 1
    print("run-doc-examples: README and getting-started commands ran", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
