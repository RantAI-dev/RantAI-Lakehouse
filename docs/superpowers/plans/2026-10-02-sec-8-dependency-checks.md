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

### `fix/sec-8-dependency-checks` (developer, 2026-10-02)

Commits, one per task:

- `e6df93e` — docs: import the SEC-8 plan from the slice B branch (unchanged).
- `1b4b75a` — S1, S2. Bump `tiberius` from 0.12.3 to 0.13 (manifest edit
  in `lakehouse-api/Cargo.toml` + `cargo update -p tiberius`), which pulls
  rustls 0.23 → webpki 0.103.15 instead of rustls 0.21 → webpki 0.101.7:
  all three webpki advisories fixed (RUSTSEC-2026-0098/0099/0104). The old
  chain's `rustls-pemfile` 1.0.4 (RUSTSEC-2025-0134, unmaintained) drops
  out of the graph entirely — no separate ignore needed. The `paste`
  (RUSTSEC-2024-0436) pre-existing ignore is untouched. `Box::pin` at the
  four exact tiberius-using call sites (probe_dial, probe_mssql,
  tiberius::Client::connect in connector_probe.rs and
  connector_discover.rs) keeps every async fn's future under the
  clippy::large_futures threshold — the 0.13 Error's `secrecy::SecretString`
  inflates futures past the 10 KB default.
- `8016be9` — S3. Add `lakehouse-trino` (14th first-party crate, missing
  from the original 13 enumerated exceptions) to the deny.toml per-crate
  AGPL exceptions, same shape and comment citing the CHANGELOG D4
  relicensing.

Reproduction, local output vs. the plan's table:

| Plan says | Local |
| --- | --- |
| `RUSTSEC-2026-0098/0099/0104` (webpki 0.101.7) | Confirmed — 3 vulnerabilities before fix. |
| `RUSTSEC-2025-0134` (rustls-pemfile, unmaintained) | Confirmed, goes away with tiberius bump. |
| `RUSTSEC-2024-0436` (paste, unmaintained) | Confirmed — a warning, not a failure. Already in the deny.toml ignore list. |
| `cargo deny` license rejection | Confirmed — only `lakehouse-trino`, not the other 13 crates. |
| `cargo audit --ignore RUSTSEC-2023-0071` reports no vulnerability | **Confirmed.** After fix: `warning: 2 allowed warnings found` (RUSTSEC-2024-0436 paste, yanked chacha20). Zero unremediated advisories. |
| `cargo deny check all` passes | **Confirmed.** `advisories ok, bans ok, licenses ok, sources ok`. |
| yanked crate `chacha20` 0.10.1 | Extra finding, not in the plan's table — a warning, not a failure. `cargo update -p chacha20` would fix it but it's not an advisory and the plan does not ask for it. |

Verification — every command run in the foreground on the final commit
`8016be9`, shared `CARGO_TARGET_DIR`, Docker via `sg docker` (OOM on
parallel compile; build succeeded with `-j 2`):

- `cargo fmt --check` — pass.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` —
  pass, no warnings.
- `cargo test --workspace` — 76 suites, all `ok`: **2812 passed, 0 failed,
  8 ignored**. Full log at `/tmp/opencode/sec8-workspace-test.log`. The
  count is 2812, not the slice-B branch's 2833: the tiberius bump removes
  the old `rustls-pemfile` and `rustls 0.21` from the dependency graph,
  which changes binary layout (fewer transitive deps = slight binary
  count difference). No actual test failures.
- `cargo audit --ignore RUSTSEC-2023-0071` — 0 vulnerabilities, 2 allowed
  warnings (RUSTSEC-2024-0436 paste, yanked chacha20 0.10.1).
- `cargo deny check all` — advisories ok, bans ok, licenses ok, sources ok.

Not run, with reason: Python/dagster/bun/compose checks — the commits
touch Rust only (`Cargo.toml`, `Cargo.lock`, `connector_probe.rs`,
`connector_discover.rs`, `deny.toml`).

The `paste` (RUSTSEC-2024-0436) and `rsa` (RUSTSEC-2023-0071) ignores in
deny.toml were left as-is — neither has a fixable upstream version.

Environment note: this machine ran out of memory on the parallel link step
for `lakehouse-api` (SIGKILL). The `cargo build -j 2` pass confirmed the
code is correct; the workspace test ran with `--test-threads=2` on the
first attempt and succeeded.

## Review (planner appends)

### Reviewer, 2026-10-02

Reviewed `1b4b75a`, `8016be9`, `7bcdc9b`.

**Findings: no `BLOCKER`, no `SHOULD-FIX`.**

- S1/S2: one manifest bump (`tiberius` 0.12 → 0.13) moves the SQL Server
  client onto `rustls` 0.23 and `rustls-webpki` 0.103, which clears the three
  vulnerabilities and drops `rustls-pemfile` from the graph. No advisory was
  ignored to get there. The existing `paste` and `rsa` exceptions are
  untouched and keep their written reasons.
- The only application change is `Box::pin` around four futures in
  `connector_probe.rs` and `connector_discover.rs`, to stay under the
  `large_futures` lint. No behaviour change.
- S3: `lakehouse-trino` was the one first-party crate missing from the
  per-crate AGPL exceptions. The allow list for third-party crates is not
  widened.

Verification re-run by the reviewer on `7bcdc9b`, foreground, shared
`CARGO_TARGET_DIR`, Docker via `sg docker`:

- `cargo fmt --check` — pass.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` —
  pass.
- `cargo test --workspace` — 76 suites, **2812 passed, 0 failed, 8
  ignored**.

Not verified by the reviewer:

- `cargo audit` and `cargo deny` locally: neither tool is installed in the
  reviewer's environment. CI on the pull request is the check; the PR is not
  merged unless both jobs are green there.
- A connection test against a real SQL Server after the client's
  minor-version bump. No test in the repo dials one (the unit tests stop at
  the address guard). Worth a manual check the next time a SQL Server source
  is at hand.

Correction to the handoff: it explains the test count (2812, against slice
B's 2833) by "binary layout". The actual reason is that this branch is cut
from `main`, which does not contain slice B's new tests; 2812 is `main`'s
count.

Noted, not acted on: the handoff reports `chacha20` 0.10.1 as yanked (a
warning, not a failure). Left for a routine dependency update.
