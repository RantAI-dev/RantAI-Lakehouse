# OPS-24: a push to `main` stops repeating the pull request; CI runs wait less

Backlog `OPS-24` (the row is filed by the planner with the next bookkeeping; the ID was
free in the repository and in the task base on 2026-11-13). Planner: Claude Opus, 2026-11-13, at the
owner's request ("fix the workflow first so its way faster, for the pr that do this,
immediate merge so other pr immediately get the new treatment"). Developer: a different
agent. Anchors verified at `main` `9d3443cc5` (`origin/main` moved to `2306e7302` while this
was written; `git diff 9d3443cc5 2306e7302 -- .github scripts/ci docs/CI.md rust/.config` is
empty). Branch `ci/faster-runs` off `origin/main`. No Rust, TypeScript or Python product
source is touched.

Two slices. **S1** (this pull request, tasks T1 to T7) is built, reviewed and merged today.
**S2** (compile the tests once, `cargo nextest archive`) is planned in full at the end and
follows as its own pull request, because the numbers below say it is worth 4 to 6 percent
of a pull request's runner time, rests on a size nobody has measured, and must not delay
S1, which removes about three quarters of what `main` costs. Neither slice depends on the
self-hosted worker.

No test, lint, gate, allowlist or threshold is removed or loosened for a pull request
(AGENTS.md rule 2). No nextest `retries`.

## 0. Measured (2026-11-13, `gh api`, read-only)

### 0.1 One code pull-request run (run 43333004203, attempt 1, Rust scope)

| Part | Jobs | Running, job-min | Share |
|---|---|---|---|
| Acceptance gates and `image-smoke` | 28 | 156.5 | 62% |
| `Rust · test N/3` | 3 | 71.1 (1,461 + 1,272 + 1,531 s) | 28% |
| `images` | 1 | 11.2 (671 s; the API image step is 592 s) | 4% |
| `fmt`, `clippy`, `build`, `msrv` | 4 | 8.8 (35, 184, 148, 161 s) | 3.5% |
| Frontend, lints, `changes`, `ci-required` | 4 | 5.7 | 2% |
| **Total** | 40 ran of 42 | **253** | |

Queued: 415 job-minutes. Wall: 48.6 min. The four small Rust jobs had all finished 476 s
after the run was created. The last jobs to finish were gates that waited up to 1,444 s for
a runner. The organisation's cap (about 20 jobs at once, all runs) is what a run waits for.

Inside the three test jobs (same run, logs of jobs 131582927335, …372, …319):

| | 1/3 | 2/3 | 3/3 | Sum |
|---|---|---|---|---|
| "Free runner disk" (first of two steps) | 142 s | 67 s | 139 s | 348 s |
| Compile (`Finished test profile … in`) | 7m42s | 8m46s | 9m53s | 1,581 s |
| Tests (`Summary [ … s]`) | 593 s | 859 s | 645 s | 2,097 s |
| Tests run | 2,582 | 2,519 | 2,451 | 7,552 |

