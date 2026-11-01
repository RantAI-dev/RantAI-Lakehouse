#!/usr/bin/env bash
# REL-2 S3-T4: install the RantAI Lakehouse with one command.
#
#   curl -fsSL https://get.rantai.dev/lakehouse | bash
#
# WHAT IT DOES
#   1. checks the host: Linux on x86_64 (REL-2 decision D12 — arm64
#      follows when every third-party image in the stack has an arm64
#      build);
#   2. downloads `lakehousectl-linux-x86_64` and the release's
#      `SHA256SUMS`;
#   3. checks the binary's checksum against the release's own list, so
#      a download that was tampered with is refused before it runs;
#   4. runs `lakehousectl install "$@"`, which does the rest: the
#      pre-flight, the verification of the release, the secrets, the
#      images and the stack.
#
# The version is the newest release, unless one is named:
#
#   curl -fsSL https://get.rantai.dev/lakehouse | bash -s -- --version 0.2.0
#
# Every argument is passed on to `lakehousectl install`, so
# `--bundle <dir>` (the air-gapped install), `--dir <path>` and
# `--ignore-preflight <check>` work as they do there.
#
# WHY THE SCRIPT IS SO SMALL
#   It trusts nothing but the checksum: the binary it downloads is
#   checked against the release's `SHA256SUMS`, and the binary itself
#   verifies the release against the key compiled into it (REL-2
#   decision D3). The script needs `curl` and `sha256sum`, and
#   nothing else.
set -euo pipefail

RELEASES_URL="https://github.com/RantAI-dev/RantAI-Lakehouse/releases/download"
LATEST_URL="https://api.github.com/repos/RantAI-dev/RantAI-Lakehouse/releases/latest"
BINARY="lakehousectl-linux-x86_64"

say() {
    echo "install.sh: $*"
}

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

require_command() {
    command -v "$1" > /dev/null 2>&1 || fail "$1 is needed and is not installed."
}

# The host: Linux on x86_64, with a clear refusal otherwise.
case "$(uname -s)" in
    Linux) ;;
    *) fail "this release runs on Linux, and this host is $(uname -s)." ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) ;;
    *) fail "this release runs on x86_64, and this host is $(uname -m). The arm64 build is not out yet (REL-2 decision D12)." ;;
esac

require_command curl
require_command sha256sum

# The version: the newest release, unless one is named. The arguments
# are read, not consumed: every one of them is passed on to
# `lakehousectl install`.
version="${LAKEHOUSE_VERSION:-}"
previous=""
for argument in "$@"; do
    if [ "$previous" = "--version" ]; then
        version="$argument"
    fi
    case "$argument" in
        --version=*) version="${argument#--version=}" ;;
    esac
    previous="$argument"
done

if [ -z "$version" ]; then
    say "asking GitHub for the newest release."
    tag="$(curl -fsSL "$LATEST_URL" \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        | head -n 1)"
    [ -n "$tag" ] || fail "the newest release could not be read from $LATEST_URL."
    version="${tag#v}"
fi
say "installing the release $version."

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT HUP INT TERM

say "downloading $BINARY."
curl -fsSL -o "$work/$BINARY" "$RELEASES_URL/v$version/$BINARY" \
    || fail "$BINARY could not be downloaded from the release $version."
say "downloading the release's checksums."
curl -fsSL -o "$work/SHA256SUMS" "$RELEASES_URL/v$version/SHA256SUMS" \
    || fail "the release's checksums could not be downloaded."

# The checksum: the release's own list says what the binary hashes to.
expected="$(awk -v name="$BINARY" '$2 == name || $2 == "*" name { print $1 }' "$work/SHA256SUMS" | head -n 1)"
[ -n "$expected" ] || fail "the release's checksums name no $BINARY."
actual="$(sha256sum "$work/$BINARY" | awk '{ print $1 }')"
if [ "$expected" != "$actual" ]; then
    fail "the downloaded $BINARY does not match the release's checksum: expected $expected, got $actual."
fi
say "the checksum matches."

chmod +x "$work/$BINARY"
exec "$work/$BINARY" install "$@"
