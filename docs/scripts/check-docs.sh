#!/usr/bin/env bash
# Build claimdag, fail if the reference page drifted from --help, and run
# the README and getting-started console blocks in a scratch directory.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

# cogapp 3.6.0 is what CI installs. --check fails when the generated help
# would change, --diff (since 3.5) prints the line that moved, and
# --check-fail-msg (3.6) says how to refresh the page.
if ! python3 -m cogapp --help >/dev/null 2>&1; then
  echo "check-docs: need cogapp 3.6.0 (python3 -m pip install cogapp==3.6.0)" >&2
  exit 1
fi

# Debug and release print the same help. Debug is the build the test
# workflow already pays for; this job does not need a release profile.
cargo build -p claimdag-cli

bindir=$(mktemp -d)
trap 'rm -rf "$bindir"' EXIT
ln -s "$root/target/debug/claimdag" "$bindir/claimdag"
# Prefer this binary over any claimdag already on PATH.
export PATH="$bindir:$PATH"
export CLAIMDAG_BIN="$bindir/claimdag"
export CLAIMDAG_ROOT="$root"
export NO_COLOR=1
export LC_ALL=C

fail=0
python3 -m cogapp --check --diff \
  --check-fail-msg "reference.org drifted from claimdag --help; run docs/scripts/check-docs.sh" \
  docs/orgmode/reference.org || fail=1
python3 "$root/docs/scripts/run-doc-examples.py" || fail=1
exit "$fail"
