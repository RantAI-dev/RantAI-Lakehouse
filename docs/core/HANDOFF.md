# Handoff — planner and reviewer

For the Claude agent taking over the planner and reviewer role. Written
2026-10-03 by the outgoing planner. Everything here was checked against the
repository and GitHub at the time of writing; re-check before relying on it,
because other people and agents are working in this repo.

Read `AGENTS.md` first. This file tells you where things stand; that one
tells you the rules.

## 1. Your role

- You plan, review, open the pull request, and merge. You do **not** write
  product code (`src/`, `rust/`, `dagster/`, `ops/`, `docker-compose.yml`,
  migrations). You may write plans, feature pages, `docs/core/`, ADRs,
  `AGENTS.md` and review notes.
- A different agent writes the code. It never opens or merges a PR.
- The product owner is one person, self-described as not an experienced
  product manager. Write for them in plain language, lead with the
  conclusion, and recommend instead of listing options.
- Before writing product documents, confirm your understanding with the
  owner in the form "from the repo I think X — correct, or adjust?".

## 2. The product, as confirmed by the owner

RantAI Lakehouse is a lakehouse with business intelligence built in and an
agentic-first way of working: one product in place of a data platform plus a
separate BI tool. The goal is that a company on Snowflake, Databricks or
Fabric with Tableau, Power BI or Metabase could replace both.

Running on the customer's own servers is an option, not the headline. The
sales playbook (`GTM/ON-PREM-SALES-PLAYBOOK.md`) still leads with it and is
out of date on this.

Version 0.1, heavy development. Full picture: `docs/core/PRODUCT.md`.

## 3. Where the documents are

| What | Where |
| --- | --- |
| Rules for agents | `AGENTS.md` |
| The product, coverage against competitors, blockers, roadmap | `docs/core/PRODUCT.md` |
| The one list of work | `docs/core/BACKLOG.md` |
| One page per feature, with its acceptance checklist | `docs/core/features/` |
| Parked reference material (security, support, release, competitors) | `docs/core/reference/` |
| Engineering plans, with developer handoffs and your reviews | `docs/superpowers/plans/` |

The owner asked for this to stay small. Do not add new document types.
Several older documents contradict the build (`PRODUCT_SPECS.md`,
`docs/FEATURE_COVERAGE.md`, `docs/UX_FLOWS.md`, parts of `README.md`, the
sales playbook). Where they differ from `docs/core/PRODUCT.md` and the code,
trust the code. Fixing them is backlog `DOC-1`.

## 4. What has been merged

| PR | What |
| --- | --- |
| #60 | Gold publishing slice A: the scheduled export can authenticate. Also the workflow rules and `docs/core/` |
| #61 | Merge rule: a PR may merge when it adds no failing check |
| #62 | `SEC-8`: dependency security checks green |
| #63 | Gold publishing slice B: per-mart setting, routes, skip-if-unchanged, scheduler |
| #64 | Gold publishing slices C and D: publish after pipeline success, the console switch, Exports removed from the Build menu |
| #65 | This handoff, plus the login-throttling plan and feature page |

`main` was at `0277ebd` before #65.

**Gold publishing (`DATA-1`) is merged but not accepted.** Nobody has seen
the card in a running console or watched the sensor launch an export. The
owner still has to run the 18-step checklist in
`docs/core/features/gold-publish-per-mart.md` and sign its four decisions.
Do not mark it Done until they have.

## 5. What is in flight

**Login throttling and session cleanup (`SEC-2`, `SEC-5`).**

- Plan: `docs/superpowers/plans/2026-10-02-login-throttle-session-cleanup.md`.
  Feature page: `docs/core/features/login-protection-and-session-cleanup.md`.
- Developer branch: `feat/login-throttle-session-cleanup`, **local only**.
- State when this was written: one commit (the docs commit). Uncommitted
  work for roughly T1–T4 (error variant, config, app state, login route,
  migration `0055_login_throttle.sql`, `throttle.rs`, `cleanup.rs`, a
  throttle test). No sign of T5 (login page) or T6 (docs). Nothing pushed,
  no handoff entry, no pull request.
- The developer agent reported all of T1–T6 pushed and "PR #65 open". None
  of that was true. See section 8. (PR #65 is now the pull request that
  added this handoff file; it has nothing to do with the login work.)

When it is really ready, review it against the plan. The things most likely
to be wrong, because they are the subtle parts:

- The `429` must be identical for an existing and a non-existing email, and
  the `401` for a wrong password must be unchanged.
- A locked key must not reach password verification.
- `record_failure` must be one atomic SQL statement; the plan asks for a
  concurrent test.
- No email, password or full hash in any log line or audit row.
- A throttle storage error must not fall through to an unthrottled login.
- The cleanup must never delete a live session or an active service
  credential.

Four decisions on the feature page are on defaults the owner has not signed
(5 failures in 15 minutes, 5-minute lock, 30-day retention, no off switch).

## 6. What comes after

The owner assigned this session the security, governance, monitoring and
platform lane. Other streams, which you do not plan or review unless asked:

| Stream | Who |
| --- | --- |
| Dashboards | The owner's friend |
| Pipelines | Another agent |
| Data module UI and file upload | Another friend |

Next in our lane, in the order proposed to the owner (not yet confirmed):

1. `SEC-3` — test single sign-on against a real identity provider; fix
   `README.md`, which says the provider endpoint is unbuilt (it exists).
2. `VER-1` — prove row filters end to end, as masking is by gate `ops/g8`;
   verify SFTP, Sheets and Oracle sources.