The cache restored was an exact hit (`Restored from cache key
"v0-rust-stable-test-Linux-x64-d53df738-4681ab64" full match: true`, 691 MB), so the 8 to 10
minutes of compiling are the workspace's own crates and 188 test binaries:
`Swatinem/rust-cache` stores dependencies only. The runner's disk was `145G, 59G used, 86G
free` BEFORE "Free runner disk" and `110G` free after it: OPS-12's "about 14 GB free" is no
longer true of the runner image.

### 0.2 Pushes to `main`

| Fact | Number | Source |
|---|---|---|
| A push run | 42 jobs, 250 / 272 / 276 job-min running, 450 to 845 queued | runs 43349099725, 43342480860, 43324228296 |
| Wall time of a push run | 61 to 211 min | `gh run list --workflow CI --branch main --event push`, 2026-11-12 |
| Push runs that never started | 5 of the last 30 (`total_count: 0` jobs, `cancelled`) | 43331837220, 43343033876, 43274677702 and two more |
| Pushes on 2026-11-12 | 22: 7 documents-only, 3 console-only, 12 with Rust, Dagster or stack files | `git log --first-parent`, classified with the scope rules |
| Pushes on 2026-11-11 | 19: 7 documents-only, 3 console-only, 9 heavy | same |
| Merge commit or single parent | since 2026-11-10: 19 merge commits, **33 single-parent commits** (squash merges); all 9 heavy pushes of 2026-11-11 were squashes | `git log --first-parent --format=%p` |
| Coverage on a push | 44 to 55 job-min (`cargo llvm-cov` 2,650 / 2,677 / 3,316 s), also for a documents-only merge | runs 43357924019, 43349099644, 43332933916; documents-only push `eaebacf28` started Coverage run 43343033807 (67 min wall) |
| A documents-only pull request | 3 jobs, 41 s running, 164 s wall | run 43342602185 |
| Pull-request CI runs on 2026-11-12 | 205 (167 cancelled by a newer push, 37 green); peak 23 in one hour; 26 branches | `gh run list --event pull_request` |
| Actions cache | 10.18 GB of a 10 GB cap, 172 entries | `actions/cache/usage` |

The five pushes that never ran explain themselves: `ci.yml:24-27` puts every push to `main`
in one concurrency group with `cancel-in-progress: false`. One run holds the group for an
hour or more, the next waits, and a third push replaces the waiting one. The comment there
("F4: a push to main must finish its run; cancelling it made its ci-required read as
failed") is not achieved for those pushes today.

`main` therefore costs about 18 runs × 265 = 4,800 job-minutes a day of CI plus about 1,000
of Coverage, from the same 20 slots the pull requests wait for.

### 0.3 What reads `main`'s CI run

Nothing automatic. Branch protection (read from the API: `strict: true`, contexts
`ci-required`, `Repo lints`, `cargo audit (advisories)`, `cargo deny check (all)`,
`gitleaks (working tree)`) is evaluated on a pull request's head. `deploy-staging.yml:3-7`
starts on the push and does not wait for CI. `release.yml` starts on a tag and reads no
check. No workflow uses `workflow_run`; nothing but `coverage.yml` names the artifacts
`lcov-report` and `sbom-cyclonedx`. What `main`'s run is good for is (a) the caches pull
requests read, and (b) a person reading "is this red on `main` too" (AGENTS.md step 6).

### 0.4 The proof, tried by hand against real pushes

| Push | Kind | Head it is compared with | Trees equal | First parent is an ancestor of the head | `ci-required` on the head | Result with the filter of T3 |
|---|---|---|---|---|---|---|
| `43d36f55b` (#486) | merge commit | `7e098119e` (`HEAD^2`) | yes | yes | success | `PROVEN` |
| `9d3443cc5` (#500) | squash | `22c641fe6` (from the API, fetched as `refs/pull/500/head`) | yes | yes | success | `PROVEN` |
| `2306e7302` (#499) | squash, console only | `a8916d71f` | yes | yes | success, Rust jobs skipped | not proven for a Rust scope (correct: nothing heavy to skip) |
| `eaebacf28` (#498) | squash, documents only | `b334584e7` | yes | yes | success, gates skipped | not proven for an acceptance scope (correct) |
| head `e431ff500` | a head whose run failed | | | | failure | not proven |

## 1. Decisions

**D1 (P1a, changes an earlier decision). A push follows the change scope.**
CI-SPEED-PLAN §4.1 says "On `push` to `main`, run everything regardless of scope" and §9
lists "every job runs on `main`" as the guard against "A path rule wrongly skips a job".
Changed because 7 of 22 pushes a day are documents-only and each costs 250 to 276
job-minutes for a tree the scope rules already let a pull request skip. The guard is kept
in another form: D3. One scope gap is closed first, because the every-push backstop is
what covered it: `.env.example` is compiled into `lakehousectl`
(`rust/crates/lakehousectl/src/secrets.rs:66`, `include_str!`) but sets only `stack`
(`detect_change_scope.sh:146-148`); it now sets `rust` too.

**D2 (keeps F4, and makes it true). Every run that is not a pull request's has its own
concurrency group.** With D1 a push covers only its own diff, so a push whose run is
replaced while waiting (5 of 30 today) would never be covered by a later run. Group key
becomes `github.run_id` for non-pull-request events: no push run waits for another, none is
replaced. Pull requests keep `github.ref` and `cancel-in-progress: true`. Parallel runs on
`main` may write the same cache key; the second write is refused by the cache service and
both tools tolerate that.

**D3 (added; keeps §9's guard). One full run a night, and on demand.** `ci.yml` gains
`schedule` (19:23 UTC, an hour with 0 pull-request runs on 2026-11-12) and
`workflow_dispatch`. Neither has a base commit, so the scope script's existing rule "Base
commit missing or unreadable: forcing all=true" runs every job. Cost: about 265 job-minutes
a night instead of 4,800 a day. It is also the answer to "is it red on `main` too". Removing
it is deleting four lines; it is an open question for the owner (Q2).

**D4 (P1b, refined with evidence). "Proven redundant" means all of the following; anything
else runs everything in scope, as today (fail closed).** For the pushed commit `H`:
1. the event is `push`, and the push's scope needs Rust tests or acceptance jobs;
2. a pull-request head `P` is found: `H^2` when `H` has two parents; when `H` has one
   parent, the head of the one merged pull request whose `merge_commit_sha` is `H` and whose
   base is `main` (asked from the API), fetched as `refs/pull/<n>/head` and checked to be
   that exact commit. **The planner's rule covered merge commits only; 33 of the last 52
   pushes are squashes (§0.2), so without this the rule would miss most pushes;**
3. `H`'s first parent is an ancestor of `P` (the branch held `main`'s tip when merged). This
   is what makes "the pull-request run tested `P`'s tree" true: the run tested the merge of
   `P` with `main` as it was then, and `main` only moves forward, so that merge was `P`;
4. `H^{tree}` equals `P^{tree}`;
5. the most recently completed check run named `ci-required` on `P`, made by GitHub Actions,
   is `success`, and none is still running;
6. in that same check suite, the job that stands for each needed group succeeded (was not
   skipped): `Rust · fmt` when the push needs the Rust group, `Images · build lakehouse-api
   + dagster once` when it needs the acceptance group. `required_gate.sh` makes a group
   all-or-nothing, so one job of the group plus a green `ci-required` proves the group. This
   closes the case where the pull-request run had a narrower scope than the push.

Known limit, accepted: a pull request whose base was changed to `main` after its last run
carries a `ci-required` from the other base. Branch protection accepts that check today
too; D3's nightly run is the backstop.

**D5. What a proven push still runs, and what it skips.** Still runs: `changes`,
`repo-lints`, the console and Dagster unit jobs when in scope, `fmt`, `clippy`, `build`,
`msrv` (8.8 job-min together; `clippy`, `build` and `msrv` write `rust-cache` entries pull
requests read, `ci.yml:346,370,557`), `images` (the only writer of the image layer cache,
`ci.yml:737,747`), and ONE leg of `test` in its cheapest form: it restores the cache, and
compiles (`cargo nextest run --no-run`) only when the restore was not an exact hit, because
`rust-cache` does not save on an exact hit anyway (its `cache-hit` output; read in
`action.yml` of `v2`). Skipped: the nextest runs, the doctest step, the count check, the 25
gate jobs and `image-smoke`.

**D6. `ci-required` after the change.** A job that failed or was cancelled is red whatever
the scope or the proof (`required_gate.sh:83-87`, unchanged). "skipped" is accepted only as
below.

| Jobs | "skipped" is accepted when |
|---|---|
| `changes`, `repo-lints` | never |
| `verify` | neither `frontend` nor `all` |
| `fmt`, `clippy`, `build`, `msrv`, `test` | neither `rust` nor `all` (a proven push does not excuse them) |
| `dagster-unit-tests`, `dagster-loader-tests` | neither `dagster` nor `all` |
| `images` | none of `rust`, `dagster`, `stack`, `all` (a proven push does not excuse it) |
| the 25 gate jobs and `image-smoke` | none of `rust`, `dagster`, `stack`, `all`; **or** the event is `push` and `PUSH_PROVEN` is `true` |

| Result | Required | Not required |
|---|---|---|
| `success` | pass | pass |
| `skipped` (or empty) | **fail** | pass |
| `failure`, `cancelled` | **fail** | **fail** |
| anything else | **fail** | **fail** |

`PUSH_PROVEN=true` on any event other than `push` is ignored. The event no longer makes
anything required by itself (D1).

**D7. Permissions.** `changes` alone gets `contents: read`, `checks: read`,
`pull-requests: read` (a job-level block replaces the workflow's, so `contents: read` is
repeated). All three are read scopes, which is all a fork's pull request can have, and the
proof script returns before any API call unless the event is `push`. No secret is used. The
script receives the commit, the repository and the event through `env:`, checks that every
SHA is 40 hex digits and every pull-request number is digits, and never evaluates text from
the API.

**D8. A kill switch that is not a pull request.** Repository variable `CI_PUSH_FULL=true`
makes a push force `all=true` and ignores the proof: yesterday's behaviour, from the next
push, set by the planner or owner in the repository settings. Unset is the new behaviour.

**D9 (P2, changed: slice S2, not today).** Compiling once saves two of three compiles
(about 1,054 s) and the partitions' disk and cache steps, and costs one archive upload and
one download and extraction per partition. With three partitions that is an estimated 10 to
15 job-minutes of 253 (4 to 6%), and the tests then wait for a second runner after the
build job. The estimate rests on transfer speeds and an archive size that are not measured:
a local `lakehouse-api` integration test binary with full debug information is 435 MB, and
there are 119 of them; with `line-tables-only` the size is unknown. T5 prints the real size
from CI so S2 starts from a number. S2 is planned in full in section 2b, on GitHub-hosted
runners only.

**D10 (P3, revised twice on the planner's new facts).** The CI worker is one machine of
about the size of ONE hosted runner, with one runner registration, that can be asleep.
- **No job a pull request requires goes to it by default, and this machine should not get
  one.** Hosted: the four small Rust jobs of five pull requests run in parallel and were
  all done 8 minutes after the run was created (§0.1). Worker: one job at a time. A run's
  four jobs are 5 + 147 + 101 + 126 = 379 s of compiling on 4 vCPUs plus about 20 s per job,
  so about 7.7 min per run when the top crate changed; five runs in series end after 38
  min. At the measured peak (23 runs an hour, one every 157 s) the worker would be asked
  for 459 s of work every 157 s: its queue only grows. And a required job waiting for a
  sleeping machine blocks every pull request for up to 24 hours. Where a worker beats a
  hosted runner: only through the persistent target directory (no cache restore, no disk
  step, an unchanged crate is not rebuilt), and only when the job does not have to queue.
- **The mechanism is kept**, because it is cheap and a larger machine may come: a switch
  variable per job group, default hosted, with the labels fixed in the workflow.
- **Minimum machine for required jobs** (reasoning: keep utilisation under 0.7 at the
  measured peak, and finish a burst of five runs within today's 8 minutes; work per run on
  `n` threads is about `1,895 / n + 20` s with four instances): **24 hardware threads or
  more, 48 GB RAM or more (12 GB per instance; a hosted runner gives a job 16), four runner
  instances, always on.** This laptop (8 to 12 threads, 8 to 16 GB usable, one instance,
  can sleep) is below it.
- **Staging is excluded by construction, and again at run time.** The variable is a switch
  (`true`), not a label list; the labels `[self-hosted, lakehouse-ci]` are fixed in the
  repository (a constant of `select_runner.sh`, a literal in `coverage.yml`), and a job is
  only offered to a runner that has every label. The staging runner
  has `self-hosted, Linux, X64, staging` (read from the API), so no variable value can
  reach it. Second line: the first step of every job that can run self-hosted fails unless
  the runner's environment holds `LAKEHOUSE_CI_WORKER=1`, which the owner puts in the CI
  worker's `.env` only. It runs BEFORE `actions/checkout`, whose `git clean -ffdx` would
  delete the `.env` the staging deploy writes into its workspace
  (`deploy-staging.yml:42-64`).
- **A fork's pull request always gets `ubuntu-latest`**: the choice is made by
  `scripts/ci/select_runner.sh` from the event and `pull_request.head.repo.full_name`, with
  a self-test. Honest limit: this protects an unmodified workflow. A fork can edit the
  workflow in its own pull request; only the repository setting in the owner's checklist
  stops that. GitHub's documentation does not say whether `vars` reaches a fork's run (the
  contexts page and the variables page were read; neither mentions forks), so the script
  does not rely on it.
- **The test partitions stay on hosted runners in S1.** Tests share one labelled Postgres
  with `max_connections=200`, and the crate documents a worst case of 16 connections per
  test thread (`lakehouse-test-support/src/lib.rs:258-266`, `:286-290`); several test jobs
  on one Docker daemon share that budget. The partition count and the place the partitions
  run become variables in S2, where a partition no longer compiles.

**D11 (new: what this worker can take).** Work that only runs on `main` or by hand, that
nothing waits for, and that never runs a pull request's code:

| Candidate | Read by anything? | Needs | Verdict |
|---|---|---|---|
| `coverage.yml`, job `coverage` | No (§0.3). Not a required check | Docker (testcontainers), 44 to 55 min on 4 vCPU, memory of a full instrumented test build | **Qualifies.** After D1 and T2 it is half of what `main` still costs on hosted runners. Behind `CI_COVERAGE_ON_WORKER`, default hosted |
| `coverage.yml`, job `sbom` | No | 34 s | Stays hosted: nothing to gain |
| `catalog-search-bench.yml` | Its numbers go into a RESULT document as "on a GitHub runner" | a stack, up to 180 min | No: it is a measurement, a different machine changes the numbers; 4 runs ever |
| `g9-upload-large.yml` | Same (`DATA-17-RESULT.md`) | reads container memory, 2 GiB files | No: same reason; last run 2026-10-31 |
| `security-all-branches.yml` | People | minutes, 03:17 UTC | No: about 2 job-minutes a day, off peak |
| The nightly run of D3 | People | 42 jobs | No: gates collide on one machine; the night has free hosted runners |

When the worker is offline a Coverage run waits; `coverage.yml:10-13` allows one running
and one waiting run and a newer push replaces the waiting one, so nothing piles up; GitHub
fails a job no runner took within 24 hours. Nothing waits for it, so that is acceptable.
`coverage.yml` has no `pull_request` trigger, so only `main`'s code reaches the worker
through it.

**D12 (added). Coverage does not start for a push that cannot change it.** `coverage.yml`
gets a `paths:` filter (the files the Rust tests read). CI-SPEED-PLAN §4.1 forbids
workflow-level path filters because "A filtered workflow reports nothing, which leaves a
required check pending forever"; Coverage is not a required check, so the reason does not
apply.

**D13 (P5).** In S1: the Rust test job pulls its three test images first, with a retry
around the PULL only; the step cannot fail the job and no test is retried. The Docker Hub
`502` on `axllent/mailpit` was reported by the planner; its run id was not given to me (not
verified by me). A retry or mirror for the gates' pulls is out of scope (26 jobs, each with
its own flags).

**D14 (P4).** Every job a code pull request runs today still runs on it, on the same
runner unless the owner sets a variable. No new action is used, so no pin is added; the
existing pins and tags are left as they are. If a new action seems necessary, stop and
report.

### What a run costs today and after

"After" numbers are estimates: step times of run 43333004203 added up for the jobs D5
keeps, and 2026-11-12's traffic.

| Event | Today: jobs / job-min | After S1: jobs / job-min | Rests on |
|---|---|---|---|
| Code pull request | 42 (40 run) / 253; path: `changes` 8 s → `images` 671 s → slowest gate 863 s, or a test partition 1,531 s; wall 48.6 min with 415 job-min queued | 42 / 253 plus under 1 for self-tests and the pre-pull; **same path**; less waiting, not put in a number | S1 does not make a pull request's own run cheaper. It removes what competes for the 20 slots. The first day's `gh run list` gives the new wall time |
| Documents-only pull request | 3 / 0.7 | 3 / 0.7 | unchanged |
| Code push, proven | 42 / 250 to 276, plus Coverage 44 to 55 | 9 (10 with the console job, 11 or 12 with Dagster) / about 24 when the test cache is current, about 33 when not; Coverage unchanged on hosted, 0 hosted when on the worker | 8 + 35 + 35 + 184 + 148 + 161 + 671 + about 216 (+527) + 9 s |
| Code push, not proven | same | as today for the jobs in scope | fail closed |
| Documents-only push | 42 / 250 to 276, plus Coverage 44 to 55 | 3 / 0.7; Coverage does not start | run 43342602185 |
| Nightly | none | 42 / about 265 | D3 |
| A day like 2026-11-12 | about 5,800 (18 runs that ran, 20 Coverage runs) | about 1,220; about 620 with Coverage on the worker | 7 × 0.7 + 3 × 6 + 12 × 28 + 265 + 12 × 50 |

### How each change is proven

| Task | Before the merge (this pull request's own run, or locally) | Only after the merge |
|---|---|---|
| T1 | Self-tests in `Detect Change Scope` and `ci-required` (push cases without a push). The run itself is a pull request with `all=true`: all 42 jobs run with the shortened `if:` | First documents-only merge: 3 jobs, `ci-required` green. First code merge: no job outside its scope |
| T2 | `yaml.safe_load` and review | First documents-only merge starts no Coverage run |
| T3 | Self-test (22 cases) in `Detect Change Scope`; the step prints `proven=false reason=not a push event`; the dry run of §3 against four real pushes | The merge of this pull request is itself the first proven push: see §3b |
| T4 | Gate self-test; on the pull request the matrix has three legs, all gates run, the count check prints its sum | §3b |
| T5 | Each partition's log shows the three pulls and partition 1 prints `test binaries: count=… total_gib=…` | nothing |
| T6 | Self-tests; `compile_runner=["ubuntu-latest"]` in the log; the four jobs show `Hosted Compute Agent` and `full match: true`. The worker path is NOT run by this pull request | Only if a variable is set (§3c) |
| T7 | `yaml.safe_load`, the lint of T6, review | The planner dispatches Coverage by hand with the variable set (§3c) |

## 2. Tasks (S1)

One task per commit, in this order. T1 alone can be merged; so can T1 to T2, T1 to T4, and
so on: no task needs a later one. T4 needs T1 and T3. T7 needs T6. Every commit message
starts `ci(ci-fast):` or `docs(ci-fast):`, and its body names `OPS-24` and the decision
(`D1`, …). Every edited site carries a short comment that says why and names `OPS-24`
(rule 13). Push after every commit. Do not open a pull request.

Do the scripts first in each task, run their self-tests, then edit the workflow.

### T1. A push follows the scope; every push has its own run; a full run at night (D1, D2, D3, D8)

Anchors: `.github/workflows/ci.yml:3-11` (`on:`), `:24-27` (`concurrency:`), `:59-64` (step
"Detect change scope"), 37 lines that contain `github.event_name == 'push' ||`;
`scripts/ci/detect_change_scope.sh:8-10,19-33,106-107`;
`scripts/ci/tests/test_detect_change_scope.sh:76-102,175,193-209,300-315`;
`scripts/ci/required_gate.sh:10,109-112`;
`scripts/ci/tests/test_required_gate.sh:142,181,189`; `docs/CI.md:43`.

1. `detect_change_scope.sh`:
   - Line 20: the condition becomes `if [ "$EVENT" = "push" ] && [ "${FORCE_ALL:-}" = "true" ]; then`.
     Line 19's comment says: a push follows the scope like a pull request (`OPS-24` D1);
     `FORCE_ALL=true` (repository variable `CI_PUSH_FULL`) is the switch back to "a push
     runs everything". Line 21's message becomes `Push event with CI_PUSH_FULL=true: forcing all=true.`
     Nothing else in that block changes.
   - Header (lines 8-10): add `FORCE_ALL` and say `EVENT_NAME` may be `push`,
     `pull_request`, `schedule` or `workflow_dispatch`, and that the last two have no base
     and therefore force `all=true`.
   - Lines 106-107: add `|| [[ "$file" == ".env.example" ]]` to the Rust condition, with a
     comment naming `rust/crates/lakehousectl/src/secrets.rs:66`. The `stack` rule for the
     file stays.
2. `test_detect_change_scope.sh`:
   - In `run_case` (line 94) the event becomes `EVENT_NAME="${CASE_EVENT:-pull_request}"`
     and one line is added below it: `FORCE_ALL="${CASE_FORCE_ALL:-}" \`.
   - Line 175 becomes
     `assert_case ".env.example sets rust and stack" "true" "false" "false" "true" "false" "false" ".env.example"`.
     This expectation is made wider, not weaker: say so in the commit body, with the
     `include_str!` anchor.
   - Replace the block "Push event test" (lines 193-209) with these cases (argument order
     is rust, frontend, dagster, stack, docs_only, all):
     - `CASE_EVENT=push assert_case "a documents-only push is documents-only" "false" "false" "false" "false" "true" "false" "docs/test.md"`
     - `CASE_EVENT=push assert_case "a Rust push sets rust only" "true" "false" "false" "false" "false" "false" "rust/crates/lib.rs"`
     - `CASE_EVENT=push assert_case "a workflow push forces all" "true" "true" "true" "true" "false" "true" ".github/workflows/ci.yml"`
     - `CASE_EVENT=push CASE_FORCE_ALL=true assert_case "a documents-only push with CI_PUSH_FULL=true forces all" "true" "true" "true" "true" "false" "true" "docs/test.md"`
     - `CASE_FORCE_ALL=true assert_case "CI_PUSH_FULL does not widen a pull request" "false" "false" "false" "false" "true" "false" "docs/test.md"`
   - After the block "Unreadable base commit test" (line 300) add two blocks of the same
     shape: `EVENT_NAME="push"` with `BASE_SHA` forty zeros must give `all=true` ("a push
     with an unreadable base forces all=true"); `EVENT_NAME="schedule"` with `BASE_SHA=""`
     must give `all=true` ("a scheduled run forces all=true").
3. `required_gate.sh`: line 110 becomes `if [ "$scope_all" = "true" ]; then`, with a comment
   (`OPS-24` D1: the event no longer requires anything by itself; the scope does). Line 10
   lists the four events.
4. `test_required_gate.sh`:
   - Line 181 becomes `expect 1 "Skipped Rust test on a Rust push blocks gate" "${base_push_green[@]}" SCOPE_RUST=true RUST_TEST_RESULT=skipped`.
   - Line 189 becomes `expect 1 "Skipped image smoke test on a Rust push blocks gate" "${base_push_green[@]}" SCOPE_RUST=true IMAGE_SMOKE_RESULT=skipped`.
   - Add below them:
     - `expect 0 "Documents-only push with only the lints passes gate" "${base_docs_green[@]}" EVENT_NAME=push`
     - `expect 1 "Failed job on a documents-only push blocks gate" "${base_docs_green[@]}" EVENT_NAME=push RUST_TEST_RESULT=failure`
     - `expect 1 "Push that forces all with a skipped gate blocks gate" "${base_push_green[@]}" SCOPE_ALL=true G1_RUSTFS_RESULT=skipped`
     - `expect 0 "Scheduled run with everything green passes gate" "${base_all_green[@]}" EVENT_NAME=schedule`
     - `expect 1 "Scheduled run with a skipped gate blocks gate" "${base_all_green[@]}" EVENT_NAME=schedule G4_CDC_RESULT=skipped`
   - The two changed lines are not weakened: the old ones asserted a rule this plan
     replaces (D1); the new ones assert the same refusal under the new rule. Say so in the
     commit body.
5. `ci.yml`:
   - Under `on:` after the `pull_request:` block add `schedule:` with one entry
     `- cron: '23 19 * * *'` and `workflow_dispatch:`, with a comment: `OPS-24` D3, the full
     run that replaces "every push runs everything"; neither event has a base commit, so
     the scope script forces `all=true`.
   - `concurrency.group` becomes
     `ci-${{ github.workflow }}-${{ github.event_name == 'pull_request' && github.ref || github.run_id }}`.
     Keep the F4 line and add: `OPS-24` D2, a run that is not a pull request's has its own
     group, because one group for all pushes left 5 of 30 push runs never started.
     `cancel-in-progress` is unchanged.
   - Step "Detect change scope": add `FORCE_ALL: ${{ vars.CI_PUSH_FULL }}` to its `env:`.
   - Delete the exact text `github.event_name == 'push' || ` (with its trailing space)
     everywhere it occurs: 37 places (35 job `if:` lines and the two step `if:` lines at
     3963 and 3971). Lines 737 and 747 contain `github.event_name == 'push' &&` and must
     NOT change.
6. `docs/CI.md`: replace line 43 with:
   "A `push` to `main` follows the same scopes as a pull request (`OPS-24` D1): the diff
   is the push's own (`github.event.before` to the pushed commit). Every push has its own
   run and none waits for another (D2). One full run starts every night at 19:23 UTC and
   can be started by hand (`workflow_dispatch`): it has no base commit, so every job runs
   (D3). Setting the repository variable `CI_PUSH_FULL` to `true` makes every push run
   everything again."
   In the scope table (line 36) add `.env.example` to the `rust` row.

Check: `bash scripts/ci/tests/test_detect_change_scope.sh` and
`bash scripts/ci/tests/test_required_gate.sh` end with `0 failed`;
`grep -c "github.event_name == 'push' ||" .github/workflows/ci.yml` prints `0`;
`grep -c "github.event_name == 'push' &&" .github/workflows/ci.yml` prints `2`.

### T2. Coverage starts only when a file the Rust tests read changed (D12)

Anchor: `.github/workflows/coverage.yml:3-8`.

Under `push:` add, after `branches: [main]`:

```yaml
    paths:
      - "rust/**"
      - "src/lib/dashboard-specs.ts"
      - "docker-compose.yml"
      - ".env.example"
      - "ops/fixtures/**"
      - ".github/workflows/coverage.yml"
