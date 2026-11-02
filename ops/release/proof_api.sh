#!/usr/bin/env bash
# The authenticated API of the update proof (REL-2 S4 decision D21).
#
# WHY THIS SCRIPT EXISTS
#   The proof's third marker is a stored connector credential, and it is
#   read back through the API. That route is AUTHENTICATED: `GET
#   /api/connectors/{id}` needs the `connector:manage` permission, and a
#   call with no session answers 401. The marker check that ran after the
#   REAL update to bundle B failed exactly there — "curl: (22) The
#   requested URL returned error: 401" — because it called the API with
#   no session at all (PR #335 update-proof, marker check 401).
#
#   Every API call the proof makes goes through THIS script, so that no
#   step can forget the sign-in: the script signs in, keeps the session
#   in a cookie jar, and makes the call with the jar.
#
# WHY THE SIGN-IN IS PER CALL
#   The update restarts the stack, and the rollback restarts it again. A
#   session made before an update is not a session to rely on after it,
#   so every call signs in again: one request, and every step is
#   independent of the one before it.
#
# USAGE
#   ops/release/proof_api.sh login
#   ops/release/proof_api.sh get  /api/connectors/<id>
#   ops/release/proof_api.sh post /api/connectors '<json>'
#
# WHAT IT READS
#   - `install/.env`: the bootstrap administrator's email
#     (`AUTH_BOOTSTRAP_EMAIL`);
#   - the password file: the account's CURRENT password. It is the
#     bootstrap password the install printed at first, and the ROTATED
#     one after the proof rotates it — the rotation replaces the
#     password and revokes every session, so the old one is refused
#     afterwards.
#
# The defaults can be overridden with `PROOF_API_URL`,
# `PROOF_API_ENV_FILE`, `PROOF_API_PASSWORD_FILE` and
# `PROOF_API_COOKIE_JAR`.
set -euo pipefail

API_URL="${PROOF_API_URL:-http://127.0.0.1:8080}"
ENV_FILE="${PROOF_API_ENV_FILE:-install/.env}"
PASSWORD_FILE="${PROOF_API_PASSWORD_FILE:-proof-password.txt}"
COOKIE_JAR="${PROOF_API_COOKIE_JAR:-proof-cookies.txt}"

say() {
    # On STDERR, and not on stdout: the response body is what the
    # caller reads from stdout, and a status line in the middle of it
    # would break the caller's own `jq`.
    echo "proof_api: $*" >&2
}

fail() {
    echo "proof_api: $*" >&2
    exit 1
}

usage() {
    echo "usage: proof_api.sh login | get <path> | post <path> <json>" >&2
    exit 2
}

require_command() {
    command -v "$1" > /dev/null 2>&1 || fail "$1 is needed and is not installed."
}

# The bootstrap administrator's email, from the install's own `.env`.
#
# The email is the account's identity, and it does not change when the
# password is rotated — so the `.env` stays the right place to read it
# from, after the update and after the rollback.
bootstrap_email() {
    local email
    email="$(grep -E '^AUTH_BOOTSTRAP_EMAIL=' "$ENV_FILE" | cut -d= -f2- | tr -d '"' || true)"
    [ -n "$email" ] || fail "the file $ENV_FILE names no AUTH_BOOTSTRAP_EMAIL."
    printf '%s' "$email"
}

# The account's current password.
current_password() {
    local password
    [ -f "$PASSWORD_FILE" ] || fail "the file $PASSWORD_FILE holds no password."
    password="$(tr -d '\r\n' < "$PASSWORD_FILE")"
    [ -n "$password" ] || fail "the file $PASSWORD_FILE holds an empty password."
    printf '%s' "$password"
}

# Sign in, and keep the session in the cookie jar.
#
# The payload is built with `jq -n --arg`: the password is a secret, and
# a secret is never spliced into a string by the shell.
sign_in() {
    local email password payload
    email="$(bootstrap_email)"
    password="$(current_password)"
    payload="$(jq -n --arg email "$email" --arg password "$password" \
        '{email: $email, password: $password}')"
    curl -fsS -c "$COOKIE_JAR" -X POST "$API_URL/api/auth/login" \
        -H 'content-type: application/json' \
        -d "$payload" > /dev/null \
        || fail "the sign-in was refused."
    say "signed in."
}

require_command curl
require_command jq

if [ "$#" -lt 1 ]; then
    usage
fi

command="$1"
case "$command" in
    login)
        sign_in
        ;;
    get)
        [ "$#" -eq 2 ] || usage
        path="$2"
        sign_in
        curl -fsS -b "$COOKIE_JAR" "$API_URL$path" \
            || fail "the API refused the call to $path."
        ;;
    post)
        [ "$#" -eq 3 ] || usage
        path="$2"
        body="$3"
        sign_in
        curl -fsS -b "$COOKIE_JAR" -X POST "$API_URL$path" \
            -H 'content-type: application/json' \
            -d "$body" \
            || fail "the API refused the call to $path."
        ;;
    *)
        usage
        ;;
esac
