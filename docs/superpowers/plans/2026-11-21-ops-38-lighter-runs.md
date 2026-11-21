# `OPS-38`: lighter CI runs so the queue recovers

Planner and reviewer: Claude Opus. Developer: Gemini 4 Hydrogen (owner
2026-11-21: "minta antigravity kerjain", "gw butuh ci cepet"). Base: `main`.
Branch: `ci/ops-38-lighter-runs`. A CI-workflow PR: merged after its fast
checks (scope self-tests, repo lints, gitleaks), per the owner's rule.

## Why (measured 2026-11-20/21)

- A full run alone takes 24–27 min (the nightly schedule runs), so OPS-24 and
  OPS-25 still work.
- But at most 20–21 hosted jobs run at once (the org's cap), and on 11-20 there
  were 178 pull-request runs (48 on 11-19). Jobs waited 30–93 min for a runner
  and ran 10–28 min.
- Two causes inside the workflow:
  1. **Draft PRs run the full workflow.** Stacked and unfinished PRs (e.g. six
     stacked dashboard branches pushed together) take ~40 jobs each.
  2. **The gate map falls back to every gate.** Any `rust/crates/lakehouse-api`
     file without a `path` rule in `scripts/ci/gate_paths.txt` selects all nine
     groups (fail closed, OPS-25 D5). A change to `src/sql_lineage.rs` ran 39 jobs.

## Decisions (planner, owner-directed 2026-11-21)

- **D1. A draft PR runs only the fast checks.** On `pull_request` events whose
  PR is a draft: `Detect Change Scope`, `Repo lints`, `Rust · fmt`, the
  frontend job, gitleaks/security (separate workflow, unchanged) and
  `ci-required` run; every other job (Rust build, clippy, MSRV, tests, gates,
  images, Dagster) is skipped. `ci-required` passes on a draft when the fast
  checks pass and says in its log that the PR is a draft. Add
  `types: [opened, synchronize, reopened, ready_for_review]` to the
  `pull_request` trigger so marking a PR ready starts the full run. The
  `changes` job exports `draft` (`github.event.pull_request.draft`, compared to
  exactly `true`, fail closed: unknown → not a draft → full run).
- **D2. Narrow the gate map with evidence only.** Add `path` rules to
  `scripts/ci/gate_paths.txt` for `rust/crates/lakehouse-api/src/**` (and
  `lakehouse-store/src/**`) files ONLY where it is proven which gates can reach
  the code: a file maps to the groups whose gate scripts (`ops/g*`, the gate
  jobs' commands in `ci.yml`) call a route defined in that file, or to `none`
  when no gate calls any route of it and no gate-exercised module imports it.
  Record the evidence for each new rule as a comment line above it (the route
  paths grepped and the result). A file whose reach is unclear keeps the
  default (all groups). Start with the files PRs touched most (export,
  csv_export, csv_safe, sql_lineage, query, query_history, reports, dashboard,
  render, ai/*, alerts, pipelines compile/forms), and stop there.
- **D3. Nothing else changes**: no check dropped from a ready PR, no change to
  the push proof, the nightly full run or the kill switch `CI_PUSH_FULL`.

## Tasks

- **T1 (D1):** the trigger types, the `draft` output, the `if:` on every heavy
  job (one helper expression, not 30 copies, if the workflow allows: e.g. an
  output `heavy = draft != 'true'` from `changes`), and `ci-required` reading
  it (`scripts/ci/required_gate.sh` and its self-test
  `scripts/ci/tests/test_required_gate.sh`: a draft with skipped heavy jobs
  passes; a ready PR with a skipped heavy job still fails unless its group was
  not selected). Self-tests must fail on the old script.
- **T2 (D2):** the new `path` rules with evidence comments; add cases to
  `scripts/ci/tests/test_detect_change_scope.sh` for two of them (a file mapped
  to `none` selects no gate; a mapped file selects only its groups); run
  `python3 ops/lint/check_gate_groups.py` and its test.
- **T3:** `docs/CI.md` gains a short "Draft PRs" paragraph and the rule for
  adding a `path` line with evidence. `CHANGELOG.md` one line. Backlog row
  `OPS-38` (Done, this PR) and a Dates row.

## Verification

```bash
bash scripts/ci/tests/test_detect_change_scope.sh
bash scripts/ci/tests/test_required_gate.sh
bash scripts/ci/tests/test_detect_push_proof.sh
python3 ops/lint/check_gate_groups.py && python3 -m pytest ops/lint/test_check_gate_groups.py -q
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"
git status
```

No cargo, bun or docker is needed.

## Out of scope

Self-hosted runners, the merge queue, the flaky tests (`REL-27`, the login
selection test: a separate PR).

## Handoff

## Review