```

Rewrite the comment above `on:` to say: these are the `rust` scope of
`scripts/ci/detect_change_scope.sh` plus the fixtures the Rust tests read; a path filter is
safe here because no required check comes from this workflow (`OPS-24` D12).
`workflow_dispatch` stays. In `docs/CI.md:21` change "Runs on `push` to `main` and manual
dispatch only" to "Runs on a `push` to `main` that changes a file the Rust tests read, and
on manual dispatch,".

Check: the syntax check of §3; `git diff --stat` shows two files.

### T3. The proof script, its self-test, and its outputs on `changes` (D4, D7, D8)

New files: `scripts/ci/detect_push_proof.sh` and
`scripts/ci/tests/test_detect_push_proof.sh`, both mode 755
(`git update-index --chmod=+x`). Anchors: `ci.yml:37-64`, `docs/CI.md:59-61`.

**The script.** `set -euo pipefail`. Header comment in the style of
`detect_change_scope.sh`: what it decides, its inputs and outputs, and that it always exits
0 because "not proven" is an answer, not an error.

Inputs (environment): `EVENT_NAME`, `PUSH_SHA`, `REPO` (`owner/name`), `BASE_BRANCH`
(default `main`), `SCOPE_RUST`, `SCOPE_DAGSTER`, `SCOPE_STACK`, `SCOPE_ALL`, `GH_TOKEN`
(read by `gh`), `GITHUB_OUTPUT` (optional).

Two constants at the top, with a comment that they must equal the `name:` of the jobs `fmt`
and `images` in `ci.yml` and that the self-test checks it:
`RUST_SENTINEL="Rust · fmt"` and
`ACCEPTANCE_SENTINEL="Images · build lakehouse-api + dagster once"`.

One function `finish <true|false> <head sha or empty> <pr number or empty> <reason>` prints
the four values and, when `GITHUB_OUTPUT` is set, appends `proven=`, `proven_by=`,
`proven_pr=`, `reason=` to it, then `exit 0`. Every "not proven" below is
`finish false "" "" "<reason>"`. Every command that can fail is written
`if ! x="$(…)"; then finish false … ; fi`.

Steps, in this order:
1. `EVENT_NAME` is not `push`: not proven, reason `not a push event`. No `git`, no `gh`.
2. `need_rust` is `true` when `SCOPE_RUST` or `SCOPE_ALL` is `true`. `need_acc` is `true`
   when any of `SCOPE_RUST`, `SCOPE_DAGSTER`, `SCOPE_STACK`, `SCOPE_ALL` is `true`. Both
   false: not proven, reason `no Rust test or acceptance job in this push's scope`. No `gh`.
