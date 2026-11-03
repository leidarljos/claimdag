"""The one claimdag version, and the copies cog writes from it.

The version is written by hand in one place: ``[workspace.package]`` version
in the workspace ``Cargo.toml``. Every other copy is a cogapp block in a
file ``cogfiles.txt`` lists.

    cog -r @cogfiles.txt        rewrites the copies
    cog --check @cogfiles.txt   fails when one has drifted

Run cog from the checkout root. cogapp installs the ``cog`` command;
cocogitto installs a different one.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "Cargo.toml"


def version() -> str:
    with MANIFEST.open("rb") as handle:
        return tomllib.load(handle)["workspace"]["package"]["version"]


def minor() -> str:
    """Sphinx ``version`` is X.Y; ``release`` is the full workspace version.

    Sphinx defines ``version`` as the short X.Y form and ``release`` as the
    full version string:
    https://www.sphinx-doc.org/en/master/usage/configuration.html#confval-version
    """
    parts = version().split(".")
    if len(parts) < 2:
        return version()
    return ".".join(parts[:2])


def dependency_line() -> str:
    """The ``[workspace.dependencies]`` pin for the claimdag crate.

    Cargo keeps this text as the requirement on the published crates, and
    it will not select the path package when the requirement is
    incompatible with ``[workspace.package]`` (``failed to select a version
    for the requirement``). The pin has to name that same release.
    """
    return f'claimdag = {{ path = "crates/claimdag", version = "{version()}" }}\n'


def sphinx_lines() -> str:
    return f'release = "{version()}"\nversion = "{minor()}"\n'


def main() -> None:
    print(version())


if __name__ == "__main__":
    main()
