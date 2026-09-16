"""dagster/dispar_orchestrate/adapters/sheets.py -- Google Sheets source
for a `sheets`-adapter connector. Ships `supported: false`
unconditionally in Tier 1 (WS3 plan review Z10, open question 4).

# What an earlier revision actually sent, and why it could never have worked

An earlier revision of this adapter POSTed the raw `service_account_json`
secret blob as the `assertion` form parameter of a token-endpoint request
(the shape a real Google OAuth2 JWT-bearer exchange uses). That blob is
the service account's *credentials file* -- `{"type":
"service_account", "private_key": "...", "client_email": "...", ...}` --
not a JWT. A JWT-bearer `assertion` has to be a base64url-encoded,
three-part `header.claims.signature` string, where the signature is
produced by signing the header+claims with the RSA private key embedded
in that same JSON blob (RFC 7523; `google-auth`'s
`service_account.Credentials._make_authorization_grant_assertion` is the
reference implementation of this exact step). Posting the raw JSON in
place of that string was never a working exchange that happened to be
insecure or untested -- Google's token endpoint validates the `assertion`
shape before it even looks at a signature, so the earlier revision's
request would have been rejected outright by
`https://oauth2.googleapis.com/token` every single time. It only *looked*
complete, because it built a request and would have gotten an HTTP
response back -- just always an error one.

# What is actually installed here, checked directly (not assumed)

Verified in this branch's Dagster venv (`~/.cache/rantai-dagster-venv`,
`pip list` plus a direct `import`, both run for this task):

- `import google.auth` / `import google.oauth2` both raise
  `ModuleNotFoundError` -- the only Google package present is `protobuf`
  (a transitive dependency of something else in this environment, not a
  Google API client). `google-auth`'s `service_account.Credentials` is
  the library that would build and sign the claim set correctly; it is
  not here.
- No crypto-signing-capable library is installed either: neither `PyJWT`
  nor `cryptography` nor `python-jose` appear in `pip list`, so there is
  no way to hand-roll the RS256 signature RFC 7523 requires without first
  adding a new dependency. (`rust/crates/lakehouse-api/src/connector_probe.rs`'s
  `probe_sheets` doc comment makes the equivalent, corrected check for
  the Rust side: `jsonwebtoken = "9"` IS already a workspace dependency
  there, but only used by `lakehouse-auth`'s `oidc` module to VERIFY
  incoming tokens, never to MINT a service-account assertion -- so
  neither side of this workspace has ever run a real signing call
  against a real service account. The Python side is the simpler case:
  it has no signing-capable library present at all, verified library by
  library.)

Rather than pin a new cryptography-adjacent dependency and write a
signing call this task cannot verify against a real Google service
account, `sheets` ships `supported: false` unconditionally in Tier 1 --
"unsupported, honestly" (AGENTS.md rule 14) over a crypto code path
nobody has run for real. This module therefore makes NO network call and
NO `ssrf_guard.resolve_checked` call at all: there is nothing to dial
because no assertion can be built to authenticate the dial with.

# `connector_type.supported = true` is a different, coarser claim

`connector_type` (`rust/migrations/0035_connector_type.sql`)
seeds `('Google Sheets', 'sheets', true, NULL)`. That `supported` column
means "the adapter and wizard exist" -- a user can create a `sheets`
connector, save an ingest spec for it, and see it in the UI. This
module's own per-call `AdapterBuildResult.supported = False` is a
strictly finer-grained signal: the adapter exists, but no run of it can
ever move data, because the one thing standing between "the object
exists" and "it can authenticate" -- a signed JWT-bearer assertion -- has
no implementation. A reader who only checks `connector_type.supported`
would reasonably conclude Google Sheets ingestion works today; it does
not, for any `sheets` connector, regardless of configuration. Flagging
this pairing as potentially misleading is this task's own judgment call,
not a decision it is positioned to resolve -- the fix (adding a
`connector_type`-level "wizard only" distinction, or gating creation
behind a warning) is a product call for a follow-up workstream.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

_UNSUPPORTED_REASON = (
    "no verified Google service-account token exchange exists: google.auth and "
    "google.oauth2 are not installed in this Dagster environment, and no crypto-signing "
    "library (PyJWT, cryptography) is installed either to sign a JWT-bearer assertion "
    "with the service account's RSA private key (RFC 7523) -- a prior revision posted "
    "the raw service-account JSON as the assertion instead of a signed JWT, which "
    "Google's token endpoint would reject outright, not a working exchange"
)


@dataclass(frozen=True)
class AdapterBuildResult:
    """Mirrors `rest.py`/`files.py`'s own `AdapterBuildResult` shape
    (`source`, plus adapter-specific fields) -- here `supported`/`reason`
    replace `resolved`, since this adapter never resolves or dials a
    host at all."""

    source: Any
    supported: bool
    reason: str | None = None


def build_source(spec: dict, secrets: dict[str, str]) -> AdapterBuildResult:
    """Always reports `supported: false` -- see the module doc comment.

    Never inspects `spec`/`secrets` beyond accepting them (matching
    `sql.py`/`files.py`/`rest.py`'s call shape): there is no code path
    here that could use a `serviceAccountJson` secret to authenticate
    anything, so pretending to examine one would only invite a reader to
    believe some configurations succeed where none do.
    """
    del spec, secrets
    return AdapterBuildResult(source=iter(()), supported=False, reason=_UNSUPPORTED_REASON)
