#!/bin/sh
# Render the formula with fake sums and check it; run by `just lint-ci`.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/deck-brew.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

n=0
for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  n=$((n + 1))
  sum=$(printf '%064d' "$n" | tr 0 a)
  # Both formats release.yml may write: `shasum` output and the bare sum.
  if [ "$n" -eq 1 ]; then
    echo "$sum" >"$tmp/deck-9.9.9-$target.tar.gz.sha256"
  else
    echo "$sum  deck-9.9.9-$target.tar.gz" >"$tmp/deck-9.9.9-$target.tar.gz.sha256"
  fi
done

sh "$here/render.sh" 9.9.9 "$tmp" >"$tmp/deck.rb"
if grep -q '@' "$tmp/deck.rb"; then
  echo "check.sh: placeholder left in the rendered formula" >&2
  exit 1
fi
grep -q 'v9.9.9/deck-9.9.9-aarch64-apple-darwin.tar.gz' "$tmp/deck.rb"

rm "$tmp/deck-9.9.9-x86_64-apple-darwin.tar.gz.sha256"
if err=$(sh "$here/render.sh" 9.9.9 "$tmp" 2>&1 >/dev/null); then
  echo "check.sh: render.sh succeeded with a missing sum" >&2
  exit 1
fi
case "$err" in
  *x86_64-apple-darwin*) ;;
  *)
    echo "check.sh: the error does not name the missing target: $err" >&2
    exit 1
    ;;
esac
echo "homebrew formula render check ok"
