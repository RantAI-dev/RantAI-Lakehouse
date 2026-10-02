# SEC-8 — Make the dependency security checks green

**Status:** not started. Written 2026-10-02 by the planner for a developer
agent, under the role split in `AGENTS.md`.

**Base:** `main` at or after `ac8099b`. Branch `fix/sec-8-dependency-checks`.

**Why.** `cargo audit (advisories)` and `cargo deny check (all)` fail on
`main` and therefore on every pull request. They are real findings, and
while they are red a PR that touches dependencies cannot merge
(`AGENTS.md`, step 6). The slice B branch of the Gold publishing work is
waiting on this.

## What is failing (read from the CI logs of PR #60, 2026-10-02)

| Check | Finding |
| --- | --- |
| `cargo audit`, `cargo deny` advisories | `RUSTSEC-2026-0098`, `RUSTSEC-2026-0099`, `RUSTSEC-2026-0104` — vulnerabilities in the `rustls-webpki` certificate library (name-constraint handling; a reachable panic in revocation-list parsing) |
| `cargo audit`, `cargo deny` advisories | `RUSTSEC-2025-0134` — `rustls-pemfile` is unmaintained |
| `cargo audit` | `RUSTSEC-2024-0436` — reported as a warning (unmaintained crate); confirm which crate and whether it fails the job |
| `cargo deny` licenses | "license is not explicitly allowed", pointing at `license = "AGPL-3.0-or-later"` — the workspace's own licence since the relicence is not in the allow list in `rust/deny.toml` |

The developer must reproduce these locally first (`cargo install
cargo-audit cargo-deny` if absent, then the exact commands in
`.github/workflows/security.yml`) and correct this table in the handoff if
the local output differs.

## Tasks

One commit each.

### S1 — Clear the vulnerabilities

- Update the affected crates to fixed versions with the narrowest change
  that works: `cargo update -p <crate>` first; a manifest bump only if the
  fix needs a new minor or major version.
- If an advisory has no fixed version reachable through our dependency
  tree, do **not** silently ignore it. Stop and report: which crate pulls it
  in, and what the options are.
- **Accept:** `cargo audit --ignore RUSTSEC-2023-0071` (the command CI runs)
  reports no vulnerability.

### S2 — The unmaintained crates

- For each unmaintained advisory, find what pulls the crate in
  (`cargo tree -i <crate>`). Prefer updating the parent so the crate drops
  out.
- If it cannot be removed, add an ignore in `rust/deny.toml` **and** the
  matching `--ignore` in `security.yml`, each with a comment giving the
  reason and what would let it be removed, in the style of the existing
  `RUSTSEC-2023-0071` exception. An ignore without a reason is a finding.
- **Accept:** `cargo deny check advisories` passes.

### S3 — The licence check

- Add the workspace's own licence to `rust/deny.toml` in the way
  `cargo-deny` expects for first-party crates, with a comment citing the
  relicence in `CHANGELOG.md`. Do not widen the allow list for third-party
  crates beyond what is needed; if a third-party crate is the one rejected,
  stop and report it instead.
- **Accept:** `cargo deny check` passes in full.

## Rules

- Rule 2: do not weaken a check to get green. Every ignore needs a
  checkable reason.
- Dependency updates can change behaviour. Run the full verification block
  for Rust once on the final commit (`cargo fmt --check`, workspace
  `clippy`, workspace `test`). Use
  `CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target`; tests need Docker
  (`sg docker -c '...'` if the shell lacks the group).
- Do not touch application code unless an updated crate's API forces it; if
  it does, keep the change minimal and say so in the handoff.
- Do not open or merge a pull request.

## Out of scope

- The `gitleaks (full git history)` job. It is red on purpose
  (`docs/CI.md`); backlog `SEC-1`.
- Front-end dependency updates.

## Handoff (developer appends)

_Empty._

## Review (planner appends)

_Empty._
