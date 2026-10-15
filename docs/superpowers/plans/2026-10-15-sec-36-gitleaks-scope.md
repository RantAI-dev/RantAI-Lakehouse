# `SEC-36` History scan scope: plan

Planner: Claude Opus (Security & Delivery lane). Developer: a non-Opus agent.
Base: `origin/main` at `41d7677a`. Branch: `fix/sec-36-gitleaks-scope`,
worktree `/home/shiro/rantai/wt/sec-36`. Spec: `docs/core/specs/sec-36.md`
(decisions D1–D3 signed by the product owner on 2026-10-15). A Task, so no
feature page.

## 1. Anchors (verified at `41d7677a`)

- `.github/workflows/security.yml`, job `history-scan`: checkout with
  `fetch-depth: 0`, then a step `git fetch --all --tags`, then
  `gitleaks detect --source . --config .gitleaks.toml --redact -v`.
- With no `--log-opts`, `gitleaks` v8 runs `git log` with `--all` itself, so
  removing the fetch step alone is not enough: `actions/checkout` with
  `fetch-depth: 0` already fetches every branch. **Verify** in the
  `gitleaks` v8.30.1 source (`sources/git.go` or `detect/git.go`) which
  `git log` arguments are used with and without `--log-opts`, and quote the
  line in the handoff.
- On `pull_request`, `actions/checkout` checks out `refs/pull/<n>/merge`,
  whose history is `main` plus the pull request's commits.
- `docs/CI.md` section "`history-scan`: baselined, red only on a new
  finding" (line 151 on) describes the job and says "The history scan
  reads every branch" in its table.
- #144's branch already carries 22 exact fingerprints with proof in
  `.gitleaksignore` (its diff against `main` is one appended block after
  the `ee0d663d…bld-3…:623` entry).

## 2. Tasks (one commit each, in order)

**T1. Scope the PR and `main` scan to `HEAD` (D1).** In `history-scan`:
delete the "Fetch all branches" step; run
`gitleaks detect --source . --config .gitleaks.toml --redact -v --log-opts="HEAD"`
(or the `--log-opts` value T1's verification shows is right, e.g. adding
`--full-history`). Rewrite the step comment and the job summary text to say
what is scanned and why (D1, `SEC-36`). Keep the job name unchanged so
nothing that refers to it breaks.
*Check:* `actionlint` if available, else `python3 -c 'import yaml,sys; yaml.safe_load(open(".github/workflows/security.yml"))'`;
locally, in a scratch clone with `git fetch origin '+refs/heads/*:refs/remotes/origin/*'`:
`docker run --rm -v "$PWD":/repo -w /repo zricethezav/gitleaks:v8.30.1 detect --source /repo --config /repo/.gitleaks.toml --redact --log-opts="HEAD"`
reports no leaks, and the same with `--log-opts="--all"` reports the
#144 findings. Quote both totals (AC2).

**T2. Daily all-branch scan (D2).** New workflow
`.github/workflows/security-all-branches.yml`: `on: schedule` (once a day,
`cron: '17 3 * * *'`) and `workflow_dispatch`; `permissions: contents: read`;
checkout `fetch-depth: 0`, then the same pinned `gitleaks` install and
`detect` as `history-scan` but with `--log-opts="--all"`. A header comment
says why it exists (public repo, D2) and that it is deliberately not a
pull-request check. No `continue-on-error`.
*Check:* YAML parses; `on:` has no `push` or `pull_request`.

**T3. Docs.** `docs/CI.md`: the `history-scan` section says what it scans
now and names the daily job; the "reads every branch" sentence in the table
is corrected; a short paragraph says a red daily run means a branch carries
a finding and its owner fixes it (exact fingerprints with proof, or a value
the rule does not match). `CHANGELOG.md` `[Unreleased]`: one bullet under
the existing CI or Security heading, in the file's style.
*Check:* `grep -n 'fetch --all' docs/CI.md .github/workflows/security.yml`
prints nothing.

## 3. Pull request

One slice. The planner adds the backlog and spec commits to the same branch.

## 4. Out of scope

