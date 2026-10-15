#!/usr/bin/env bash
# Runs the standard set of dashboard requests against a throwaway local stack
# and says pass or fail. The stack belongs to this script: its own compose
# project name, its own host ports, and it is always torn down, including on
# an error or an interrupt.
#
# This is a person's run, not CI's (owner, 2026-10-15). Run it on every change
# to an assistant tool or prompt and put the numbers in that PR's review; the
# results are not kept in the repository.
#
# Usage:
#   LLM_KEY=<key> ops/ai_eval/run_dashboard_eval.sh [--baseline results.json]
#                                            [--out results.json] [--image tag]
#
# Env the caller sets (all optional except LLM_KEY):
#   LLM_KEY         required; no model, no run
#   LLM_URL         model endpoint (default: compose's)
#   LLM_MODEL       model name    (default: compose's)
#   API_HOST_PORT   default 27080; CH_HTTP_HOST_PORT 27123;
#   CH_NATIVE_HOST_PORT 27900; POSTGRES_HOST_PORT 27432
# Any AI_* variable (AI_EVAL_TIMEOUT_S and so on) is passed to the harness.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

usage() {
    sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

BASELINE=""
OUT=""
IMAGE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --baseline) BASELINE="${2:?--baseline needs a file}"; shift 2 ;;
        --out)      OUT="${2:?--out needs a file}"; shift 2 ;;
        --image)    IMAGE="${2:?--image needs a tag}"; shift 2 ;;
        -h|--help)  usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

# No key, no run: the chat answers 503 without one and every case would fail
# for the same wrong reason. A missing key is a setup error, so exit 2.
if [ -z "${LLM_KEY:-}" ]; then
    echo "run_dashboard_eval: LLM_KEY must be set (the model the assistant calls)" >&2
    exit 2
fi

PROJECT="ai-eval-$(date +%s)"
API_HOST_PORT="${API_HOST_PORT:-27080}"
CH_HTTP_HOST_PORT="${CH_HTTP_HOST_PORT:-27123}"
CH_NATIVE_HOST_PORT="${CH_NATIVE_HOST_PORT:-27900}"
POSTGRES_HOST_PORT="${POSTGRES_HOST_PORT:-27432}"

# Refuse a port the dev stack or another run already holds, so this run never
# joins or breaks someone else's stack.
port_busy() {
    # `ss` is not always present; fall back to a failed bind check.
    if command -v ss >/dev/null 2>&1; then
        ss -ltn "sport = :$1" 2>/dev/null | tail -n +2 | grep -q .
    else
        (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null && { exec 3<&- ; return 0; } || return 1
    fi
}
for pair in "API:$API_HOST_PORT" "CH_HTTP:$CH_HTTP_HOST_PORT" "CH_NATIVE:$CH_NATIVE_HOST_PORT" "POSTGRES:$POSTGRES_HOST_PORT"; do
    name="${pair%%:*}"; port="${pair##*:}"
    if port_busy "$port"; then
        echo "run_dashboard_eval: port $port ($name) is already in use; set a different one or stop that stack." >&2
        exit 2
    fi
done

# Throwaway credentials, generated per run and never written to the repository.
ADMIN_EMAIL="ai-eval@eval.example.com"
ADMIN_PASSWORD="$(head -c 24 /dev/urandom | base64 | tr -d '/+=')"
CH_USER="ai_eval"
CH_PASSWORD="$(head -c 18 /dev/urandom | base64 | tr -d '/+=')"

cleanup() {
    local code=$?
    echo "run_dashboard_eval: tearing down $PROJECT" >&2
    COMPOSE_PROJECT_NAME="$PROJECT" docker compose -p "$PROJECT" down -v --remove-orphans >/dev/null 2>&1 || true
    exit "$code"
}
trap cleanup EXIT INT TERM

dc() { docker compose -p "$PROJECT" "$@"; }

export API_HOST_PORT CH_HTTP_HOST_PORT CH_NATIVE_HOST_PORT POSTGRES_HOST_PORT
export AUTH_BOOTSTRAP_EMAIL="$ADMIN_EMAIL" AUTH_BOOTSTRAP_PASSWORD="$ADMIN_PASSWORD"
export CH_USER CH_PASSWORD
export LLM_URL="${LLM_URL:-https://api.minimax.io/v1}" LLM_MODEL="${LLM_MODEL:-MiniMax-M3}" LLM_KEY

echo "run_dashboard_eval: starting $PROJECT (api :$API_HOST_PORT)" >&2
if [ -n "$IMAGE" ]; then
    LAKEHOUSE_API_IMAGE="$IMAGE" dc up -d --wait postgres clickhouse oidc-mock lakehouse-api
else
    dc up -d --wait postgres clickhouse oidc-mock lakehouse-api
fi

echo "run_dashboard_eval: seeding ClickHouse" >&2
COMPOSE_PROJECT="$PROJECT" scripts/seed-clickhouse.sh

# Wait for the API's own liveness route (unauthenticated) to answer.
echo "run_dashboard_eval: waiting for the API" >&2
for _ in $(seq 1 60); do
    if curl -fsS "http://127.0.0.1:${API_HOST_PORT}/health" >/dev/null 2>&1; then
        break
    fi
    sleep 2
done
if ! curl -fsS "http://127.0.0.1:${API_HOST_PORT}/health" >/dev/null 2>&1; then
    echo "run_dashboard_eval: the API never answered /health" >&2
    exit 2
fi

# The harness logs in as the bootstrap admin and calls ClickHouse directly for
# the ground truth; the seed's credentials and ports are passed through.
export AI_EVAL_EMAIL="$ADMIN_EMAIL" AI_EVAL_PASSWORD="$ADMIN_PASSWORD"
export AI_EVAL_API_URL="http://127.0.0.1:${API_HOST_PORT}"
export AI_EVAL_CH_URL="http://127.0.0.1:${CH_HTTP_HOST_PORT}"

set -- --cases "$SCRIPT_DIR/dashboard_cases.json" --repeat 3 --need 2 --label "dashboard"
[ -n "$BASELINE" ] && set -- "$@" --baseline "$BASELINE"
[ -n "$OUT" ] && set -- "$@" --out "$OUT"

python3 "$SCRIPT_DIR/ai_eval.py" "$@"
exit $?
