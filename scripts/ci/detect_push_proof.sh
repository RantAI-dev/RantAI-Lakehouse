#!/usr/bin/env bash
# Decide whether a push to `main` only repeats a pull request's run (OPS-24 D4).
#
# A push is "proven" when all of the following hold for the pushed commit H:
#   1. the event is `push`, and the push's scope needs Rust tests or acceptance
#      jobs (nothing to skip otherwise);
#   2. a pull-request head P is found: H's second parent when H is a merge
#      commit; otherwise the head of the one merged pull request whose
#      `merge_commit_sha` is H and whose base is $BASE_BRANCH, fetched as
#      `refs/pull/<n>/head` and checked to be that exact commit (a squash merge
#      has one parent, and most merges here are squashes);
#   3. H's first parent is an ancestor of P: the branch held `main`'s tip when
#      it was merged, so the run that tested P tested H's tree;
#   4. H's tree equals P's tree;
#   5. the most recently completed `ci-required` check run on P, made by GitHub
#      Actions, is a success and none is still running;
#   6. in that check suite, the job that stands for each group the push needs
#      succeeded: `Rust · fmt` for the Rust group, `Images · build lakehouse-api
#      + dagster once` for the acceptance group. `required_gate.sh` makes a
#      group all-or-nothing, so one job of the group plus a green `ci-required`
#      proves the group, and this closes the case where the pull-request run had
#      a narrower scope than the push.
#
# Anything else is "not proven", and the caller then runs everything in scope.
# That is the fail-closed answer: an error, an empty answer, an unexpected
# value, a failed API call or an API call that took longer than 60 seconds
# never turns into a skip (OPS-24 R6d: every `gh` call is wrapped in
# `timeout 60`).
#
# Inputs (environment):
#   EVENT_NAME     github.event_name
#   PUSH_SHA       the pushed commit, github.sha (must be 40 hex digits)
#   REPO           owner/name, github.repository
#   BASE_BRANCH    the branch pushed to; default `main`
#   SCOPE_RUST, SCOPE_DAGSTER, SCOPE_STACK, SCOPE_ALL  the push's change scope
#   GH_TOKEN       read by `gh`
#   GITHUB_OUTPUT  optional; `proven`, `proven_by`, `proven_pr` and `reason` are
#                  appended to it
#
# Output: one line `proven=<true|false> proven_by=<sha> proven_pr=<n> reason=<text>`.
# The script always exits 0: "not proven" is an answer, not an error.
set -euo pipefail

# These two must equal the `name:` of the `fmt` and `images` jobs in
# `.github/workflows/ci.yml`; `scripts/ci/tests/test_detect_push_proof.sh`
# checks that they do.
RUST_SENTINEL="Rust · fmt"
ACCEPTANCE_SENTINEL="Images · build lakehouse-api + dagster once"

EVENT="${EVENT_NAME:-}"
SHA="${PUSH_SHA:-}"
REPO_NAME="${REPO:-}"
BASE_BRANCH="${BASE_BRANCH:-main}"

# $GITHUB_OUTPUT is line-oriented and read by the workflow, so every value
# written to it is reduced to one line. A check run's conclusion comes from the
# API and is not validated by GitHub; R6a is about the case where such a value
# holds a line break.
one_line() {
  printf '%s' "${1:-}" | tr -d '\r\n'
}

# Print the answer, write it to $GITHUB_OUTPUT when there is one, and stop.
finish() {
  local proven head pr reason
  proven="$(one_line "${1:-}")"
  head="$(one_line "${2:-}")"
  pr="$(one_line "${3:-}")"
  reason="$(one_line "${4:-}")"
  echo "proven=$proven proven_by=$head proven_pr=$pr reason=$reason"
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
      echo "proven=$proven"
      echo "proven_by=$head"
      echo "proven_pr=$pr"
      echo "reason=$reason"
    } >> "$GITHUB_OUTPUT"
  fi
  exit 0
}

# 1. Only a push can repeat a pull request's run. No `git`, no `gh`.
if [ "$EVENT" != "push" ]; then
  finish false "" "" "not a push event"
fi

# 2. A push that needs neither Rust tests nor acceptance jobs has nothing this
# proof could excuse, so it is not proven and no API call is made.
need_rust="false"
if [ "${SCOPE_RUST:-false}" = "true" ] || [ "${SCOPE_ALL:-false}" = "true" ]; then
  need_rust="true"