`.gitleaks.toml` rules; `.gitleaksignore` entries (#144 brings its own);
the working-tree `gitleaks` job; `ci.yml`.

## 5. Verify, do not assume

- Does `actions/checkout@v7` with `fetch-depth: 0` fetch all branches?
  Whatever the answer, `--log-opts` must make the scan independent of it.
- Does `git log -p` skip merge commits' diffs? Then a conflict resolution
  is not scanned; say so in `docs/CI.md` if true (it is true today too).

## 6. Handoff

### Handoff (2026-10-15, Round 1)

1. Commits:
```
aaa2cc4510d5 docs(security): document history-scan HEAD scoping and daily all-branch scan (SEC-36)
611e3fc5ebc5 ci(security): add daily all-branch history scan workflow (SEC-36)
60303bb90930 ci(security): scope history-scan to HEAD on main and PRs (SEC-36)
c8c8662db6ab docs(core): SEC-36 spec and plan; backlog status for SEC-16/17/18/27/36
```

2. Verifications run:
- **T1 gitleaks HEAD vs --all in scratch clone (AC2):**
  Scratch clone created under `/home/shiro/rantai/review-logs/sec36-scratch` with `git fetch origin '+refs/heads/*:refs/remotes/origin/*'`.
  - Command: `docker run --rm -v /home/shiro/rantai/review-logs/sec36-scratch:/repo -w /repo zricethezav/gitleaks:v8.30.1 detect --source /repo --config /repo/.gitleaks.toml --redact --log-opts="HEAD"`
    Counts / output: `1289 commits scanned. scanned ~25966800 bytes (25.97 MB) in 4.81s; no leaks found` (0 leaks, exit code 0).
  - Command: `docker run --rm -v /home/shiro/rantai/review-logs/sec36-scratch:/repo -w /repo zricethezav/gitleaks:v8.30.1 detect --source /repo --config /repo/.gitleaks.toml --redact --log-opts="--all"`
    Counts / output: `2106 commits scanned. scanned ~38545966 bytes (38.55 MB) in 6.55s; leaks found: 25` (exit code 1).
    Findings breakdown: 22 `generic-api-key` findings on `origin/feat/phase-1-rest` (#144); 3 `rantai-internal-lan-host` on `origin/backup-pre-rewrite`.
  - Scratch clone deleted when done (`rm -rf /home/shiro/rantai/review-logs/sec36-scratch`).
- **Gitleaks v8.30.1 source verification (`sources/git.go`):**
  Inspected upstream `sources/git.go` lines 74–95:
  Without `--log-opts`:
  `cmd = exec.CommandContext(ctx, "git", "-C", sourceClean, "log", "-p", "-U0", "--full-history", "--all", "--diff-filter=tuxdb")` (lines 93–94)
  With `--log-opts`:
  `args := []string{"-C", sourceClean, "log", "-p", "-U0"}` (line 75)
  `args = append(args, userArgs...)` (line 90)
  `cmd = exec.CommandContext(ctx, "git", args...)` (line 91)
- **Merge commit diff behavior in `git log -p`:**
  Ran `git log -p -n 1 6a1285d6` on a merge commit in the repo. Git outputs the commit message and metadata but skips diff output for merge commits by default (diffs require `-m`, `-c`, or `--cc`). Documented in `docs/CI.md`.
- **T1 workflow YAML validation:**
  Command: `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/security.yml"))'`
  Output: valid YAML.
- **T2 workflow YAML & trigger validation:**
  Command: `python3 -c 'import yaml; p = yaml.safe_load(open(".github/workflows/security-all-branches.yml")); tr = p.get("on") or p.get(True); assert "push" not in tr and "pull_request" not in tr and "schedule" in tr and "workflow_dispatch" in tr'`
  Output: validated schedule `17 3 * * *` and workflow_dispatch present; push and pull_request absent.
- **T3 grep check:**
  Command: `grep -n 'fetch --all' docs/CI.md .github/workflows/security.yml`
  Output: 0 matches (exit code 1).
- **Working-tree gitleaks pre-push check:**
  Command: `docker run --rm -v /home/shiro/rantai/wt/sec-36:/repo -w /repo zricethezav/gitleaks:v8.30.1 dir /repo --config /repo/.gitleaks.toml --no-banner --redact`
  Counts / output: `scanned ~20069881 bytes (20.07 MB) in 3.79s; no leaks found` (exit code 0).
- **Languages not touched:**
  Rust (cargo fmt/clippy/test), TypeScript (bun run typecheck/lint/test), Python (ops/lint, pytest), Docker Compose config/up: *not verified* (only CI workflows, docs/CI.md and CHANGELOG.md touched; per AGENTS.md "Keep build time down: Only build what changed").

3. Skipped / premises:
- Nothing skipped. The plan's premises held: `--log-opts="HEAD"` isolates the scan to the checked-out commit on main and PRs, and `--log-opts="--all"` in the daily workflow catches the 22 unmerged findings from #144.

4. Repository status:
```
$ git status --short
(clean)

$ git log --oneline --abbrev=12 origin/main..HEAD | head
bdb0ab2a549d docs(core): handoff for SEC-36 history scan scope
aaa2cc4510d5 docs(security): document history-scan HEAD scoping and daily all-branch scan (SEC-36)
611e3fc5ebc5 ci(security): add daily all-branch history scan workflow (SEC-36)
60303bb90930 ci(security): scope history-scan to HEAD on main and PRs (SEC-36)
c8c8662db6ab docs(core): SEC-36 spec and plan; backlog status for SEC-16/17/18/27/36

$ git ls-remote --heads origin fix/sec-36-gitleaks-scope
bdb0ab2a549d5d4e567e92ca0f9c1f95c47b3739	refs/heads/fix/sec-36-gitleaks-scope
```

## 7. Review

(planner)
