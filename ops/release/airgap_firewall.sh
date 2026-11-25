#!/usr/bin/env bash
# REL-12: make the release proofs air-gapped IN FACT.
#
# The proofs used to sinkhole a few registry names in `/etc/hosts` only.
# That is not a cutoff: a cached DNS answer, a different registry name or
# an IP-literal pull still reaches the network, and in job 125316933212
# `docker pull alpine:3.20` SUCCEEDED on a runner where it had failed
# earlier the same day. The workflow keeps the hosts sinkhole as
# belt-and-suspenders, and this script adds the real cutoff.
#
# WHY uid 0 AND NOT A BLANKET OUTPUT DROP:
# `dockerd` and `containerd` run as root, so every `docker pull` and
# registry request leaves the host in the `OUTPUT` chain owned by uid 0.
# `Runner.Worker` runs as the NON-ROOT `runner` user and talks to GitHub
# over 443 continuously while steps run and between them. A blanket
# `OUTPUT -p tcp --dport 443 -j REJECT` would cut the runner off and hang
# or fail the workflow. Scoping the REJECT to `--uid-owner 0` blocks the
# daemon while the runner keeps its own connection. `lakehousectl` makes
# no direct registry HTTP calls: it only runs the `docker` CLI
# (`rust/crates/lakehousectl/src/docker.rs`), whose traffic originates
# from the daemon.
#
# WHY THIS SCRIPT IS THE HOST SIDE, NOT DOCKER-USER:
# Container traffic traverses `FORWARD`/`DOCKER-USER`, not host `OUTPUT`.
# When REL-12 wrote this script, Alpine init containers still ran
# `apk add --no-cache curl jq` / `openssl` at startup, so blocking container
# egress would have broken the very stack the proof installs. REL-16 (#711)
# replaced them with the bundled `rantai-lakehouse-init` image, and the
# release workflow now REJECTs container egress on `DOCKER-USER` itself.
# This script stays on the host `OUTPUT` side: the two cuts are
# complementary and the release proof asserts both before it installs.
#
# WHY LOOPBACK IS ACCEPTED FIRST:
# Both proofs call the local console/API on 127.0.0.1 and `update-proof`
# uses a throwaway registry on localhost:5000. `-o lo -j RETURN` comes
# before every REJECT, so those calls are unaffected.
#
# The jump is inserted at the head of OUTPUT and the whole chain is
# removed on `disable`; both operations are idempotent. `ip6tables` is
# optional — when it is absent, IPv4 is still set up and the script
# succeeds with a printed note.
#
# The workflow calls this as `sudo bash ops/release/airgap_firewall.sh
# enable|disable` from the repository root. `disable` is wired with
# `if: always()` in both proof jobs so the chain never outlives them.

set -euo pipefail

CHAIN="LAKEHOUSE_AIRGAP"

usage() {
  echo "usage: airgap_firewall.sh {enable|disable}" >&2
  echo "  enable   install the LAKEHOUSE_AIRGAP chain and its OUTPUT jump" >&2
  echo "  disable  remove the jump, flush and delete the chain" >&2
}

have() {
  command -v "$1" >/dev/null 2>&1
}

chain_exists() {
  "$1" -n -L "$CHAIN" >/dev/null 2>&1
}

# `-C` asks the kernel whether the rule is already there; adding it twice
# would be harmless but would make the chain drift and the tests noisy.
add_rule() {
  local tool="$1"
  shift
  if ! "$tool" -C "$@" >/dev/null 2>&1; then
    "$tool" -A "$@"
  fi
}

setup_family() {
  local tool="$1"
  local udp_reject="$2"

  if ! chain_exists "$tool"; then
    "$tool" -N "$CHAIN"
  fi
  # `-I` without a rule number inserts at position 1, so our chain is
  # checked before any other OUTPUT rule. `-C` with the same arguments
  # makes the jump idempotent.
  if ! "$tool" -C OUTPUT -j "$CHAIN" >/dev/null 2>&1; then
    "$tool" -I OUTPUT -j "$CHAIN"
  fi

  add_rule "$tool" "$CHAIN" -o lo -j RETURN
  add_rule "$tool" "$CHAIN" -m owner --uid-owner 0 -p tcp -m multiport \
    --dports 80,443 -j REJECT --reject-with tcp-reset
  add_rule "$tool" "$CHAIN" -m owner --uid-owner 0 -p udp --dport 443 \
    -j REJECT --reject-with "$udp_reject"
}

teardown_family() {
  local tool="$1"

  if "$tool" -C OUTPUT -j "$CHAIN" >/dev/null 2>&1; then
    "$tool" -D OUTPUT -j "$CHAIN"
  fi
  if chain_exists "$tool"; then
    "$tool" -F "$CHAIN"
    "$tool" -X "$CHAIN"
  fi
}

case "${1:-}" in
  enable)
    setup_family iptables icmp-port-unreachable
    if have ip6tables; then
      setup_family ip6tables icmp6-adm-prohibited
    else
      echo "airgap_firewall: ip6tables is unavailable; IPv6 egress is not blocked on this host" >&2
    fi
    ;;
  disable)
    teardown_family iptables
    if have ip6tables; then
      teardown_family ip6tables
    fi
    ;;
  *)
    usage
    exit 2
    ;;
esac