3. `PUSH_SHA` must match `^[0-9a-f]{40}$` and `git cat-file -e "$PUSH_SHA^{commit}"` must
   succeed; `REPO` must match `^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$`. Otherwise not proven.
4. `git rev-list --parents -n 1 "$PUSH_SHA"` gives the commit and its parents.
   - Two parents: `first` is the first, `P` is the second, the pull-request number is empty.
   - One parent: `first` is it. `gh api "repos/$REPO/commits/$PUSH_SHA/pulls"`; with `jq`
     keep the entries where `.merge_commit_sha == $PUSH_SHA`, `.merged_at != null` and
     `.base.ref == $BASE_BRANCH`. Exactly one must remain; `N` is its `.number` (digits
     only) and `P` its `.head.sha` (40 hex). Then
     `git fetch --quiet origin "refs/pull/$N/head"` and
     `git rev-parse FETCH_HEAD` must equal `P`. Any failure: not proven, with a reason that
     says which step (`no merged pull request has this commit as its merge commit`, `the
     fetched pull-request head is not the head the API named`, …).
   - Any other number of parents: not proven.
5. `git merge-base --is-ancestor "$first" "$P"` must succeed. Reason otherwise:
   `the pull-request head did not contain the tip of main it was merged into`.
6. `git rev-parse "$PUSH_SHA^{tree}"` must equal `git rev-parse "$P^{tree}"`. Reason
   otherwise: `the pushed tree differs from the pull-request head's tree`.
7. `gh api --paginate "repos/$REPO/commits/$P/check-runs?per_page=100"` into a variable,
   then this filter, tried by the planner on real data (§0.4), with
   `jq -s -r --arg need_rust … --arg need_acc … --arg rust_name "$RUST_SENTINEL" --arg acc_name "$ACCEPTANCE_SENTINEL"`:

   ```jq
   [ .[].check_runs[] | select(.app.slug == "github-actions") ] as $runs
   | [ $runs[] | select(.name == "ci-required") ] as $gate
   | if ($gate | length) == 0 then "NOT no ci-required check run on the pull-request head"
     elif ($gate | any(.status != "completed")) then "NOT a ci-required check run on the pull-request head has not completed"
     else ($gate | sort_by(.completed_at) | last) as $g
       | def ran($n): any($runs[]; .name == $n and .check_suite.id == $g.check_suite.id and .status == "completed" and .conclusion == "success");
         if $g.conclusion != "success" then "NOT the latest ci-required on the pull-request head ended \($g.conclusion)"
         elif ($need_rust == "true" and (ran($rust_name) | not)) then "NOT the pull-request run did not run the Rust jobs"
         elif ($need_acc == "true" and (ran($acc_name) | not)) then "NOT the pull-request run did not run the acceptance jobs"
         else "PROVEN \($g.id)"
         end
     end
   ```

   If `gh` or `jq` fails (a non-JSON body makes `jq` fail), or the output is neither
   `PROVEN <digits>` nor `NOT …`: not proven. `NOT <text>`: not proven with that text.
8. `PROVEN <id>`: `finish true "$P" "$N" "tree <tree> equals pull-request head <P>, whose ci-required check run <id> succeeded"`.

**The self-test**, in the style of `test_detect_change_scope.sh` (a temporary repository,
`log_pass`/`log_fail`, a results line, exit 1 on any failure). No network: a fake `gh` is
put first on `PATH`, the way that file fakes `git`. The fake appends a line to a marker
file every time it is called, and prints the content of `pulls.json` when an argument
contains `/pulls`, of `check_runs.json` when one contains `/check-runs`; a file `gh.rc`
holding a number makes it exit with that number. The temporary repository has `origin` set
to its own path and `refs/pull/7/head` created with `git update-ref`, so the real
`git fetch origin refs/pull/7/head` works. Fixtures: `M0` on `main`; a branch commit `P` on
top of `M0`; `H_merge` (`git merge --no-ff`); `H_squash` (`git merge --squash` then commit);
`M1` on `main` after the branch point with `H_stale` merging the unchanged branch into it;
`H_evil` made with `git commit-tree` from a different tree with parents `M0` and `P`. A
helper writes `check_runs.json` from: the `ci-required` runs (conclusion, status,
`completed_at`, suite id) and the two sentinel runs (conclusion, suite id).

