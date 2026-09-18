"""Unit tests for oidc-mock's authorization-code + PKCE addition (WS8 plan
Phase A).

No network, per AGENTS.md's Python rules: these import `server.py` and call
`mint_authorization_code`/`exchange_authorization_code` directly, which is
where the flow's real decisions live (PKCE comparison, redirect_uri binding,
single use, expiry). The HTTP handlers around them are thin parameter
plumbing, and the round trip over real sockets is covered by the g4 gate
against the running container, not here."""
from __future__ import annotations

import base64
import hashlib
import json
import unittest

import server as oidc_mock


def _pkce_pair() -> tuple[str, str]:
    verifier = base64.urlsafe_b64encode(b"0" * 32).rstrip(b"=").decode()
    challenge = base64.urlsafe_b64encode(
        hashlib.sha256(verifier.encode()).digest()
    ).rstrip(b"=").decode()
    return verifier, challenge


class AuthorizationCodeFlowTests(unittest.TestCase):
    def test_authorize_mints_a_code_bound_to_the_pkce_challenge_and_nonce(self):
        verifier, challenge = _pkce_pair()
        code = oidc_mock.mint_authorization_code(
            sub="test-analyst",
            redirect_uri="http://lakehouse-api:8080/api/auth/oidc/callback",
            nonce="nonce-abc",
            code_challenge=challenge,
        )
        record = oidc_mock.AUTH_CODES[code]
        self.assertEqual(record["sub"], "test-analyst")
        self.assertEqual(record["nonce"], "nonce-abc")

    def test_exchange_rejects_a_mismatched_code_verifier(self):
        verifier, challenge = _pkce_pair()
        code = oidc_mock.mint_authorization_code(
            sub="test-analyst",
            redirect_uri="http://x/callback",
            nonce="n",
            code_challenge=challenge,
        )
        with self.assertRaises(oidc_mock.PkceMismatch):
            oidc_mock.exchange_authorization_code(
                code=code,
                redirect_uri="http://x/callback",
                code_verifier="wrong-verifier",
            )

    def test_exchange_rejects_a_redirect_uri_that_does_not_match_authorize(self):
        verifier, challenge = _pkce_pair()
        code = oidc_mock.mint_authorization_code(
            sub="test-analyst",
            redirect_uri="http://x/callback",
            nonce="n",
            code_challenge=challenge,
        )
        with self.assertRaises(oidc_mock.RedirectUriMismatch):
            oidc_mock.exchange_authorization_code(
                code=code,
                redirect_uri="http://attacker.invalid/callback",
                code_verifier=verifier,
            )

    def test_a_code_can_only_be_exchanged_once(self):
        verifier, challenge = _pkce_pair()
        code = oidc_mock.mint_authorization_code(
            sub="test-analyst",
            redirect_uri="http://x/callback",
            nonce="n",
            code_challenge=challenge,
        )
        oidc_mock.exchange_authorization_code(
            code=code, redirect_uri="http://x/callback", code_verifier=verifier
        )
        with self.assertRaises(oidc_mock.CodeAlreadyUsed):
            oidc_mock.exchange_authorization_code(
                code=code, redirect_uri="http://x/callback", code_verifier=verifier
            )


if __name__ == "__main__":
    unittest.main()
