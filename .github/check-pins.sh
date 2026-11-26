#!/bin/sh
# Checks that the workflows run only pinned code: every action by a commit SHA (a tag or a
# branch can be moved to other code), and mise-action with a fixed mise version (without
# one it downloads the latest mise on every run) and in locked mode (so that mise installs
# only what `mise.lock` records). Run by `just lint-ci`.
#
# Usage: check-pins.sh [workflow ...]   (default: every workflow of this repository)
set -eu

if [ "$#" -eq 0 ]; then
  dir="$(dirname "$0")/workflows"
  for file in "$dir"/*.yml "$dir"/*.yaml; do
    if [ -f "$file" ]; then
      set -- "$@" "$file"
    fi
  done
fi
if [ "$#" -eq 0 ]; then
  echo "no workflow to check" >&2
  exit 1
fi

status=0

# `uses: owner/repo[/path]@<40 hex digits>`, on a line of its own or in a flow mapping
# (`- { uses: … }`); an action of this repository (`./…`) has no ref. Comment lines are skipped.
unpinned="$(grep -n -E '(^|[[:space:]{,])uses:' "$@" /dev/null |
  grep -v -E '^[^:]*:[0-9]+:[[:space:]]*#' |
  grep -v -E 'uses:[[:space:]]*[^[:space:]@]+@[0-9a-f]{40}([[:space:],}]|$)' |
  grep -v -E 'uses:[[:space:]]*\./' || true)"
if [ -n "$unpinned" ]; then
  echo "actions that are not pinned to a commit SHA:" >&2
  echo "$unpinned" >&2
  status=1
fi

# A mise-action step must set `version:` among its inputs: on a line of the same step, which
# ends at the next step or at the first line indented no deeper than its dash (so a
# `version:` of a later job's matrix does not count).
unversioned="$(awk '
  function flush() {
    if (line && !versioned) printf "%s:%d: jdx/mise-action without a version\n", file, line
    line = 0
  }
  FNR == 1 { flush(); file = FILENAME; dash = -1 }
  /^[[:space:]]*(#|$)/ { next }
  { indent = match($0, /[^[:space:]]/) - 1 }
  /^[[:space:]]*- / { flush(); dash = indent }
  indent <= dash && !/^[[:space:]]*- / { flush() }
  /uses:[[:space:]]*jdx\/mise-action@/ { line = FNR; versioned = 0 }
  line && /^[[:space:]]*version:[[:space:]]*[^[:space:]#]/ { versioned = 1 }
  END { flush() }
' "$@")"
if [ -n "$unversioned" ]; then
  echo "mise-action steps that install the latest mise:" >&2
  echo "$unversioned" >&2
  status=1
fi

# A workflow that uses mise-action sets `MISE_LOCKED` to 1, for the action and for the steps
# after it: mise then installs a tool only as `mise.lock` records it, checksum included.
for file in "$@"; do
  if grep -q -E 'uses:[[:space:]]*jdx/mise-action@' "$file" &&
    ! grep -q -E '^[[:space:]]*MISE_LOCKED:[[:space:]]*("1"|1)[[:space:]]*$' "$file"; then
    echo "$file: jdx/mise-action without MISE_LOCKED" >&2
    status=1
  fi
done

exit "$status"
