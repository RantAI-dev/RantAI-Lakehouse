"""Minimal OIDC discovery + JWKS server for R1 (Lakekeeper authorization).

Not a real identity provider. It exists for exactly one reason: Lakekeeper's
`openid_provider_uri` authenticator needs *something* that (a) serves
OIDC discovery + JWKS documents so Lakekeeper can validate a bearer JWT's
signature, and (b) whose private key this container also holds, so it can
pre-mint one long-lived RS256 token per principal this stack needs to
identify (see PRINCIPALS below). Every token is also minted once at
container start and written to a shared volume other compose services
read directly from disk, for writers that accept a static bearer token
outright (`iceberg-catalog-rest`'s `token` property, Debezium, dlt).

ClickHouse's `DataLakeCatalog` REST engine is the one writer in this build
that does NOT accept a static bearer token — its `catalog_credential`
setting is hard-coded to the Iceberg REST spec's OAuth2
`client_id:client_secret` exchange (measured empirically against 26.3: a
bare token is rejected at parse time with "expected client_id and
client_secret separated by `:`"). `/token` below exists ONLY for that one
caller, pointed at directly via ClickHouse's `oauth_server_uri` setting
(bypassing Lakekeeper's own `/v1/oauth/tokens`, whose behavior in
openfga/openid mode is unverified — see ADR 0011). It is a client-
credentials grant in name only: it does not check `client_secret` against
anything (there is nothing else in this stack it could check it against),
it only requires `client_id` to name one of TOKEN_ENDPOINT_PRINCIPALS
(every PRINCIPAL except `admin`, which this endpoint refuses to mint —
see that list's own comment) and mints that principal's token on demand.
This is a deliberate simplification for a self-hosted dev/CI stack, not a
production identity provider; see ADR 0011.

Idempotent: if keys/tokens already exist on the shared volume (a restart,
not a fresh volume), they are reused rather than regenerated, so already-
granted principal identities (their `sub` claims) do not change under a
container restart.
"""

from __future__ import annotations

import base64
import http.server
import json
import os
import socketserver
import time
import urllib.parse
from pathlib import Path

import jwt
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives import serialization

DATA_DIR = Path(os.environ.get("OIDC_MOCK_DATA_DIR", "/data"))
# The HTTP handler below only ever serves PUBLIC_DIR, never DATA_DIR itself.
# `private_key.pem` lives directly in DATA_DIR (a sibling of PUBLIC_DIR, not
# a parent of it), so there is no path under the served document root that
# reaches it. See `ensure_keys`/`Handler` — PR #33 review, blocker 1: the
# key used to sit inside the served directory tree and was fetchable over
# the published host port.
PUBLIC_DIR = DATA_DIR / "public"
TOKENS_DIR = Path(os.environ.get("OIDC_MOCK_TOKENS_DIR", "/tokens"))
ISSUER = os.environ["OIDC_MOCK_ISSUER"]  # e.g. http://oidc-mock:8090
AUDIENCE = os.environ.get("OIDC_MOCK_AUDIENCE", "lakekeeper")
KEY_ID = "oidc-mock-1"
# 30 days, not 10 years (PR #33 review, blocker 2): this mock still has no
# token-refresh mechanism (see module doc), so the lifetime has to outlive
# a normal dev/CI stack's uptime without operator intervention. 30 days
# comfortably covers a sprint-length local stack or a long-running demo
# environment while bounding how long a leaked token stays valid to weeks,
# not a decade. A stack left running past that needs `oidc-mock` restarted
# with a fresh `lakehouse_oidc_tokens` volume anyway (see docs/OPERATIONS.md).
TOKEN_LIFETIME_SECONDS = 30 * 24 * 3600

# Every principal this build needs Lakekeeper to recognize under
# enforcement, plus one deliberately ungranted principal for the negative
# test (deliverable 5 of the R1 task). `admin` is the bootstrap/instance
# admin identity (LAKEKEEPER__INSTANCE_ADMINS) used only by the one-shot
# init jobs that bootstrap Lakekeeper and grant the others — it is not
# used by any long-running writer.
PRINCIPALS = [
    "admin",
    "rust-iceberg",
    "debezium",
    "dlt",
    "clickhouse-reader",
    "trino",
    # ADR 0010 — Gold export to Iceberg from Rust: `lakehouse-api`'s
    # `routes::gold` reads a Gold mart from ClickHouse and appends it to
    # the `gold` Iceberg namespace through Lakekeeper, the same
    # write/read-back shape `rust-iceberg` already proves for Bronze in
    # G1. A separate principal (not a reuse of `rust-iceberg`) so this
    # identity is independently auditable/revocable even though ADR
    # 0011's grants are warehouse-scoped (not per-namespace) — the same
    # identity-separation rationale ADR 0011 gives for keeping
    # `rust-iceberg`/`debezium`/`dlt` as three principals instead of one,
    # despite all three holding the same relation set today.
    "gold-export",
    "unauthorized-test",
]

