# Working in this repository (agents and humans)

The full code standard is `docs/CODE-STANDARD.md`. Read it before writing
code. This file is the short form an agent must hold in context at all times.

## What this repo is

RantAI Lakehouse: a Next.js console (`src/`), a Rust API workspace
(`rust/crates/*`, axum + sqlx + iceberg), a Dagster code location
(`dagster/dispar_orchestrate`), acceptance gates (`ops/g*`), and a
`docker-compose.yml` that is also the deployment template. Design decisions
are in `docs/adr/`, phases/gates/risks in
`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md`, measurements in
`docs/plans/*-RESULT.md`.

## Five principles

1. **Say why, at the code.** Module docs, migration headers, compose blocks
   explain the invariant, the security posture, the ADR/finding. Body
   comments justify decisions; they never restate the next line.
2. **Never fabricate.** Unsupported → `supported: false` with an honest
   message. Skipped → recorded with the reason. No invented metrics, counts,
   latencies, or successes. Gaps are written down where the reader looks.
3. **Fail closed.** Unclassified route → 500, unset run token → 401, missing
   required secret → compose refuses to start, non-allowlisted `secretRef` →
   `NotAllowed`.
4. **Upstream error text never reaches a response.** Classify
   (`classify_sqlx_error`, `StoreError` → `"database error"`) before
   surfacing.
5. **Measured, not assumed.** Budgets and version claims cite a
   `docs/plans/*-RESULT.md`; a falsified claim is corrected by name.

Comments and commit messages are in English. Going around a choke point
(`apiFetch`, `POLICY_TABLE`, `ApiJson`, `_assert_or_create_schema`, the
allowlisted resolver) is allowed only with a comment at the site saying why.
Known gap: `ApiError::Internal(err.to_string())` leaks upstream text in
fourteen Phase-1 handlers — do not add another.

## Vocabulary you will see and must use correctly

`P0–P6` phases · `G1–G4` gates · `R1–R11` risk register · `D1–D4` security
fixes (CHANGELOG) · `ADR NNNN` · `P5 review fix (BLOCKER|SHOULD-FIX)` /
`PR #NN review` provenance tags at a fix site.

## Non-negotiables by language

- **Rust:** no `unwrap`/`expect` outside `#[cfg(test)]`; `?` / `let … else` /
  `ok_or_else`; every `Result` fn has `# Errors`; product names backticked
  (`doc_markdown` is enforced, never allowed); `#[allow(.., reason = "…")]`
  with a checkable reason; secrets in `SecretValue` (no `Serialize`, no
  `Debug` leak, `.expose_secret()`); dialers use the allowlisted resolver and
  `resolve_checked`; handlers return `ApiResult<ApiJson<T>>`; every route in
  `POLICY_TABLE` and asserted both ways in `tests/route_auth.rs`; SQL values
  bound, `format!` only for constant identifiers; migrations `NNNN_*.sql`
  with a why-header, never edited once applied, targeted `UPDATE … AND col =
  '<old>'`; booleans parsed with `== "true"`.
- **TypeScript:** contracts → clients → `services/index.ts` → features →
  `app/` pages, one direction; features import `@/services` only; `type` for
  data, `interface` for `XxxService`; camelCase mirrors serde; `| string`
  widening; all fetches via `apiFetch`; `useService`/`useServiceAction` for
  state; kebab-case files, `"use client"` first line; no new mocks — remove
  consumers instead of optional-ing fields.
- **Python:** narrative module docstring, `from __future__ import annotations`,
  frozen dataclass `from_env()`, `requests` with `timeout=` +
  `raise_for_status()`, `Failure` for config/auth vs `skipped_verbs` for
  degraded, `_sql_string_literal` and server-controlled identifiers only,
  `EXPECTED_SCHEMAS` is the single schema owner, schedules
  `default_status=RUNNING` with the reason, test names are full sentences,
  no network in unit tests.
- **Compose/shell:** `${X:-}` for dev defaults, `${X:?}` for must-set, never
  a default token; unique YAML keys; `$$` escaping; `jq -n --arg` for JSON;
  init containers `restart: "no"`, idempotent, and polling for real
  readiness (Postgres restarts after `pg_isready` first succeeds); own proxy
  container to publish a port, never `network_mode: service:` + `ports:`.

## Rules learned the hard way (each one happened here)

1. Verify a claim against the signature before writing it in a comment.
2. Never weaken a test, lint, allowlist, `:?`, or threshold to get green; if
   a test is wrong, fix it and say why in the commit.
3. Never remove a security exception without proving the literal is gone
   from history.
4. Grep for the existing helper before writing one; duplicated guards are a
   finding.
5. No default credentials in compose or `.env.example`.
6. Tests must compile and pass against *this branch's* code, not a later
   branch's fields.
7. "Green" means you ran it, in the foreground, from a fresh build, and can
   quote the command and the counts. Otherwise write *not verified*.
8. Compose changes are proven by `docker compose up` on the affected profile
   from a clean project, not by `config`.
9. Resolve merge conflicts hunk by hunk; never `--theirs`/`--ours` a file
   both sides changed; recompile afterwards.
10. A fix lands on the branch that owns the code (stack: p0 → p1 → p2 → p3 →
    p4 → p5 → p6 → r1 → hardening-guards → gold-export → compose-fixes), then
    the branches above are rebased.
11. Never commit `__pycache__/`, `*.pyc`, `dagster/build/`, `*.egg-info/`,
    `rust/target/`. `git status` before every commit.
12. No secrets, real hostnames/IPs/ports, or client names in code, docs, or
    commit messages.
13. Cite the finding (`P5 review fix`, `PR #35 blocker 2`, `R10`, `D4`) at
    the fix site and in the commit body.
14. Prefer "unsupported, honestly" over "works, approximately."
15. Don't yield to wait on a background job you have to report on.

## Verification before any commit

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)
docker compose --profile '*' config --quiet   # plus a real `up` for compose edits
git status                                    # nothing generated staged
```

Build settings that matter on this machine: use the shared
`CARGO_TARGET_DIR` rather than a per-worktree `target/` (disk), and remember
`sqlx::test` embeds migrations at compile time — rebuild after editing one.
