# REL-16 — the one image every one-shot init job of the stack runs.
#
# Why this image exists
# ---------------------
# The init jobs used to run plain Alpine and `apk add` curl, jq or openssl
# from their `command:` on every start. On an air-gapped install there is no
# package mirror (dl-cdn.alpinelinux.org), so the stack failed on first
# start even though every image in the bundle was present — a one-off
# "temporary error" from the Alpine CDN was enough to fail G1 in CI.
#
# Installing the tools at BUILD time is the whole fix: starting one of
# these containers needs no network at all. The image is built and shipped
# by the release just like the API, console, Dagster and oidc-mock images
# (.github/workflows/release.yml, ops/release/release-images.json), and the
# air-gap proof in that workflow now blocks container egress so a future
# runtime package install would fail the proof instead of shipping.
#
# The base is the exact Alpine digest the init jobs used before this image
# existed, so the runtime stays the same and only WHEN the tools are
# installed changes. Pinned by digest for the same reason every other
# image in this repository is: a name moves, a digest does not.
FROM alpine:3.20@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc

# curl/jq: Lakekeeper bootstrap, warehouse registration and grants.
# openssl: the G6 SQL Server fixture's one-shot certificate job.
# --no-cache leaves no apk index in the layer, keeping the image small.
RUN apk add --no-cache curl jq openssl
