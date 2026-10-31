#!/usr/bin/env bash
# REL-2 S2-T3 (decision D11): build the OFFLINE release bundle.
#
# # What one bundle is
#
# One file, `rantai-lakehouse-<version>-linux-x86_64.tar.zst`, holding
#
#   docker-compose.yml        the release's Compose file, rendered from the
#                             template with every image pinned by digest
#   release.json              the release manifest: the version, every
#                             image's digest, every file's checksum, the
#                             upgrade path
#   release.json.sig          the manifest's signature (cosign, the
#                             release key). `lakehousectl` verifies the
#                             manifest through it and trusts nothing else.
#   images/<name>.tar         one `docker save` per image, BY DIGEST
#                             REFERENCE, so the host needs no registry:
#                             `docker load` reads the file and the loaded
#                             image is the one the manifest names.
#   sbom/<name>.cdx.json      the CycloneDX SBOM of each image
#   SHA256SUMS                every file's checksum, so an operator can
#                             check the bundle before installing it
#
# `lakehousectl` joins the bundle in REL-2 S3, when the binary exists.
# Everything else is here.
#
# # Why the checksums are checked BEFORE the bundle is written
#
# The bundle is what an AIR-GAPPED host installs. There is no registry to
# ask and no network to reach, so the only thing that says "these are the
# bytes the release published" is the manifest's signature and the
# checksums. If the Compose file in the bundle is not the one the
# manifest names, the bundle is a different release wearing the same
# name — so the script refuses to build it.
#
# # Usage
#
#   ops/release/make_bundle.sh --manifest <release.json> \
#                              --compose <docker-compose.yml> \
#                              --signature <release.json.sig> \
#                              --sbom-dir <dir> \
#                              --out <dir>
#
#   manifest   the release manifest the pipeline wrote
#   compose    the rendered Compose file the release ships
#   signature  the manifest's signature
#   sbom-dir   the directory holding `<image name>.cdx.json` per image
#   out        the directory the bundle is written to
#
# # What it needs
#
# Docker (for `docker save`), `jq`, `zstd`, `tar` and `sha256sum`.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: make_bundle.sh --manifest <release.json> --compose <docker-compose.yml> \
                       --signature <release.json.sig> --sbom-dir <dir> --out <dir>
EOF
}

fail() {
    echo "make_bundle: $*" >&2
    exit 1
}

require_command() {
    command -v "$1" > /dev/null 2>&1 || fail "$1 is needed and is not installed."
}

# The name a bundle file carries: the reference's repository name without
# its tag. `ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@sha256:…` is
# `rantai-lakehouse-api`, and the SBOM of that image is expected to be
# `sbom/rantai-lakehouse-api.cdx.json` — the same rule the pipeline uses
# when it writes the SBOMs.
image_name() {
    local reference="$1"
    local without_digest="${reference%%@*}"
    local last="${without_digest##*/}"
    printf '%s\n' "${last%%:*}"
}

manifest=""
compose=""
signature=""
sbom_dir=""
out_dir=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --manifest) manifest="${2:-}"; shift 2 ;;
        --compose) compose="${2:-}"; shift 2 ;;
        --signature) signature="${2:-}"; shift 2 ;;
        --sbom-dir) sbom_dir="${2:-}"; shift 2 ;;
        --out) out_dir="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) usage; fail "unknown argument: $1" ;;
    esac
done

[ -n "$manifest" ] || { usage; fail "--manifest is needed."; }
[ -n "$compose" ] || { usage; fail "--compose is needed."; }
[ -n "$signature" ] || { usage; fail "--signature is needed."; }
[ -n "$sbom_dir" ] || { usage; fail "--sbom-dir is needed."; }
[ -n "$out_dir" ] || { usage; fail "--out is needed."; }

[ -f "$manifest" ] || fail "the manifest $manifest does not exist."
[ -f "$compose" ] || fail "the Compose file $compose does not exist."
[ -f "$signature" ] || fail "the signature $signature does not exist."
[ -d "$sbom_dir" ] || fail "the SBOM directory $sbom_dir does not exist."

require_command docker
require_command jq
require_command zstd
require_command tar
require_command sha256sum

version="$(jq -er '.version' "$manifest")" || fail "the manifest has no version."
jq -e '.images | length > 0' "$manifest" > /dev/null \
    || fail "the manifest names no image."

# The Compose file the bundle carries must be the one the manifest names.
# The manifest is what the operator's signature covers, so a bundle with
# a different Compose file is a different release under the same name.
expected_compose_sha="$(jq -er '.files["docker-compose.yml"]' "$manifest")" \
    || fail "the manifest names no Compose file checksum."
actual_compose_sha="$(sha256sum "$compose" | awk '{print $1}')"
if [ "$expected_compose_sha" != "$actual_compose_sha" ]; then
    fail "the Compose file does not match the manifest's checksum: expected $expected_compose_sha, got $actual_compose_sha."
fi

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/images" "$stage/sbom"
cp "$compose" "$stage/docker-compose.yml"
cp "$manifest" "$stage/release.json"
cp "$signature" "$stage/release.json.sig"

# One `docker save` per image, by digest reference: the host that loads
# the bundle gets the exact image the manifest names, with no registry to
# ask. `unique` because several services run the same image.
while read -r reference; do
    [ -n "$reference" ] || continue
    name="$(image_name "$reference")"
    [ -n "$name" ] || fail "the image reference $reference names no image."
    echo "make_bundle: saving $name"
    docker save -o "$stage/images/$name.tar" "$reference"
    if [ ! -f "$sbom_dir/$name.cdx.json" ]; then
        fail "no SBOM for $name: $sbom_dir/$name.cdx.json does not exist. Every image in a release carries one."
    fi
    cp "$sbom_dir/$name.cdx.json" "$stage/sbom/$name.cdx.json"
done < <(jq -er '.images | to_entries | map(.value) | unique | .[]' "$manifest")

# SHA256SUMS over the bundle's own contents, with paths relative to the
# bundle's root, so `sha256sum -c SHA256SUMS` works in the directory the
# bundle was extracted into.
(
    cd "$stage"
    sha256sum docker-compose.yml release.json release.json.sig \
        images/*.tar sbom/*.cdx.json > SHA256SUMS
)

bundle="rantai-lakehouse-${version}-linux-x86_64.tar.zst"
mkdir -p "$out_dir"
tar -C "$stage" -cf - docker-compose.yml release.json release.json.sig \
    images sbom SHA256SUMS | zstd -19 -T0 -o "$out_dir/$bundle"

size="$(wc -c < "$out_dir/$bundle")"
(
    cd "$out_dir"
    sha256sum "$bundle" > "$bundle.sha256"
)
echo "make_bundle: wrote $out_dir/$bundle ($size bytes, $((size / 1024 / 1024)) MiB)."
echo "make_bundle: its checksum is in $out_dir/$bundle.sha256."
