#!/usr/bin/env bash
# Build the claimdag Sphinx/Shibuya site: org → rst → html
# Usage (from repo root or this directory):
#   ./docs/build.sh
# Optional: CLAIMDAG_DOC_EXPORTER=emacs|pandoc  CLAIMDAG_DOC_VENV=path
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VENV="${CLAIMDAG_DOC_VENV:-$ROOT/.venv-docs}"
REQ="$ROOT/docs/requirements.txt"
SRC="$ROOT/docs/source"
BUILD="$ROOT/docs/build"

echo "==> 1/4 export org → rst"
bash "$ROOT/docs/scripts/export_org_to_rst.sh"

echo "==> 2/4 ensure Python doc deps in $VENV"
if [[ ! -d "$VENV" ]]; then
  python3 -m venv "$VENV"
fi
# shellcheck disable=SC1091
source "$VENV/bin/activate"
python -m pip install -q --upgrade pip
python -m pip install -q -r "$REQ"

echo "==> 3/4 sphinx-build (Shibuya) → $BUILD"
rm -rf "$BUILD"
sphinx-build -b html -n -W "$SRC" "$BUILD"

echo "==> 4/4 rustdoc → $BUILD/rustdoc"
# claimdag-cli's binary is also named claimdag, and cargo doc writes it to the
# same path as the library. Document the library, the pane, and the server.
cargo doc --no-deps -p claimdag -p claimdag-tui -p claimdag-mcp --locked
rm -rf "$BUILD/rustdoc"
cp -a "$ROOT/target/doc" "$BUILD/rustdoc"

echo ""
echo "OK: open $BUILD/index.html"
echo "    python3 -m http.server -d $BUILD 8000   # optional"
