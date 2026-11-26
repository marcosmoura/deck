#!/bin/sh
# Run install.sh against a fake release in a temp directory and check what it does; run by
# `just lint-ci`. Nothing is downloaded: the release is a `file://` directory.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
script="$here/../install.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/deck-install-check.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

fail() {
  echo "install-check.sh: $*" >&2
  exit 1
}

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  *) fail "no release target for $(uname -s) $(uname -m)" ;;
esac

sum() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

# A release of one tarball that holds a fake `deck`, and its checksum.
name="deck-9.9.9-$target"
release="$tmp/release/v9.9.9"
mkdir -p "$release" "$tmp/$name"
printf '#!/bin/sh\necho fake deck\n' >"$tmp/$name/deck"
tar -C "$tmp" -czf "$release/$name.tar.gz" "$name"
echo "$(sum "$release/$name.tar.gz")  $name.tar.gz" >"$release/$name.tar.gz.sha256"

# Runs the script given on stdin as `curl ... | sh` does, installing into "$1".
install() {
  DECK_VERSION=9.9.9 DECK_DOWNLOAD_URL="file://$tmp/release" DECK_INSTALL_DIR="$1" sh
}

# The whole script installs the binary, executable, and leaves no staging file.
bin="$tmp/bin"
if ! out=$(install "$bin" <"$script" 2>&1); then
  fail "the install failed: $out"
fi
[ "$("$bin/deck")" = "fake deck" ] || fail "the installed deck is not the one of the release"
[ -z "$(find "$bin" -name '.deck*')" ] || fail "a staging file was left in $bin"

# A link waiting at a fixed staging name is not written through.
ln -s "$tmp/victim" "$bin/.deck.tmp"
install "$bin" <"$script" >/dev/null 2>&1 || fail "the second install failed"
[ ! -e "$tmp/victim" ] || fail "the install wrote through a planted link"

# A download cut short, here before the last line, runs nothing.
if ! sed '$d' "$script" | install "$tmp/cut" >/dev/null 2>&1; then
  fail "the script without its last line failed"
fi
[ ! -e "$tmp/cut" ] || fail "the script without its last line installed something"

# A tarball that does not match its checksum is refused, and installs nothing.
echo "$(printf '%064d' 0)  $name.tar.gz" >"$release/$name.tar.gz.sha256"
if out=$(install "$tmp/bad" <"$script" 2>&1); then
  fail "the install succeeded with a wrong checksum"
fi
case "$out" in
  *"checksum mismatch"*) ;;
  *) fail "the error does not name the checksum: $out" ;;
esac
[ ! -e "$tmp/bad/deck" ] || fail "a tarball with a wrong checksum was installed"

# Downloads from GitHub refuse anything but HTTPS with TLS 1.2 or newer; the script cannot
# be run against GitHub here, so its text is checked.
grep -q -F "curl --proto '=https' --tlsv1.2 " "$script" || fail "install.sh does not restrict curl to HTTPS and TLS 1.2"

echo "install.sh check ok"
