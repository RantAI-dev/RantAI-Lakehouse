"""Render the staging deployment's Keycloak realm import file at run time.

Why this exists
---------------
A Keycloak realm import is a static JSON file, but the realm the staging
deployment imports must carry three values that are NOT in the repository:

* the **client secret** of the confidential client the console signs in with
  (generated per deployment, and `AGENTS.md` rule 5 forbids a literal secret
  in this public repository);
* the **passwords** of the three test users, so a person can actually sign in
  while the provider is being set up;
* the **callback address** (`OIDC_REDIRECT_URI`) the provider must send the
  browser back to -- this deployment's own address, which rule 12 keeps out
  of the repository (`AGENTS.md`: no real hostnames, IPs or ports in code,
  docs or commit messages).

So the checked-in `staging-realm.template.json` holds the whole realm shape
with those three values empty, and this script copies the deployment's
generated values into it immediately before Keycloak starts.

It runs as the one-shot `keycloak-realm-init` compose service
(`ops/sso/docker-compose.staging-keycloak.override.yml`), which mounts this
directory read-only at `/template` and the shared volume at `/import`.
`keycloak` then imports `/import/lakehouse-realm.json` with `--import-realm`.

What it deliberately does not do
--------------------------------
It does not generate the secrets -- the deploy workflow writes them into
`.env` from repository secrets, so the same value reaches this renderer, the
API's `OIDC_CLIENT_SECRET` and the sign-in the operator performs. Generating
them here instead would leave the other two guessing.

Fail closed
-----------
A missing environment value, an empty rendered secret, a template whose
`lakehouse` client or users are absent, or a callback address that is not
an absolute `http(s)` URL all exit non-zero, so `keycloak` (whose
`depends_on: keycloak-realm-init: condition: service_completed_successfully`
never starts against a half-rendered realm).

What it deliberately does not do
--------------------------------
It does not touch the gate's own realm: `ops/sso/realm.template.json` and
`ops/sso/render_realm.py` are the SEC-3 gate's, unchanged by this file.
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

TEMPLATE = Path(os.environ.get("STAGING_REALM_TEMPLATE", "/template/staging-realm.template.json"))
OUTPUT = Path(os.environ.get("STAGING_REALM_OUTPUT", "/import/lakehouse-realm.json"))

# The confidential client the console signs in with. Its secret is the one
# generated value the template leaves empty.
CONFIDENTIAL_CLIENT_ID = "lakehouse"

# The users whose passwords the template leaves empty, and the environment
# variable carrying each one. The Keycloak username is the dict key.
USER_PASSWORDS = {
    "alice": "SSO_USER_A_PASSWORD",
    "bob": "SSO_USER_B_PASSWORD",
    "carol": "SSO_USER_C_PASSWORD",
}


def _require(name: str) -> str:
    """Return `name`'s environment value, or exit non-zero if unset/empty."""
    value = os.environ.get(name, "").strip()
    if not value:
        sys.exit(
            f"render_staging_realm: {name} is unset or empty; "
            "refusing to render a realm with no value the deployment needs"
        )
    return value


def _require_absolute_url(name: str, value: str) -> str:
    """Return `value` when it is an absolute http(s) URL, or exit non-zero.

    A callback address that is relative, or that names a scheme the provider
    cannot redirect to, makes Keycloak refuse every sign-in with
    "Invalid parameter: redirect_uri" -- so it is caught here, where the
    failure names the setting, rather than at the login form.
    """
    if not value.startswith(("http://", "https://")):
        sys.exit(
            f"render_staging_realm: {name} is not an absolute http(s) URL; "
            "the provider would refuse every sign-in with it"
        )
    if len(value) <= len("http://"):
        sys.exit(f"render_staging_realm: {name} names no host")
    return value


def main() -> int:
    realm = json.loads(TEMPLATE.read_text(encoding="utf-8"))

    # The realm's name is the last path segment of the issuer
    # (`{base}/realms/{realm}`), and the API compares the token's `iss` with
    # its configured issuer byte for byte -- so the realm's name and the
    # issuer's last segment have to be the same string. It is set here, from
    # the same value the deployment writes into `.env`, instead of being
    # read from the template and hoped to match.
    realm_name = os.environ.get("SSO_REALM", "").strip() or realm.get("realm", "")
    if not realm_name:
        sys.exit("render_staging_realm: the realm has no name and SSO_REALM is unset")
    realm["realm"] = realm_name

    secret = _require("OIDC_CLIENT_SECRET")
    redirect_uri = _require_absolute_url("SSO_REDIRECT_URI", _require("SSO_REDIRECT_URI"))
    post_logout_uri = _require_absolute_url(
        "SSO_POST_LOGOUT_REDIRECT_URI", _require("SSO_POST_LOGOUT_REDIRECT_URI")
    )
    passwords = {user: _require(var) for user, var in USER_PASSWORDS.items()}

    client = next(
        (c for c in realm.get("clients", []) if c.get("clientId") == CONFIDENTIAL_CLIENT_ID),
        None,
    )
    if client is None:
        sys.exit(f"render_staging_realm: template has no client {CONFIDENTIAL_CLIENT_ID!r}")
    client["secret"] = secret
    # The template carries one empty placeholder: the deployment's own
    # callback. Replacing the whole list keeps a stale address from
    # surviving in the imported realm.
    client["redirectUris"] = [redirect_uri]
    # The provider refuses a logout whose return address it does not know,
    # so the client's valid post-logout redirect URIs are the deployment's
    # own address -- filled in here, like the callback above.
    attributes = dict(client.get("attributes") or {})
    attributes["post.logout.redirect.uris"] = post_logout_uri
    client["attributes"] = attributes

    rendered_users: set[str] = set()
    for user in realm.get("users", []):
        username = user.get("username")
        if username not in passwords:
            continue
        credentials = user.get("credentials") or []
        if not credentials:
            sys.exit(f"render_staging_realm: user {username!r} has no credential entry to fill")
        credentials[0]["value"] = passwords[username]
        rendered_users.add(username)
    missing = set(passwords) - rendered_users
    if missing:
        sys.exit(f"render_staging_realm: template has no user(s) {sorted(missing)}")

    if not client.get("secret"):
        sys.exit("render_staging_realm: rendered client secret is empty")
    if not client.get("redirectUris"):
        sys.exit("render_staging_realm: rendered redirect URI list is empty")
    if not (client.get("attributes") or {}).get("post.logout.redirect.uris"):
        sys.exit("render_staging_realm: rendered post-logout redirect URI is empty")

    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(realm, indent=2) + "\n", encoding="utf-8")
    print(f"render_staging_realm: wrote {OUTPUT} for realm {realm.get('realm')!r}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