Each case is one test whose name is the sentence; it asserts `proven`, and where given
`proven_by`, `proven_pr` and whether `gh` was called:

1. a pull-request event is not proven and `gh` is never called
2. a push whose scope has no Rust or acceptance job is not proven and `gh` is never called
3. a merge commit whose second parent passed `ci-required` with both groups run is proven by that parent
4. a failed `ci-required` on the head is not proven
5. a `ci-required` still in progress on the head is not proven
6. a head with no `ci-required` check run is not proven
7. an older success followed by a newer failure is not proven
8. an older failure followed by a newer success is proven
9. a `ci-required` made by another app than GitHub Actions is not proven
10. a Rust scope with the Rust group skipped on the head is not proven
11. an acceptance scope with the images job skipped on the head is not proven
12. a sentinel that succeeded in another check suite is not proven
13. a Dagster-only scope needs only the acceptance group (Rust group skipped, images success: proven)
14. a merge whose first parent is not an ancestor of the head is not proven and `gh` is never called
15. a merge commit whose tree differs from the head's tree is not proven
16. a failing `gh` is not proven and the script still exits 0
17. a body that is not JSON is not proven and the script still exits 0
18. a squash commit whose pull request passed is proven by that pull request's head, with its number
19. a single-parent commit that no merged pull request names is not proven
20. a squash whose pull request targeted another base is not proven
21. a squash whose fetched head is not the head the API named is not proven
22. a malformed `PUSH_SHA` is not proven

Last, a consistency check written like `test_consistency` in `test_required_gate.sh`
(`python3` with `yaml`): the two constants, read from the script with `grep`, equal
`jobs.fmt.name` and `jobs.images.name` of `.github/workflows/ci.yml`; and a negative test
that a changed name makes the check fail.

**`ci.yml`, job `changes`:**
- Add a job-level block `permissions:` with `contents: read`, `checks: read`,
  `pull-requests: read`, with a comment (D7: read-only; the push proof reads the pull
  request's check runs; a job-level block replaces the workflow's, so `contents` is
  repeated).
- `outputs:` gains
  `proven: ${{ (github.event_name == 'push' && vars.CI_PUSH_FULL != 'true' && steps.proof.outputs.proven == 'true') && 'true' || 'false' }}`,
  `proven_by: ${{ steps.proof.outputs.proven_by }}`,
  `proven_pr: ${{ steps.proof.outputs.proven_pr }}`,
  `proof_reason: ${{ steps.proof.outputs.reason }}`.
- After step "Detect change scope" add step "Self-test push proof"
  (`run: bash ./scripts/ci/tests/test_detect_push_proof.sh`) and step "Detect whether this
  push repeats a passed pull request" with `id: proof` and `env:`
  `EVENT_NAME: ${{ github.event_name }}`, `PUSH_SHA: ${{ github.sha }}`,
  `REPO: ${{ github.repository }}`, `BASE_BRANCH: ${{ github.ref_name }}`,
  `GH_TOKEN: ${{ github.token }}`, and the four `SCOPE_*` from `steps.scope.outputs.*`.
  Its `run:` is:

  ```bash
  ./scripts/ci/detect_push_proof.sh || {
    echo "::warning::the push-proof script failed; nothing is treated as proven"
    echo "proven=false" >> "$GITHUB_OUTPUT"
    echo "reason=the proof script failed" >> "$GITHUB_OUTPUT"
  }
  ```

Nothing reads `proven` yet: this task changes no job's behaviour.