fi
need_acc="false"
if [ "$need_rust" = "true" ] || [ "${SCOPE_DAGSTER:-false}" = "true" ] || [ "${SCOPE_STACK:-false}" = "true" ]; then
  need_acc="true"
fi
if [ "$need_rust" = "false" ] && [ "$need_acc" = "false" ]; then
  finish false "" "" "no Rust test or acceptance job in this push's scope"
fi

# 3. The inputs are validated before any of them reaches a command or an API
# path: a SHA is 40 hex digits, the repository is `owner/name` (OPS-24 D7).
if ! [[ "$SHA" =~ ^[0-9a-f]{40}$ ]]; then
  finish false "" "" "the pushed commit is not a 40-hex-digit SHA"
fi
if ! git cat-file -e "${SHA}^{commit}" 2>/dev/null; then
  finish false "" "" "the pushed commit is not readable in this checkout"
fi
if ! [[ "$REPO_NAME" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]]; then
  finish false "" "" "the repository name is not owner/name"
fi

# 4. Find the pull-request head the push repeats.
if ! parents="$(git rev-list --parents -n 1 "$SHA" 2>/dev/null)"; then
  finish false "" "" "git could not read the pushed commit's parents"
fi
read -r -a parent_fields <<< "$parents"
parent_count=$(( ${#parent_fields[@]} - 1 ))
case "$parent_count" in
  2)
    # A merge commit: its second parent is the pull-request head.
    first="${parent_fields[1]}"
    head="${parent_fields[2]}"
    pr_number=""
    ;;
  1)
    # A squash (or a single-parent commit): ask the API which merged pull
    # request names this commit as its merge commit.
    first="${parent_fields[1]}"
    pulls=""
    pulls_rc=0
    pulls="$(timeout 60 gh api "repos/${REPO_NAME}/commits/${SHA}/pulls" 2>/dev/null)" || pulls_rc=$?
    if [ "$pulls_rc" -eq 124 ]; then
      finish false "" "" "the GitHub API did not answer within 60 seconds when asked for the pull requests of this commit"
    fi
    if [ "$pulls_rc" -ne 0 ]; then
      finish false "" "" "the GitHub API could not be asked for the pull requests of this commit"
    fi
    if ! selection="$(printf '%s' "$pulls" | jq -r --arg sha "$SHA" --arg base "$BASE_BRANCH" '
        [ .[] | select(.merge_commit_sha == $sha and .merged_at != null and .base.ref == $base) ]
        | if length == 0 then "none"
          elif length == 1 then "one \(.[0].number) \(.[0].head.sha)"
          else "many \(length)" end' 2>/dev/null)"; then
      finish false "" "" "the pull-request list from the API is not JSON this script can read"
    fi
    case "$selection" in
      none)
        finish false "" "" "no merged pull request has this commit as its merge commit"
        ;;
      many*)
        finish false "" "" "more than one merged pull request names this commit as its merge commit"
        ;;
      "one "*)
        read -r _ pr_number head <<< "$selection"
        ;;
      *)
        finish false "" "" "the pull-request list from the API is not JSON this script can read"
        ;;
    esac
    if ! [[ "$pr_number" =~ ^[0-9]+$ ]]; then
      finish false "" "" "the pull request the API named has no usable number"
    fi
    if ! [[ "$head" =~ ^[0-9a-f]{40}$ ]]; then
      finish false "" "" "the pull request the API named has no usable head commit"
    fi
    # The API names a commit; the checkout has to hold that same commit under
    # that pull request's ref, or the trees compared below are not the ones the
    # pull-request run tested.
    if ! git fetch --quiet origin "refs/pull/${pr_number}/head" 2>/dev/null; then
      finish false "" "" "the pull request's head could not be fetched"
    fi
    if ! fetched="$(git rev-parse FETCH_HEAD 2>/dev/null)"; then
      finish false "" "" "the fetched pull-request head is not a commit"
    fi
    if [ "$fetched" != "$head" ]; then
      finish false "" "" "the fetched pull-request head is not the head the API named"
    fi
    ;;
  *)
    finish false "" "" "the pushed commit is not a merge commit with one or two parents"
    ;;
esac

