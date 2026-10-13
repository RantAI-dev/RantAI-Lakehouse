# `SEC-27` part 1: `cargo audit` is red on `main` — Implementation Plan

**Status:** ready to build. Asked for by the product owner on 2026-10-13.
Written by the planner (Claude Opus) for a developer agent, under the role
split in `AGENTS.md`. **Base:** `main` at `c083125`; branch
`fix/sec-27-quinn-proto`.

**Why.** The Security workflow's `cargo audit` job fails on `main`:
`quinn-proto` 0.11.17 has three advisories (`RUSTSEC-2026-0338`,
`RUSTSEC-2026-0340`, `RUSTSEC-2026-0343`, severity 7.5), each fixed in
0.11.18. `AGENTS.md` step 6: a pull request that touches dependencies must
not merge with a dependency check red.

## Decisions

- Update the lock file only: `quinn-proto` to the lowest fixed release line
  (0.11.18 or later within 0.11). No `Cargo.toml` change, no other crate
  updated unless `quinn-proto` requires it; the commit names every crate
  whose version moved.
- No advisory is ignored or waived to get green (rule 2).

## Tasks

1. `cd rust && cargo update -p quinn-proto` (add `--precise` if it moves
   more than needed). Check: `git diff rust/Cargo.lock` shows only
   `quinn-proto` and what it requires; the workspace clippy command passes
   with `--locked`.

## Out of scope

The two `gitleaks` jobs (`SEC-27` part 2). The `BACKLOG.md` row for `SEC-27`
arrives with pull request #120.

## Not verified locally

`cargo audit`, `cargo deny` and `cargo test`: the pull request's Security
and CI workflows are the proof.

## Handoff

*(developer)*

## Review

*(planner)*