`docs/CI.md:59-61`: replace the paragraph with: "`ci.yml` runs with `permissions:
contents: read` for the whole workflow, which is the token a fork PR is given. One job asks
for more, and only to read: `changes` adds `checks: read` and `pull-requests: read`, which
`scripts/ci/detect_push_proof.sh` uses on a push to `main` to read the merged pull
request's check runs (`OPS-24` D7). On a pull request the script returns before any API
call."

Check: `bash scripts/ci/tests/test_detect_push_proof.sh` ends with `0 failed`; the dry run
of §3.

### T4. A proven push skips the test runs and the gates (D5, D6)

Anchors (line numbers of `9d3443cc5`; T1 changed the text of the `if:` lines, not their
place): `ci.yml:382-392` (matrix), `:455-460` (Cache cargo), `:475-478`, `:486-491`,
`:498-501`, `:506-514`, `:520-527`, the 27 jobs with the acceptance `if:` (first at 572,
last at 3755), `:3903-3949`, `:3962-3971`; `required_gate.sh:10-53,57-63,129-182`;
`test_required_gate.sh:38-43`; `docs/CI.md:63,129-136`.

1. `required_gate.sh`: document `PUSH_PROVEN` in the header. After `local scope_all=…` add
   `local push_proven="false"`, set to `true` only when `"$event_name" = "push"` AND
   `"${PUSH_PROVEN:-false}" = "true"`; echo it with the other inputs. After
   `req_acceptance` is computed add `local req_gates="$req_acceptance"`, set to `"false"`
   when `push_proven` is `true`, with a comment (D6: only the event `push` together with
   the proof excuses a gate; a failed or cancelled gate is still an error, see
   `check_job`). Change the third argument from `"$req_acceptance"` to `"$req_gates"` for
   the 25 `check_job` calls from `g1-rustfs` to `g8-time-travel` and for `image-smoke`.
   `images` keeps `"$req_acceptance"`; `test` keeps `"$req_rust"`. `check_job` itself does
   not change.
2. `test_required_gate.sh`: add `PUSH_PROVEN` to the `unset` list (line 38). Add an array
   `base_push_proven_green`: `EVENT_NAME=push SCOPE_RUST=true PUSH_PROVEN=true`; `changes`,
   `repo-lints`, `fmt`, `clippy`, `build`, `test`, `msrv`, `images` are `success`; `verify`
   and the two Dagster jobs are `skipped`; all 25 gates and `image-smoke` are `skipped`.
   Tests:
   - `expect 0 "Proven push with the gates skipped passes gate"` (the array)
   - `expect 0 "Proven push whose gates ran green passes gate"` (the array, `G4_CDC_RESULT=success`)
   - `expect 1 "A failed gate on a proven push blocks gate"` (`G4_CDC_RESULT=failure`)
   - `expect 1 "A cancelled gate on a proven push blocks gate"` (`G6_OPS_RESULT=cancelled`)
   - `expect 1 "A skipped images job on a proven push blocks gate"` (`IMAGES_RESULT=skipped`)
   - `expect 1 "A skipped Rust test job on a proven push blocks gate"` (`RUST_TEST_RESULT=skipped`)
   - `expect 1 "A skipped clippy job on a proven push blocks gate"` (`RUST_CLIPPY_RESULT=skipped`)
   - `expect 1 "Skipped gates on a push that is not proven block gate"` (`PUSH_PROVEN=false`)
   - `expect 1 "Skipped gates on a push with no proof value block gate"` (`PUSH_PROVEN=`)
   - `expect 1 "The proof is ignored on a pull request"` (`EVENT_NAME=pull_request`)
   - `expect 1 "The proof is ignored on a scheduled run"` (`EVENT_NAME=schedule SCOPE_ALL=true`)
3. `ci.yml`, the 26 jobs that are gates or `image-smoke` (every job with the acceptance
   `if:` EXCEPT `images`): the `if:` becomes

   `if: needs.changes.outputs.proven != 'true' && (needs.changes.outputs.rust == 'true' || needs.changes.outputs.dagster == 'true' || needs.changes.outputs.stack == 'true' || needs.changes.outputs.all == 'true')`

   `images` keeps the `if:` T1 left it with, and gets a comment: it also runs on a proven
   push because it is the only writer of the image layer cache pull requests read (D5).
   Counts afterwards: `grep -c "needs.changes.outputs.proven != 'true' && (needs.changes.outputs.rust" .github/workflows/ci.yml`
   prints `26`.
4. `ci.yml`, job `test`:
   - `matrix.partition` becomes
     `${{ fromJSON(needs.changes.outputs.proven == 'true' && '[1]' || '[1, 2, 3]') }}`,
     with a comment (D5: a proven push keeps one leg, to keep the dependency cache pull
     requests restore; any value but `true` gives the three legs).
   - Step "Cache cargo" gets `id: cache`.
   - After "Install cargo-nextest" add a step "Build the tests without running them (proven
     push, cache not current)" with
     `if: needs.changes.outputs.proven == 'true' && steps.cache.outputs.cache-hit != 'true'`,
     `run: cargo nextest run --workspace --locked --profile ci --no-run`,
     `working-directory: rust`. Comment: `rust-cache` saves only when the restore was not
     an exact hit, so only then is there something to build.
   - Step "cargo nextest run (partition …)" gets `if: needs.changes.outputs.proven != 'true'`.
   - The doctest step's `if:` becomes `matrix.partition == 1 && needs.changes.outputs.proven != 'true'`.
   - Steps "Record nextest partition count" and "Upload nextest partition count": `if:`
     becomes `always() && needs.changes.outputs.proven != 'true'`.
5. `ci.yml`, job `ci-required`:
   - Step "Enforce required gate": add `PUSH_PROVEN: ${{ needs.changes.outputs.proven }}`.
   - Before it add a step "Say what this push did not repeat" with
     `if: github.event_name == 'push'` and `env:` `PROVEN`, `PROVEN_BY`, `PROVEN_PR`,
     `PROOF_REASON` from `needs.changes.outputs.*`. It appends to `$GITHUB_STEP_SUMMARY`
     and echoes: when `PROVEN` is `true`, "Proven push: the test runs and the acceptance
     jobs were skipped. `$PROOF_REASON` (pull request #`$PROVEN_PR`, head `$PROVEN_BY`)."
     (leave the number out when it is empty); otherwise "Not proven: `$PROOF_REASON`.
     Every job in scope ran." The values are used only inside double quotes.
   - The two count steps (3962, 3970): `if:` becomes
     `needs.changes.outputs.proven != 'true' && (needs.changes.outputs.rust == 'true' || needs.changes.outputs.all == 'true')`.
6. `docs/CI.md`: after the paragraph T1 wrote add a subsection "### A push that repeats a
   passed pull request" holding D4's six conditions, D5's two lists and D6's two tables,
   reworded as statements of what the workflow does. At line 63, after "fails if any
   required job failed, cancelled, or was skipped when the scope indicated it should have
   run" add ", except the acceptance jobs and the test runs of a push proven to repeat a
   passed pull request (below)". In "Caches" after the `Swatinem/rust-cache` bullet add:
   "On a proven push the cache writers still run: `clippy`, `build`, `msrv`, one leg of
   `test` (it compiles only when its cache is not current) and `images`."

Check: `bash scripts/ci/tests/test_required_gate.sh` ends with `0 failed` (its consistency
tests still pass: no job was added); the count above.

### T5. The Rust test job: pull the test images first, and print two sizes (D13, D9)

Anchor: `ci.yml`, job `test`, after the second "Free runner disk" step (448-452) and after
the nextest step (486-491).

1. After the second "Free runner disk" step add a step "Pull the test images (the pull is
   retried, a test never is)" with `if: needs.changes.outputs.proven != 'true'` (leave the
   `if:` out if T4 was dropped):

   ```bash
   for image in postgres:16-alpine clickhouse/clickhouse-server:26.8 axllent/mailpit:v1.33.0; do
     for attempt in 1 2 3; do
       if docker pull --quiet "$image"; then break; fi
       echo "pull of $image failed (attempt $attempt of 3)"
       sleep $((attempt * 10))
     done
   done
   docker images --format '{{.Repository}}:{{.Tag}}' | sort
   ```

   Comment: the tags are those the tests name
   (`lakehouse-test-support/src/lib.rs:279`, `lakehouse-api/tests/email_settings_route.rs:52-53`,
   the `with_tag("26.8")` of the ClickHouse tests); a tag that drifts only loses the retry,
   the test still pulls its own. The step ends with `docker images`, so it cannot fail the
   job. The repository name of the ClickHouse image is the default of
   `testcontainers-modules`: confirm it with
   `grep -rn 'const NAME' ~/.cargo/registry/src/*/testcontainers-modules-*/src/clickhouse/mod.rs`
   and, if it differs, use what you find and say so in the handoff.
2. After the nextest step add a step "Report disk and test binary sizes (OPS-24, measured
   for slice S2)" with `if: always() && matrix.partition == 1 && needs.changes.outputs.proven != 'true'`
   and `working-directory: rust`:

   ```bash
   set +e   # a measurement: no line of it may fail the job
   df -h / || true
   du -sh target 2>/dev/null || true
   find target/debug/deps -maxdepth 1 -type f -executable ! -name '*.so' -printf '%s %p\n' 2>/dev/null \
     | sort -rn > "$RUNNER_TEMP/bins.txt" || true
   awk '{s+=$1; n++} END {printf "test binaries: count=%d total_gib=%.2f\n", n, s/1073741824}' "$RUNNER_TEMP/bins.txt" || true
   largest="$(head -1 "$RUNNER_TEMP/bins.txt" | cut -d' ' -f2-)"
   if [ -n "$largest" ]; then
     raw=$(stat -c %s "$largest"); packed=$(zstd -3 -T0 -c "$largest" | wc -c)
     echo "largest binary: raw=$raw zstd3=$packed"
   fi
   true
   ```

   Comment: a measurement, not a check; it prints the disk the job really used (§0.1: 86 GB
   were free before "Free runner disk") and the size an archive of the test binaries would
   have.

No `docs/CI.md` line.

Check: the syntax check of §3.

### T6. The small Rust jobs can run on a CI worker, behind a switch that is off (D10)

New files, mode 755: `scripts/ci/select_runner.sh`, `scripts/ci/worker_prepare.sh`,
`scripts/ci/tests/test_select_runner.sh`, `scripts/ci/tests/test_worker_prepare.sh`.
Anchors: `ci.yml:37-64` (`changes`), `:298-319` (`fmt`), `:321-350` (`clippy`), `:352-374`
(`build`), `:537-562` (`msrv`). The `test` job is NOT changed (D10).

1. `select_runner.sh`. Inputs: `EVENT_NAME`, `REPO`, `HEAD_REPO` (the pull request's head
   repository, empty on other events or when the fork was deleted), `SWITCH`, `OUTPUT_NAME`.
   It prints and writes to `GITHUB_OUTPUT` one line `<OUTPUT_NAME>=<json>`. The value is
   `["self-hosted","lakehouse-ci"]` only when `SWITCH` is exactly `true` AND `REPO` is not
   empty AND (`EVENT_NAME` is not `pull_request`, OR `HEAD_REPO` equals `REPO`). In every
   other case it is `["ubuntu-latest"]`. `OUTPUT_NAME` must match `^[a-z_]+$`, or the script
   exits 1. The two label lists are constants at the top, with a comment: the labels are
   fixed here so that no variable can name another runner; the staging runner does not
   carry `lakehouse-ci`.
2. `test_select_runner.sh`, same style. Cases: the switch unset gives hosted on a pull
   request and on a push; `true` with a same-repository pull request gives the worker;
   `true` with a fork's pull request gives hosted; `true` with an empty head repository on
   a pull request gives hosted; `true` on `push`, `schedule` and `workflow_dispatch` gives
   the worker; `TRUE`, `1` and `yes` give hosted; an empty `REPO` gives hosted. Then a
   workflow lint in `python3` with `yaml`, over `ci.yml` and `coverage.yml`: for every job
   whose `runs-on` is not the plain string `ubuntu-latest` and not `[self-hosted, staging]`
   (a) the first step is named "Refuse a self-hosted runner that is not the CI worker" and
   its `if` is `runner.environment == 'self-hosted'`; (b) every step whose `run` contains
   `sudo` or `prune`, and every step that uses `Swatinem/rust-cache`, has an `if` that
   contains `runner.environment == 'github-hosted'`; (c) the text `lakehouse-ci` appears in
   a workflow file only inside a job whose workflow has no `pull_request` trigger. One
   negative test for each of (a), (b), (c) on a modified copy.
3. `worker_prepare.sh` (runs only on a self-hosted runner, after checkout). In order:
   - Runner facts, each line ending `|| true`: `echo "runner=$RUNNER_NAME"`, `nproc`,
     `free -g`, `df -h "$HOME" "$GITHUB_WORKSPACE"`, `findmnt -no OPTIONS -T "$HOME"`,
     `docker info --format 'docker={{.ServerVersion}} cpus={{.NCPU}} mem={{.MemTotal}} containers={{.Containers}} images={{.Images}}'`,
     `rustup show`, `ldd --version | head -1`. No host name and no address.
   - `root="${WORKER_CACHE_ROOT:-$HOME/.cache/lakehouse-ci}"`; the directory is
     `"$root/target/<name>"` where `<name>` is `RUNNER_NAME` with every character outside
     `A-Za-z0-9._-` replaced by `_`; an empty name is an error (exit 1). `mkdir -p`. One
     directory per runner registration: two registrations never build in the same
     directory, and cargo's own lock on the shared `~/.cargo` registry covers downloads.
   - Disk bound: the limit is `TARGET_MAX_GB` when it is an integer from 5 to 500, else 40.
     If `du -sk` of the directory is over the limit: first
     `find "$dir" -type f -atime +3 -delete`; measure again; if still over, remove THIS
     directory (`rm -rf -- "$dir"`, only after checking that `"$dir"` starts with
     `"$root/target/"`) and recreate it. Print the size before and after and which stage
     ran, as a `::warning::` when stage 2 ran. This is the one place a file outside the
     workspace is deleted on a self-hosted runner, and it is the runner's own cache; never
     `cargo clean`, never on every run.
   - Append `CARGO_TARGET_DIR=<dir>` to `GITHUB_ENV`, and `CARGO_BUILD_JOBS=<n>` when
     `WORKER_CARGO_JOBS` is an integer from 1 to 64.
4. `test_worker_prepare.sh`: with `HOME`, `WORKER_CACHE_ROOT`, `RUNNER_NAME`, `GITHUB_ENV`
   and `GITHUB_WORKSPACE` pointing into a temporary directory. Cases: a first run creates
   the directory and writes `CARGO_TARGET_DIR`; a runner name with a slash and a space
   becomes a name inside the root; an empty name fails; under the limit nothing is deleted;
   over the limit (use `truncate`-made files and old access times set with `touch -a -d`)
   the old files go and the new stay; still over the limit, the directory is emptied and a
   sibling directory is untouched; `TARGET_MAX_GB=abc` uses 40.
5. `ci.yml`, job `changes`: `outputs:` gains `compile_runner: ${{ steps.runner.outputs.compile_runner }}`.
   Add steps "Self-test runner selection"
   (`bash ./scripts/ci/tests/test_select_runner.sh`), "Self-test worker preparation"
   (`bash ./scripts/ci/tests/test_worker_prepare.sh`) and "Choose the runner of the small
   Rust jobs" (`id: runner`, `run: ./scripts/ci/select_runner.sh`) with `env:`
   `EVENT_NAME: ${{ github.event_name }}`, `REPO: ${{ github.repository }}`,
   `HEAD_REPO: ${{ github.event.pull_request.head.repo.full_name }}`,
   `SWITCH: ${{ vars.CI_WORKER_COMPILE }}`, `OUTPUT_NAME: compile_runner`.
6. `ci.yml`, jobs `fmt`, `clippy`, `build`, `msrv`:
   - `runs-on: ${{ fromJSON(needs.changes.outputs.compile_runner || '["ubuntu-latest"]') }}`.
   - New FIRST step, before "Checkout":

     ```yaml
           - name: Refuse a self-hosted runner that is not the CI worker
             if: runner.environment == 'self-hosted'
             run: |
               if [ "${LAKEHOUSE_CI_WORKER:-}" != "1" ]; then
                 echo "::error::this self-hosted runner is not marked as the CI worker (LAKEHOUSE_CI_WORKER=1 in the runner's .env); refusing before checkout"
                 exit 1
               fi
     ```

   - In `clippy`, `build`, `msrv`: the "Cache cargo" step gets
     `if: runner.environment == 'github-hosted'` (the worker keeps its own target
     directory; uploading and downloading a cache its disk already has would cost time),
     and after "Setup Rust" a step "Prepare the CI worker" with
     `if: runner.environment == 'self-hosted'`,
     `run: bash ./scripts/ci/worker_prepare.sh` and `env:`
     `TARGET_MAX_GB: ${{ vars.CI_WORKER_TARGET_MAX_GB }}`,
     `WORKER_CARGO_JOBS: ${{ vars.CI_WORKER_CARGO_JOBS }}`. `fmt` gets neither (it compiles
     nothing).
   These jobs start no container, so they leave nothing to clean up.
7. `docs/CI.md`: new section "## Runners" before "## MSRV": the two switches
   (`CI_WORKER_COMPILE` here, `CI_COVERAGE_ON_WORKER` in T7), both unset by default and
   then the workflow is as before; the fixed labels and why staging cannot be reached; the
   marker step; the fork rule and its limit; the worker's target directory and its bound;
   D10's minimum machine and the sentence that a machine below it must not get a job a pull
   request requires; that with `CI_WORKER_COMPILE` on, `main` no longer writes the hosted
   caches of `clippy`, `build` and `msrv`, so a fork's pull request compiles those cold.

Check: the two self-tests end with `0 failed`; with the variable unset the output is
`compile_runner=["ubuntu-latest"]`.

### T7. Coverage can run on the CI worker, behind its own switch that is off (D11)

Anchor: `.github/workflows/coverage.yml:15-98` (job `coverage`). Job `sbom` is not changed.

1. `runs-on: ${{ fromJSON(vars.CI_COVERAGE_ON_WORKER == 'true' && '["self-hosted","lakehouse-ci"]' || '["ubuntu-latest"]') }}`,
   with a comment: D11; this workflow has no `pull_request` trigger, so only `main`'s code
   reaches the worker; nothing waits for this job; when the worker is offline the run waits
   and the concurrency rule above keeps at most one waiting.
2. First step: the same "Refuse a self-hosted runner that is not the CI worker" step as T6.
3. "Free runner disk" and "Cache cargo": add `if: runner.environment == 'github-hosted'`.
4. After "Setup Rust": the "Prepare the CI worker" step of T6, same `env:`.
5. Before "cargo llvm-cov": a step "Note the containers that exist (worker only)" with
   `if: runner.environment == 'self-hosted'`:
   `docker ps -aq | sort > "$RUNNER_TEMP/containers-before.txt"`.
6. After "cargo llvm-cov": a step "Remove the containers this job started (worker only)"
   with `if: always() && runner.environment == 'self-hosted'`: remove every container that
   `docker ps -aq` lists now and that file did not, EXCEPT those with the label
   `org.rantai.lakehouse-test-support` (the reusable Postgres the test-support crate keeps
   on purpose): `docker rm -f` on each, then `docker ps -a`. Comment: correct only because
   one container-starting job runs on this worker at a time (this workflow's concurrency
   group; the small Rust jobs start none); a second such job needs another rule (S2). No
   image is removed and nothing else is touched.
7. `docs/CI.md`: one paragraph in "## Runners" (what T7 moves, why it qualifies, what
   happens when the worker is offline) and D11's table.

Check: `bash scripts/ci/tests/test_select_runner.sh` (its lint now covers `coverage.yml`)
ends with `0 failed`; the syntax check of §3.

## 2b. Slice S2 (its own pull request, after S1's first run): compile the tests once

Start only when T5's two lines exist. Hosted runners only; nothing here needs the worker.

Decisions:
- **S2-D1.** A job `test-build` (same `if:` as the Rust group, same environment variables
  `CARGO_PROFILE_DEV_DEBUG` and `CARGO_PROFILE_TEST_DEBUG=line-tables-only`) restores the
  cache with `shared-key: stable-test` (the key stays
  `v0-rust-stable-test-…`: `rust-cache` uses `shared-key` INSTEAD of `key` plus the job
  name, read in its `config.ts`), runs
  `cargo nextest archive --workspace --locked --archive-file "$RUNNER_TEMP/nextest-archive.tar.zst"`,
  prints its size, runs the doctests (`cargo test --workspace --locked --doc`), and uploads
  the archive (`retention-days: 1`, `compression-level: 0`).
- **S2-D2.** The partitions `needs: [changes, test-build]`, check out the repository (tests
  read fixtures and migrations from it), set up Bun, install nextest, download the archive
  and run `cargo-nextest nextest run --archive-file … --workspace-remap "$GITHUB_WORKSPACE/rust" --profile ci --color never --partition count:N/M`.
  No Rust toolchain, no cache step, no disk step unless T5 showed the disk is needed.
- **S2-D3. Same path on both sides.** 24 uses of `env!("CARGO_MANIFEST_DIR")` in 17 files
  bake the build machine's absolute path into the binaries; `--workspace-remap` does not
  change a compile-time value. Build and partitions therefore run on hosted runners (both
  see `/home/runner/work/…`). Building on a self-hosted machine is a later step that needs
  those 24 sites to read the path at run time.
- **S2-D4. Partitions.** `M` comes from a repository variable `CI_TEST_PARTITIONS` (1 to 6,
  default 3), validated by a script with a self-test and exposed by `changes`; the count
  check loops over `1..M` and keeps its floor (6,826). Start at 4: tests are 2,097 s in
  all, so about 525 s per partition.
- **S2-D5. Debug information.** Keep `line-tables-only` in the first run. `0` makes the
  binaries and the archive smaller and the link faster; what is lost is `file:line` in a
  BACKTRACE (a panic message keeps its location). Changing it changes every cache key once.
  Decide from T5's numbers and the first archive's size.
- **S2-D6. Gate.** `test-build` is added to `ci-required`'s `needs` and to
  `required_gate.sh` (`RUST_TEST_BUILD_RESULT`, required with the Rust group, also on a
  proven push, where it replaces T4's compile-only leg); the partitions become skippable on
  a proven push.
- **S2-D7. Stop rule.** From the pull request's own run the developer reports: `A` (archive
  step plus upload, seconds), `D` (download plus extraction per partition), the archive's
  size, each job's total. If `A + M × D` is 1,054 s or more (the two compiles saved), or
  the build job plus the slowest partition is more than 1,831 s (today's slowest partition
  plus five minutes), **stop, change nothing, report the numbers**; the planner decides.

Tasks: S2-T1 the partition-count script, self-test and `changes` output; S2-T2 `test-build`
and the rewired partitions; S2-T3 the gate script, its tests and the count check; S2-T4
`docs/CI.md` and `rust/.config/nextest.toml`'s comment. Verification as §3.

## 3. Verification (the developer cannot run GitHub Actions locally)

Per commit, only for what it touched:

```bash
bash scripts/ci/tests/test_detect_change_scope.sh
bash scripts/ci/tests/test_required_gate.sh
bash scripts/ci/tests/test_detect_push_proof.sh      # from T3
bash scripts/ci/tests/test_select_runner.sh          # from T6
bash scripts/ci/tests/test_worker_prepare.sh         # from T6
which actionlint && actionlint .github/workflows/ci.yml .github/workflows/coverage.yml
python3 -c "import yaml,sys; [yaml.safe_load(open(f)) for f in sys.argv[1:]]" .github/workflows/ci.yml .github/workflows/coverage.yml
git diff --stat && git status
```

If `actionlint` is not installed, the `yaml.safe_load` line is the only syntax check, and
it is weaker: it does not know expressions, contexts or `needs`. Write that in the handoff.
The self-tests need `bash`, `git`, `jq` and `python3` with `yaml`; if one is missing, say
which and write *not verified*.

Dry run of the proof (T3), only if `gh auth status` succeeds; otherwise write *not
verified, the planner runs it*. In the checkout, after `git fetch origin`:

```bash
for sha in 43d36f55b 9d3443cc5 2306e7302 eaebacf28; do
  EVENT_NAME=push PUSH_SHA="$(git rev-parse "$sha")" REPO=RantAI-dev/RantAI-Lakehouse \
    SCOPE_ALL=true bash scripts/ci/detect_push_proof.sh
