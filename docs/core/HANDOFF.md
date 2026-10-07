# Handoff — planner and reviewer

For the Claude agent taking over the planner and reviewer role. Written
2026-10-03 by the outgoing planner; sections 3 to 6 and 10 updated
2026-10-07 by the planner that took over. Everything here was checked against the
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
| One spec per backlog task: target numbers against the best competitor, acceptance checklist | `docs/core/specs/` |
| One page per feature, with its acceptance checklist | `docs/core/features/` |
| Competitor matrices (BI, Data, Query Studio) and the owner's BI decisions | `docs/core/reference/competitive-comparison.md` |
| Parked reference material (security, support, release, competitors) | `docs/core/reference/` |
| Engineering plans, with developer handoffs and your reviews | `docs/superpowers/plans/` |

The owner asked for this to stay small. Do not add new document types.
`specs/` is the one exception, added at the owner's request on 2026-10-07:
one file per task, generated together so they share a format. A number a
spec marks *(proposed)* is the planner's, not a competitor's; the owner
signs it on the feature page before a plan uses it.
Several older documents contradict the build (`PRODUCT_SPECS.md`,
`docs/FEATURE_COVERAGE.md`, `docs/UX_FLOWS.md`, parts of `README.md`, the
sales playbook). Where they differ from `docs/core/PRODUCT.md` and the code,
trust the code. Fixing them is backlog `DOC-1`.

## 4. What has been merged

| PR | What |
| --- | --- |
| #60, #63, #64 | Gold publishing (`DATA-1`), slices A to D |
| #61 | Merge rule: a PR may merge when it adds no failing check |
| #62 | `SEC-8`: dependency security checks green |
| #65 | The first version of this handoff |
| #66 | Pipelines: chains load, cycles are refused |
| #67 | Login throttling and session cleanup (`SEC-2`, `SEC-5`), reviewed over three rounds |
| #68 | Dashboards: conversational Home, map charts, custom SQL in the chart builder |
| #70 | Assistant answers a very short message in the deployment's language |
| #71 | Connectors work, file upload (`DATA-9`), the time-travel picker |

Merged but **not accepted**, each waiting for the owner's checklist on its
feature page: `DATA-1`, `SEC-2` and `SEC-5` (with four unsigned defaults),
`DATA-9`. Do not mark any Done until the owner has run it.

Thirteen staging and CI commits (`7ec3a81` on 2026-10-05 to `ee0251e` on
2026-10-07) went straight to `main` without a pull request. One of them
added `.env.staging`, which nobody has checked (`SEC-13`). Section 10.

## 5. What is in flight

- **The product plan.** On 2026-10-05 to 10-07 the BI, Data and Query
  Studio modules were compared against their competitors under the owner's
  strict rule (Have = match or beat the best competitor). The owner decided
  BI area by area; Data is not yet decided area by area (`DEC-9`, `DEC-10`).
  Result: the matrices in `reference/competitive-comparison.md`, the task
  lists in `BACKLOG.md`, one spec per task in `specs/`, and roadmap pages
  published as private artifacts for the owner.
- **Assistant parity.** Every BI item has an assistant item (`AI-1` to
  `AI-15`) that ships with it (`AGENTS.md`). AI for other modules belongs to
  the AI team, which plans it; `AI-16` is the hand-off list.
- **Managed environments plan** (`docs/superpowers/plans/2026-10-07-managed-environments.md`):
  on hold by the owner; nothing of it is committed.
- No developer branch is open in our lane.

## 6. What comes after

Streams, which you do not plan or review unless asked:

| Stream | Who |
| --- | --- |
| Dashboards | The owner's friend |
| Pipelines | Another agent |
| Data module and file upload | Another friend |
| AI outside dashboards | The AI team |

Our lane is security, governance, monitoring and platform. Next, in order:

1. **Security plan.** Write one plan for the Now security items: `SEC-14`
   and `SEC-15` (high severity, connector code owned with the Data stream),
   `SEC-9` and `SEC-11` (shared with the AI team), `SEC-10`, and chase
   `SEC-13`. Their specs say what done looks like.
2. The owner's open decisions: `DEC-8`, `DEC-9`, `DEC-10`, the four login
   defaults, and the *(proposed)* numbers in each spec as its feature comes
   up (`PRODUCT.md` section 6).
3. Acceptance checklists with the owner: `DATA-1`, `SEC-2`/`SEC-5`,
   `DATA-9`.
4. Then `SEC-12`, `SEC-16` to `SEC-21`, `SEC-3` (single sign-on), `VER-1`
   (row filters), `GOV-1`, `SEC-6`, `OPS-8`, `OPS-1`, `OPS-7`.

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
| Branch protection on `main` | Not applied (backlog `REL-1`). The PR-only rule is held by agreement, and was broken by the thirteen direct pushes in section 4 |
| `.env.staging` on `main` | Added in `7ec3a81` without review (earlier notes said `df14b78`, which is not on `main`); contents not checked (`SEC-13`). The planner's attempt to read it was blocked as a credentials file |
| Leaked key in git history | Open (backlog `SEC-1`); the history scan is red on purpose |
| GitHub Dependabot | Reports many open alerts on the default branch; not triaged |
| SQL Server connection test after the client update in #62 | Not verified against a real server |
| Background merges causing extra published copies | Backlog `DATA-10` |
| `docs/core/reference/` | Still carries the old on-premises framing |
