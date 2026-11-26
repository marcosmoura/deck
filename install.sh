#!/bin/sh
# Install the deck binary from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/marcosmoura/deck/main/install.sh | sh
#
# DECK_VERSION       a version such as 0.1.0 (default: the latest release)
# DECK_INSTALL_DIR   where the binary goes (default: ~/.local/bin)
# DECK_DOWNLOAD_URL  the release download base (default: GitHub, for DECK_REPO)
set -eu

fail() {
    echo "deck install: $*" >&2
    exit 1
}

# GitHub is read over HTTPS with TLS 1.2 or newer, redirects included.
github() {
    curl --proto '=https' --tlsv1.2 -fsSL "$@"
}

# A release file: from GitHub, or as it is from the base the caller gave (a mirror, a
# `file://` directory).
download() {
    if [ -n "${DECK_DOWNLOAD_URL:-}" ]; then
        curl -fsSL "$@"
    else
        github "$@"
    fi
}

cleanup() {
    rm -rf "$tmp"
    if [ -n "$staged" ]; then
        rm -f "$staged"
    fi
}

# Everything runs from here, called on the last line: a download of this script that was
# cut short defines functions and runs nothing.
main() {
    repo="${DECK_REPO:-marcosmoura/deck}"
    install_dir="${DECK_INSTALL_DIR:-$HOME/.local/bin}"

    case "$(uname -s)-$(uname -m)" in
        Darwin-arm64) target=aarch64-apple-darwin ;;
        Darwin-x86_64) target=x86_64-apple-darwin ;;
        Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
        Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
        *) fail "no release for $(uname -s) $(uname -m)" ;;
    esac

    if [ -n "${DECK_VERSION:-}" ]; then
        version="${DECK_VERSION#v}"
    else
        # The latest release redirects to .../releases/tag/v<version>.
        latest="$(github -I -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")" ||
            fail "cannot find the latest release of $repo"
        version="${latest##*/v}"
        case "$version" in
            */* | "") fail "$repo has no release yet" ;;
        esac
    fi

    base="${DECK_DOWNLOAD_URL:-https://github.com/$repo/releases/download}/v$version"
    name="deck-$version-$target"

    staged=""
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/deck-install.XXXXXX")"
    trap cleanup EXIT

    echo "Downloading deck $version for $target"
    download -o "$tmp/$name.tar.gz" "$base/$name.tar.gz" || fail "cannot download $base/$name.tar.gz"
    download -o "$tmp/$name.tar.gz.sha256" "$base/$name.tar.gz.sha256" || fail "cannot download the checksum"

    expected="$(cut -d ' ' -f 1 "$tmp/$name.tar.gz.sha256")"
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$tmp/$name.tar.gz" | cut -d ' ' -f 1)"
    else
        actual="$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d ' ' -f 1)"
    fi
    [ "$expected" = "$actual" ] || fail "checksum mismatch for $name.tar.gz"

    tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
    mkdir -p "$install_dir"
    # Install through a temp file and a rename, so a running deck keeps its binary. The temp
    # file gets a fresh name: a fixed one could wait there as a link, in a directory that
    # others write to, and the copy would follow it.
    staged="$(mktemp "$install_dir/.deck.XXXXXX")" || fail "cannot write in $install_dir"
    cp "$tmp/$name/deck" "$staged"
    chmod 755 "$staged"
    mv -f "$staged" "$install_dir/deck"
    staged=""
    echo "Installed $install_dir/deck"

    case ":$PATH:" in
        *":$install_dir:"*) ;;
        *) echo "warning: $install_dir is not on PATH; add it to use deck" >&2 ;;
    esac
}

main "$@"