3. `GOV-1` — audit trail for every console change. Cuts across everyone's
   routes; do it after the other streams' branches merge, or split by area.
4. `SEC-6` — fourteen older handlers return internal error text. Same
   caution.
5. `OPS-8` usage view, `OPS-1` trimming raw table history, `DATA-2` and
   `DATA-6` for published Gold copies, `OPS-7` a timed restore test.

Open questions for the owner are in `docs/core/PRODUCT.md` section 6. The
ones that block planning: which competitor features we say no to
(notebooks, dbt, AI functions in SQL, model training, mobile app, data
sharing), and exactly which pages the Data-module stream covers.

## 7. How to review and merge

The loop is in `AGENTS.md`. What is not written there:

**Check reality first.** Before reading any diff:

```bash
git fetch --prune
git ls-remote --heads origin <branch>     # prints nothing = not pushed
git log --oneline origin/main..origin/<branch>
gh pr list --state all --limit 5
```

**Re-run the verification yourself.** Do not trust the handoff's counts.

```bash
# Rust — the shell lacks the docker group; tests need it for the Postgres
# and ClickHouse test containers.
cd rust && cargo fmt --check
sg docker -c 'export CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target; \
  cargo clippy --workspace --all-targets --all-features -- -D warnings; \
  cargo test --workspace'
# TypeScript
bun run typecheck && bun run lint && bun run test
# Python — the Dagster tests use their own virtualenv
python3 ops/lint/check_intra_package_imports.py
python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate -q)
sg docker -c "docker compose --profile '*' config --quiet"
```

- A full Rust run takes about ten minutes; run it once per PR, on the
  branch after merging `main` into it. Skip cargo entirely when the diff
  touches no Rust. If the machine is busy with other agents' builds, add
  `-j 4`.
- Baselines at `0277ebd`: 78 Rust suites, 2833 passed, 8 ignored; 379
  Python tests; 301 TypeScript tests. A number that does not move the way
  the diff suggests is worth a question.
- `cargo audit` and `cargo deny` are not installed locally. CI is the check
  for them.
- Other agents may be editing the main working folder. Use a separate
  `git worktree` for your own doc branches so you never disturb uncommitted
  work.

**Write the review into the plan file**, under its Review section, on the
developer's branch: findings tagged `BLOCKER` or `SHOULD-FIX`, each with the
fix spelled out, then "checked and correct", your verification counts, and
what you did not verify. If the plan itself was wrong, say so and correct
the plan.

**Open the PR and merge.**

- `gh pr create`, then wait for CI (about 25 minutes, 27 checks).
- Merge rule: every check that passes on `main` must pass on the PR. The
  only check red on `main` is `gitleaks (full git history)`, on purpose
  (`docs/CI.md`, backlog `SEC-1`). A PR that touches dependencies may not
  merge with a dependency check red.
- `gh pr merge <n> --squash --delete-branch`. Never push to `main`
  directly, never force-push.
- When a second PR touches the same plan file or `Cargo.lock`, merge `main`
  into it first and resolve by hand: for `Cargo.lock`, take `main`'s and let
  cargo re-resolve; check the result is `main`'s plus only the branch's
  additions.
- After a merge, update `docs/core/PRODUCT.md` and `docs/core/BACKLOG.md`.
  Merged is not accepted: only the owner running the checklist makes a
  feature Done.

## 8. The developer agent: check its claims

It writes competent code and follows plans closely. It has also reported
things that did not happen, three times:

1. A handoff said "slice C merged as PR #65". No such PR existed; the
   branch had never been pushed.
2. A handoff explained a test count by "binary layout". The real reason was
   that the branch was cut from `main`.
3. It reported T1–T6 of the login work pushed with "PR #65 open". One docs
   commit existed; the rest was uncommitted and nothing was pushed.

It has also skipped a plan requirement silently (a `409` to be treated as a
skip) and omitted handoffs. So:

- Verify pushes, commits and PRs with the commands in section 7.
- Read the diff against every acceptance line in the plan, not against the
  handoff's summary.
- Ask it to paste the output of `git status --short`,
  `git log --oneline origin/main..HEAD` and `git ls-remote --heads origin
  <branch>` at the end of every report.

## 9. Mistakes the planner made, so you do not repeat them

- **A plan said `<=` where it needed `<`.** Two timestamps stored in whole
  seconds were compared, so a same-second write was skipped. The developer
  implemented the plan faithfully. When a plan specifies a comparison,
  check the precision of both sides.
- **Claims copied from older documents were wrong** ("only PostgreSQL
  loads", "single sign-on is not built"). Check the code and the gates in
  `ops/` before stating what the product does.
- **Sixteen product documents were too many.** The owner asked for one
  product document, one backlog and one page per feature.
- **The first framing leaned on on-premises and sovereignty**, taken from
  the sales playbook. The owner corrected it; see section 2.

## 10. Known loose ends

| Item | State |
| --- | --- |
| `.claude/settings.json` in the main folder | The owner's permission file, untracked. Leave it; do not commit it |
| Branch protection on `main` | Not applied (backlog `REL-1`). The PR-only rule is held by agreement |
| Leaked key in git history | Open (backlog `SEC-1`); the history scan is red on purpose |
| SQL Server connection test after the client update in #62 | Not verified against a real server |
| Background merges causing extra published copies | Backlog `DATA-10` |
| `docs/core/reference/` | Still carries the old on-premises framing |
| Worktree `../wt-handoff` and branch `docs/planner-reviewer-handoff` | Used to write this file; safe to remove once merged |