# 5. The first parent is the tip of `main` the branch was merged into; `main`
# only moves forward, so the pull-request run's merge of P with `main` was P.
ancestor_rc=0
git merge-base --is-ancestor "$first" "$head" 2>/dev/null || ancestor_rc=$?
if [ "$ancestor_rc" -eq 1 ]; then
  finish false "" "" "the pull-request head did not contain the tip of main it was merged into"
elif [ "$ancestor_rc" -ne 0 ]; then
  finish false "" "" "git could not tell whether the pull-request head contained the tip of main"
fi

# 6. The pushed tree has to be the tree the pull-request run tested.
if ! push_tree="$(git rev-parse "${SHA}^{tree}" 2>/dev/null)"; then
  finish false "" "" "the pushed commit's tree could not be read"
fi
if ! head_tree="$(git rev-parse "${head}^{tree}" 2>/dev/null)"; then
  finish false "" "" "the pull-request head's tree could not be read"
fi
if [ "$push_tree" != "$head_tree" ]; then
  finish false "" "" "the pushed tree differs from the pull-request head's tree"
fi

# 7. The pull request's own `ci-required` run has to be a completed success, and
# the job that stands for each group this push needs has to have run in it.
runs=""
runs_rc=0
runs="$(timeout 60 gh api --paginate "repos/${REPO_NAME}/commits/${head}/check-runs?per_page=100" 2>/dev/null)" || runs_rc=$?
if [ "$runs_rc" -eq 124 ]; then
  finish false "" "" "the GitHub API did not answer within 60 seconds when asked for the check runs of the pull-request head"
fi
if [ "$runs_rc" -ne 0 ]; then
  finish false "" "" "the GitHub API could not be asked for the check runs of the pull-request head"
fi
if ! verdict="$(printf '%s' "$runs" | jq -s -r \
    --arg need_rust "$need_rust" \
    --arg need_acc "$need_acc" \
    --arg rust_name "$RUST_SENTINEL" \
    --arg acc_name "$ACCEPTANCE_SENTINEL" '
    # R6a: a conclusion this script does not know is reported as `unknown`,
    # never echoed. GitHub lists these values for a check run; anything else
    # (an empty string, null, a value with a line break in it) is not one of
    # them.
    def known_conclusion:
      ["success", "failure", "neutral", "cancelled", "skipped", "timed_out",
       "action_required", "stale", "startup_failure"];
    def said: . as $c | if (known_conclusion | index($c)) then $c else "unknown" end;
    [ .[].check_runs[] | select(.app.slug == "github-actions") ] as $runs
    | [ $runs[] | select(.name == "ci-required") ] as $gate
    | if ($gate | length) == 0 then "NOT no ci-required check run on the pull-request head"
      elif ($gate | any(.status != "completed")) then "NOT a ci-required check run on the pull-request head has not completed"
      else ($gate | map(.completed_at) | max) as $latest
        # R6b: a tie on `completed_at` between a success and anything else is
        # not proven. `sort_by(...) | last` would have picked whichever came
        # last in the order the API reported them in.
        | [$gate[] | select(.completed_at == $latest)] as $top
        | ($top | last) as $g
        | def ran($n): any($runs[]; .name == $n and .check_suite.id == $g.check_suite.id and .status == "completed" and .conclusion == "success");
          if ($top | any(.conclusion != "success")) then
            if ($top | length) > 1
            then "NOT more than one ci-required check run on the pull-request head finished at the same time and not all of them succeeded"
            else "NOT the latest ci-required on the pull-request head ended \($g.conclusion | said)"
            end
          elif ($need_rust == "true" and (ran($rust_name) | not)) then "NOT the pull-request run did not run the Rust jobs"
          elif ($need_acc == "true" and (ran($acc_name) | not)) then "NOT the pull-request run did not run the acceptance jobs"
          else "PROVEN \($g.id)"
          end
      end' 2>/dev/null)"; then
  finish false "" "" "the check runs from the API are not JSON this script can read"
fi

case "$verdict" in
  "PROVEN "*)
    gate_id="${verdict#PROVEN }"
    ;;
  "NOT "*)
    finish false "" "" "${verdict#NOT }"
    ;;
  *)
    finish false "" "" "the check-run filter returned an answer this script does not understand"
    ;;
esac
if ! [[ "$gate_id" =~ ^[0-9]+$ ]]; then
  finish false "" "" "the check-run filter returned a check-run id that is not a number"
fi

finish true "$head" "$pr_number" "tree ${push_tree} equals pull-request head ${head}, whose ci-required check run ${gate_id} succeeded"
