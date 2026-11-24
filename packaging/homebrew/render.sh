#!/bin/sh
# Print the Homebrew formula for a release: render.sh <version> <dir with .sha256 files>
# The dir holds deck-<version>-<target>.tar.gz.sha256 for the four targets, each either the
# bare sum or `shasum` output (`<sum>  <file>`).
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: render.sh <version> <dir with .sha256 files>" >&2
  exit 2
fi
version=$1
dir=$2
template="$(dirname "$0")/deck.rb.in"

script=
for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  file="$dir/deck-$version-$target.tar.gz.sha256"
  if [ ! -f "$file" ]; then
    echo "render.sh: missing sha256 file for $target: $file" >&2
    exit 1
  fi
  read -r sum _ <"$file" || true
  case "$sum" in
    *[!0-9a-f]* | "")
      echo "render.sh: no sha256 sum for $target in $file" >&2
      exit 1
      ;;
  esac
  if [ "${#sum}" -ne 64 ]; then
    echo "render.sh: bad sha256 sum for $target in $file" >&2
    exit 1
  fi
  upper=$(printf '%s' "$target" | tr 'a-z-' 'A-Z_')
  script="$script
s/@SHA256_${upper}@/$sum/g"
done

out=$(sed -e "s/@VERSION@/$version/g" -e "$script" "$template")
if printf '%s\n' "$out" | grep -q '@[A-Z0-9_]*@'; then
  echo "render.sh: placeholders left in the formula" >&2
  exit 1
fi
printf '%s\n' "$out"
