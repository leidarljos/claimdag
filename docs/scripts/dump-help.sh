#!/usr/bin/env bash
# Print claimdag --help and every verb's --help, as an Org example block.
# reference.org pulls this in through cog. The docs job runs cog --check,
# so a renamed flag changes this text and the committed page no longer matches.
set -euo pipefail

# anstream colours help when stdout is a terminal. The page is plain text,
# and a hand run of this script must produce the same bytes as CI.
export NO_COLOR=1
# The strings in --help are the English in the source. A translated locale
# would make the committed page depend on the runner.
export LC_ALL=C

if [[ -n "${CLAIMDAG_BIN:-}" ]]; then
  bin=$CLAIMDAG_BIN
elif command -v claimdag >/dev/null 2>&1; then
  bin=$(command -v claimdag)
else
  echo "dump-help: claimdag is not on PATH and CLAIMDAG_BIN is unset" >&2
  exit 1
fi

# The usage line is argv0. Invoking the binary by its build path would
# commit a runner-specific path into the reference.
show_help() {
  (exec -a claimdag "$bin" "$@")
}

# Clap prints each verb on its own line in the Commands: section, indented
# two spaces, and the section ends at the next unindented heading. Help is
# not wrapped to the terminal: this crate does not enable clap's wrap_help,
# so the line breaks are the text itself.
commands=$(show_help --help | awk '
  /^Commands:/ { in_cmds = 1; next }
  in_cmds && /^[^[:space:]]/ { exit }
  in_cmds && /^  [a-z][a-z0-9-]* / { print $1 }
')

if [[ -z "${commands// }" ]]; then
  echo "dump-help: no subcommands found in claimdag --help" >&2
  exit 1
fi

# Clap pads a short option line with spaces so its columns line up. The
# spaces after the last glyph are not visible. Leaving them in would make
# a later edit that strips trailing whitespace fail the diff when no flag
# had moved.
{
  printf '%s\n' '#+begin_example'
  printf '%s\n' '$ claimdag --help'
  show_help --help

  for cmd in $commands; do
    if [[ "$cmd" == "help" ]]; then
      # `claimdag help` takes a verb to describe. `claimdag help --help`
      # is "help for the verb --help", which is not a usage page.
      continue
    fi
    printf '\n'
    printf '%s\n' "\$ claimdag $cmd --help"
    show_help "$cmd" --help
  done

  printf '%s\n' '#+end_example'
} | sed 's/[[:space:]]*$//'
