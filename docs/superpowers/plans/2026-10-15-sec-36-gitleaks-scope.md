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

(developer appends here)

## 7. Review

(planner)
