# `SEC-30`: update `rustls` past RUSTSEC-2026-0346 to 0349

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `e2663378`. Branch: `fix/sec-30-rustls`. One PR slice.

## 1. The problem

`cargo audit` and `cargo deny` are red on `main` since 2026-10-14: the lock
file pins `rustls` 0.23.45, which has four advisories published that day.

| ID | Title (as RustSec states it) |
| --- | --- |
| RUSTSEC-2026-0346 | `Debug` impls for client session values expose TLS secrets |
| RUSTSEC-2026-0347 | Buffered application data sent to outer server after ECH rejection |
| RUSTSEC-2026-0348 | Brotli certificate decompression can allocate up to 1 GiB before authentication |
| RUSTSEC-2026-0349 | Unauthenticated ServerHello can downgrade later connections to classical key exchange |

The fix RustSec names: `rustls` >= 0.23.46. Until it lands, no PR that
touches dependencies may merge (`AGENTS.md` step 6).

## 2. Decisions

- **D1.** A lock-file update only: `cargo update -p rustls` (or
  `--precise 0.23.46` or the newest 0.23.x). No `Cargo.toml` change unless a
  crate pins `rustls` below 0.23.46; if one does, raise only that floor and
  say why in the commit.
- **D2.** No other crate moves. If `cargo update -p rustls` drags others
  along, list every crate and version that changed in the handoff
  (`git diff rust/Cargo.lock`), and stop and report if anything beyond
  `rustls` and its own `rustls-*` siblings changes a minor version.
- **D3.** No advisory is ignored or waived; `deny.toml` and the
  `cargo audit --ignore` list in `.github/workflows/security.yml` are not
  touched.

## 3. Tasks

- **T1.** The update, one commit "SEC-30 T1: update rustls to <version>
  (RUSTSEC-2026-0346 to 0349)". Check: `cargo tree -i rustls` shows only the
  new version; `cargo fmt --check`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo test --workspace --no-fail-fast`.
  `cargo audit` / `cargo deny` are not installed on this machine; CI is
  their check, and the handoff says so.
- **T2.** `CHANGELOG.md` entry a customer can read.

## 4. Out of scope

`quinn-proto` (`SEC-27`), the `gitleaks` findings (`SEC-13`), every
Dependabot PR.

## 5. Handoff

(Developer appends here.)

## 6. Review

(Reviewer.)
