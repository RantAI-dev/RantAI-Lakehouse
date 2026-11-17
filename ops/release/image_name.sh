#!/usr/bin/env bash
# The name one image's files carry in the release bundle (REL-2 S2).
#
# WHY THIS IS ITS OWN FILE
#   The bundle script writes `images/<name>.tar` and `sbom/<name>.cdx.json`,
#   and the release pipeline writes the SBOM of every third-party image
#   under the same name. Two copies of this rule would drift, and the
#   bundle would then look for a file the pipeline never wrote: the
#   bundle would be refused for a name nobody chose. One rule, one place.
#
# THE RULE
#   The image's repository name, without its tag:
#     ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@sha256:…  →  rantai-lakehouse-api
#     postgres:16.15@sha256:…                                     →  postgres
#     quay.io/lakekeeper/catalog:v0.13.3@sha256:…                →  catalog
#
# USAGE
#   ops/release/image_name.sh <image-reference>...
#
#   Prints one name per reference, in the order given. Sourcing the file
#   instead gives the same rule as the `image_name` function.
set -euo pipefail

image_name() {
    local reference="$1"
    local without_digest="${reference%%@*}"
    local last="${without_digest##*/}"
    printf '%s\n' "${last%%:*}"
}

# Called as a command: print the name of every reference given.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    if [ "$#" -eq 0 ]; then
        echo "usage: image_name.sh <image-reference>..." >&2
        exit 2
    fi
    for reference in "$@"; do
        image_name "$reference"
    done
fi
