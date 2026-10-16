"""Render the SEC-3 gate's Keycloak realm import file at run time.

Why this exists
---------------
A Keycloak realm import is a static JSON file, but the realm the gate imports
must carry the *client secret* and the *two test users' passwords* the gate
generated for this run — and nothing in this repository may contain a literal
password or client secret (SEC-3 decision E2; `AGENTS.md` rule 5; the
repository is public, and gitleaks must stay clean). So the checked-in
`realm.template.json` holds the whole realm shape with empty secrets, and this
script copies the run's generated values into it immediately before Keycloak
starts.

It runs as the one-shot `keycloak-realm-init` compose service
(`ops/sso/docker-compose.keycloak.override.yml`), which mounts this directory
read-only at `/template` and the shared volume at `/import`. `keycloak` then
imports `/import/lakehouse-realm.json` with `--import-realm`.

What it deliberately does not do
--------------------------------
It does not generate the secrets — the caller (`ops/g10/g10_sso_gate.sh`, or
CI) writes them into `.env` once, so the same value reaches this renderer,
`lakehouse-api`'s `OIDC_CLIENT_SECRET`, and the gate's login steps. Generating
them here instead would leave the other two services guessing.

Fail closed: a missing environment value, an empty rendered secret, or a realm
template whose `lakehouse` client or users are absent all exit non-zero, so
`keycloak` (which `depends_on: keycloak-realm-init: condition:
service_completed_successfully`) never starts against a half-rendered realm.
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

TEMPLATE = Path("/template/realm.template.json")
OUTPUT = Path("/import/lakehouse-realm.json")

# The confidential client the console signs in with. Its secret is the one
# generated value the template leaves empty.
CONFIDENTIAL_CLIENT_ID = "lakehouse"

# Test users whose passwords the gate logs in with, and the environment
# variable carrying each one. The Keycloak username is the dict key.
USER_PASSWORDS = {
    "alice": "SSO_USER_A_PASSWORD",
    "bob": "SSO_USER_B_PASSWORD",
}


def _require(name: str) -> str:
    """Return `name`'s environment value, or exit non-zero if unset/empty."""
    value = os.environ.get(name, "").strip()
    if not value:
        sys.exit(f"render_realm: {name} is unset or empty; refusing to render a realm with no secret")
    return value


def main() -> int:
    realm = json.loads(TEMPLATE.read_text(encoding="utf-8"))

    secret = _require("OIDC_CLIENT_SECRET")
    passwords = {user: _require(var) for user, var in USER_PASSWORDS.items()}

    client = next(
        (c for c in realm.get("clients", []) if c.get("clientId") == CONFIDENTIAL_CLIENT_ID),
        None,
    )
    if client is None:
        sys.exit(f"render_realm: template has no client {CONFIDENTIAL_CLIENT_ID!r}")
    client["secret"] = secret

    rendered_users: set[str] = set()
    for user in realm.get("users", []):
        username = user.get("username")
        if username not in passwords:
            continue
        credentials = user.get("credentials") or []
        if not credentials:
            sys.exit(f"render_realm: user {username!r} has no credential entry to fill")
        credentials[0]["value"] = passwords[username]
        rendered_users.add(username)
    missing = set(passwords) - rendered_users
    if missing:
        sys.exit(f"render_realm: template has no user(s) {sorted(missing)}")

    if not client.get("secret"):
        sys.exit("render_realm: rendered client secret is empty")

    OUTPUT.write_text(json.dumps(realm, indent=2) + "\n", encoding="utf-8")
    print(f"render_realm: wrote {OUTPUT} for realm {realm.get('realm')!r}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