done
```

Expected: `proven=true proven_by=7e098119…`; `proven=true proven_by=22c641fe… proven_pr=500`;
`proven=false` (the pull-request run did not run the Rust jobs); `proven=false` (same).

The pull request's own run is the test. The planner opens the draft pull request once T1
to T4 are pushed. The handoff lists, for each task, the job and the log line that shows it
worked, read with `gh run view <id> --log --job <job id>`:

| Task | Job | Line to quote |
|---|---|---|
| T1 | `Detect Change Scope` | `PASS a documents-only push is documents-only`, the results line |
| T1 | `ci-required` | `PASS Documents-only push with only the lints passes gate`, the results line |
| T3 | `Detect Change Scope` | the proof self-test's results line; `proven=false` with `reason=not a push event` |
| T4 | `ci-required` | `PASS Proven push with the gates skipped passes gate`; `nextest total across partitions: … (floor 6826)` |
| T4 | the run | three `Rust · test N/3` jobs and all gate jobs present and green |
| T5 | `Rust · test 1/3` | the three image names; `test binaries: count=… total_gib=…`; `largest binary: raw=… zstd3=…`; both `df` lines |
| T6 | `Detect Change Scope` | `compile_runner=["ubuntu-latest"]`; both results lines |
| T6 | `Rust · clippy` | `Hosted Compute Agent`; `Restored from cache key "v0-rust-clippy-…" full match: true` |

Record the wall time and the job-minutes of the pull request's run next to §0.1's.

### 3a. Change of machine

The developer works on the ARM64 build machine in the checkout
`/home/rismana/rantai/lakehouse-agents-2`, and only under `/home/rismana/rantai/`. No cargo
build is needed for this slice. If a task wants one to measure something:
`export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-2 CARGO_BUILD_JOBS=6`, plain
`cargo`, never `cargo clean`, never `cargo test --workspace`. The developer never opens or
merges a pull request, never sets or changes a repository setting or variable, and never
registers or configures a runner.

### 3b. What the planner checks after the merge (the push path)

The merge of this pull request is the first push under the new rules, and its scope is
`all`. On that run:
1. `Detect Change Scope`: `proven=true`, `proven_by=` this pull request's head.
2. Jobs: the 25 gates and `image-smoke` skipped; one `Rust · test 1/3 (stable)`; `fmt`,
   `clippy`, `build`, `msrv`, `images`, the console and Dagster jobs ran.
3. `ci-required` green; its summary starts "Proven push:" and names the head.
4. `Rust · test 1/3`: either `full match: true` and no build step, or the build step ran
   and the post step saved the cache. `images`: the two builds exported their cache.
5. The next documents-only merge: 3 jobs in CI, no Coverage run.
6. The next morning: the scheduled run ran all 42 jobs.

If any of 1 to 4 is wrong: set the repository variable `CI_PUSH_FULL` to `true`. The next
push runs everything. That is the whole revert; the fix then comes as a pull request.

For the planner, not the developer: AGENTS.md step 6 compares a pull request with "every
check that passes on `main`". After this, a gate's state on `main` is read from the nightly
run (or the last push that was not proven). Open pull request #379 (a stale draft) edits
the same scripts and adds a `site` job; when it rebases, its `if:` loses
`github.event_name == 'push' ||`. Open pull request #502 does not touch these files.

### 3c. The worker: the owner's checklist, before any variable is set

Not the developer's work. Read from the API today: fork approval policy is
`first_time_contributors`; `allowed_actions` is `all`; default workflow permissions are
`read`; the two runners are registered on the repository, not in an organisation group
(the organisation's runner groups could not be read: 403).

1. Settings, Actions, General: "Require approval for all external contributors". Today a
   contributor with one merged pull request can run an edited workflow that names the
   worker's labels. Whoever approves a fork's run reads its `.github/` and `scripts/ci/`
   diff first.
2. The CI worker's runner `.env` holds `LAKEHOUSE_CI_WORKER=1`. The staging runner's does
   not, and the staging runner never gets the label `lakehouse-ci`.
3. The runner's user has no `sudo`. It is in the `docker` group only if Coverage runs there
   (Docker access is root on that machine): then `CI_WORKER_COMPILE` stays unset, so that
   no pull request's code runs beside Docker.
4. Nothing that user can read is a secret (keys, tokens, other checkouts).
5. Optional: a runner registered at organisation level in a group limited to this
   repository; not needed for D11.

Then, for this laptop: leave `CI_WORKER_COMPILE` unset. Set `CI_COVERAGE_ON_WORKER=true`
and `CI_WORKER_CARGO_JOBS=4`, start "Coverage & SBOM" by hand, and read the "Prepare the CI
worker" step (cores, memory, disk, mount options) and the job's time. If the job is killed
for memory or passes 90 minutes, unset the variable: the next run is on a hosted runner.

## 4. Out of scope (each with a proposal for the owner)

- **Gates chosen by changed paths: the largest lever left (156 of 253 job-minutes).**
  Proposal: a pull request runs a gate only when its paths changed, and the nightly run of
  D3 runs all of them. A first cut: `ops/g<n>/**` selects its own job; `dagster/**` selects
  the gates that load through Dagster (`g3a`, `g3`, the four `g4-*`, `g6-ingest*`,
  `g6-oracle`, `g9`, `g6-saas-*`, `g8-time-travel`); `rust/crates/lakehouse-auth/**` and
  `ops/sso/**` select `g10-sso`; `rust/crates/lakehouse-alerts/**` and `lakehouse-notify/**`
  select `g6-alerts`; anything else under `rust/crates/lakehouse-api/**` or
  `docker-compose.yml` selects all. Needs a per-gate scope in `detect_change_scope.sh` and
  `required_gate.sh` and the owner's decision that a gate may first fail at night.
- **Fewer test binaries**: the 119 integration test files of `lakehouse-api` each link the
  whole crate; this is most of the 8 to 10 minutes each partition compiles (CI-SPEED-PLAN
  §4.6 already lists it). A Rust change of its own.
- **"Free runner disk"** costs 67 to 142 s in every test partition and in Coverage, and the
  runner now has 86 GB free before it. Remove it once T5's `df` shows what the job uses.
- **`images` on a larger worker**: one job that every gate waits for (671 s). Needs a
  machine of D10's size and a persistent builder.
- **Pulls in the gates** (the `quay.io` failure): a retry around `docker compose pull` in 26
  jobs, or a registry mirror in the Docker daemon's configuration. One composite action.
- **Superseded runs**: 167 of 205 pull-request runs on 2026-11-12 were cancelled by a newer
  push (a sample of 21 used 34 job-minutes each on average, 15 of them under one minute).
  Proposal: workers push to a branch with no pull request until the slice is complete.
- **Merge queue**: a repository setting; it would make `strict` cheap, and it changes where
  a failure is first seen. **A paid plan**: more than 20 concurrent jobs.
- **The 24 `env!("CARGO_MANIFEST_DIR")` sites** (needed before a test build can move to
  another machine); **`max_connections`** of the shared test Postgres (needed before test
  jobs share a Docker daemon).
- `security.yml`, `deploy-staging.yml`, `release.yml`: not touched.

## Open questions for the owner

- **Q1. Merges.** Squash merges are 33 of the last 52 pushes, so T3 handles them. Rebase
  merges and direct pushes are never proven and run everything. Recommendation: keep it so.
- **Q2. The nightly full run** (D3, about 265 job-minutes a night). Recommendation: yes; it
  is what replaces "every job runs on `main`".
- **Q3. Coverage.** T2 stops it for documents-only pushes. Beyond that: on the laptop
  (T7), or nightly on a hosted runner (about 50 job-minutes a day instead of 600)?
  Recommendation: try the laptop once with §3c; if it is not reliably awake, make Coverage
  nightly.
- **Q4. The worker.** Recommendation: no required job on this machine (D10); the fork
  approval setting of §3c before any variable is set.
- **Q5. S2.** Recommendation: decide after T5 prints the sizes; on today's estimate the
  gates-by-path change is worth ten times more.

## 5. Handoff

## 6. Review