# `/token` (the client-credentials endpoint, below) must never mint an
# `admin` token: `admin` is Lakekeeper's instance-admin bypass identity
# (LAKEKEEPER__INSTANCE_ADMINS=["oidc~admin"]), and this endpoint has no
# secret check at all (see its own docstring) — anyone who can reach it
# could otherwise mint an admin-bypass token on demand. The one-shot init
# jobs that need `admin` (`lakekeeper-warehouse-init`, `lakekeeper-authz-
# init`) already read `admin.jwt` straight off the shared token volume
# `mint_tokens` below writes to disk at container start; they never call
# `/token`. PR #33 review, blocker 2.
TOKEN_ENDPOINT_PRINCIPALS = [p for p in PRINCIPALS if p != "admin"]


def b64url_uint(value: int) -> str:
    raw = value.to_bytes((value.bit_length() + 7) // 8, "big")
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode("ascii")


def ensure_keys() -> rsa.RSAPrivateKey:
    # Deliberately DATA_DIR, not PUBLIC_DIR: this is the private signing
    # key, and PUBLIC_DIR is the only directory the HTTP handler serves.
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    key_path = DATA_DIR / "private_key.pem"
    if key_path.exists():
        return serialization.load_pem_private_key(key_path.read_bytes(), password=None)
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    key_path.write_bytes(
        key.private_bytes(
            encoding=serialization.Encoding.PEM,
            format=serialization.PrivateFormat.PKCS8,
            encryption_algorithm=serialization.NoEncryption(),
        )
    )
    return key


def write_jwks(key: rsa.RSAPrivateKey) -> None:
    PUBLIC_DIR.mkdir(parents=True, exist_ok=True)
    pub = key.public_key().public_numbers()
    jwks = {
        "keys": [
            {
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": KEY_ID,
                "n": b64url_uint(pub.n),
                "e": b64url_uint(pub.e),
            }
        ]
    }
    (PUBLIC_DIR / "jwks.json").write_text(json.dumps(jwks))


def write_discovery() -> None:
    doc = {
        "issuer": ISSUER,
        "jwks_uri": f"{ISSUER}/jwks.json",
        "authorization_endpoint": f"{ISSUER}/authorize",
        # PyJWT/most OIDC-aware clients look this up before trying a
        # hardcoded path; Lakekeeper's own OIDC client is one of them.
        "token_endpoint": f"{ISSUER}/token",
        "response_types_supported": ["id_token"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
    }
    well_known = PUBLIC_DIR / ".well-known"
    well_known.mkdir(parents=True, exist_ok=True)
    (well_known / "openid-configuration").write_text(json.dumps(doc))


def mint_tokens(key: rsa.RSAPrivateKey) -> None:
    TOKENS_DIR.mkdir(parents=True, exist_ok=True)
    private_pem = key.private_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PrivateFormat.PKCS8,
        encryption_algorithm=serialization.NoEncryption(),
    )
    now = int(time.time())
    for sub in PRINCIPALS:
        token_path = TOKENS_DIR / f"{sub}.jwt"
        if token_path.exists():
            continue
        claims = {
            "iss": ISSUER,
            "aud": AUDIENCE,
            "sub": sub,
            "iat": now,
            "exp": now + TOKEN_LIFETIME_SECONDS,
        }
        token = jwt.encode(
            claims, private_pem, algorithm="RS256", headers={"kid": KEY_ID}
        )
        token_path.write_text(token)


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, private_key=None, **kwargs):
        self._private_key = private_key
        # PUBLIC_DIR only — never DATA_DIR, which also holds
        # private_key.pem. See the DATA_DIR/PUBLIC_DIR module comment.
        super().__init__(*args, directory=str(PUBLIC_DIR), **kwargs)

    def end_headers(self):
        self.send_header("Content-Type", "application/json")
        super().end_headers()

    def log_message(self, fmt, *args):  # quieter than the stdlib default
        pass

    def do_POST(self):  # noqa: N802 (stdlib method name)
        if self.path != "/token":
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b'{"error":"not_found"}')
            return
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode("utf-8")
        form = urllib.parse.parse_qs(body)
        client_id = (form.get("client_id") or [None])[0]
        # Client-credentials grant in name only — see module doc: this
        # exists purely so ClickHouse's `oauth_server_uri` has something to
        # call, and does not check `client_secret` against anything.
        # Deliberately TOKEN_ENDPOINT_PRINCIPALS, not PRINCIPALS: `admin`
        # must never be mintable through this unauthenticated endpoint (PR
        # #33 review, blocker 2) — see that list's own comment.
        if client_id not in TOKEN_ENDPOINT_PRINCIPALS:
            self.send_response(401)
            self.end_headers()
            self.wfile.write(
                json.dumps({"error": "invalid_client"}).encode("utf-8")
            )
            return
        token_path = TOKENS_DIR / f"{client_id}.jwt"
        access_token = token_path.read_text()
        self.send_response(200)
        self.end_headers()
        self.wfile.write(
            json.dumps(
                {
                    "access_token": access_token,
                    "token_type": "bearer",
                    "expires_in": TOKEN_LIFETIME_SECONDS,
                }
            ).encode("utf-8")
        )


def main() -> None:
    key = ensure_keys()
    write_jwks(key)
    write_discovery()
    mint_tokens(key)
    port = int(os.environ.get("OIDC_MOCK_PORT", "8090"))

    def handler_factory(*args, **kwargs):
        return Handler(*args, private_key=key, **kwargs)

    with socketserver.TCPServer(("0.0.0.0", port), handler_factory) as httpd:
        httpd.serve_forever()


if __name__ == "__main__":
    main()
