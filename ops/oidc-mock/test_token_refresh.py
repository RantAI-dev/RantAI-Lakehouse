"""Unit tests for oidc-mock's token refresh decision.

No network, per AGENTS.md's Python rules: these import `server.py` and call
`token_expiry`/`token_needs_refresh` directly, which is where the real
decision lives -- "is the token on the volume still usable, or must it be
replaced?".

Why this matters: the token volume is PERSISTENT. The tokens minted when
the volume was first created stayed on it across every container restart
and every deploy, so they expired on their own `exp` with nothing
replacing them -- and the one-shot init jobs that read `admin.jwt` off
that volume stopped being able to authenticate. A refresh decision that
never replaces anything is the bug, so the tests below pin the two
directions: a token with life left is kept, a token without is replaced.
"""

from __future__ import annotations

import base64
import json
import tempfile
import time
import unittest
from pathlib import Path

from cryptography.hazmat.primitives.asymmetric import rsa

import server as oidc_mock


def _token_with_exp(exp: int) -> str:
    """A JWT-shaped string whose payload carries `exp`."""
    payload = base64.urlsafe_b64encode(json.dumps({"exp": exp}).encode()).decode()
    return "header." + payload.rstrip("=") + ".signature"


class TokenExpiryTests(unittest.TestCase):
    def test_expiry_is_read_from_the_token_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "admin.jwt"
            path.write_text(_token_with_exp(int(time.time()) + 3600))
            self.assertEqual(
                oidc_mock.token_expiry(path), int(time.time()) + 3600
            )

    def test_a_token_that_cannot_be_read_has_no_expiry(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "admin.jwt"
            path.write_text("not-a-jwt")
            self.assertIsNone(oidc_mock.token_expiry(path))

    def test_a_missing_file_has_no_expiry(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(oidc_mock.token_expiry(Path(tmp) / "absent.jwt"))


class TokenNeedsRefreshTests(unittest.TestCase):
    def test_a_token_with_life_left_is_kept(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "admin.jwt"
            lifetime = oidc_mock.TOKEN_LIFETIME_SECONDS
            path.write_text(_token_with_exp(int(time.time()) + lifetime))
            self.assertFalse(oidc_mock.token_needs_refresh(path))

    def test_an_expired_token_is_replaced(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "admin.jwt"
            path.write_text(_token_with_exp(int(time.time()) - 1))
            self.assertTrue(oidc_mock.token_needs_refresh(path))

    def test_a_token_that_expires_within_the_margin_is_replaced(self):
        # The margin is what keeps the stack's tokens from expiring
        # between two refreshes: a token that is still valid but has less
        # than the margin left is replaced NOW, while it still works.
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "admin.jwt"
            path.write_text(
                _token_with_exp(
                    int(time.time()) + oidc_mock.TOKEN_REFRESH_MARGIN_SECONDS - 60
                )
            )
            self.assertTrue(oidc_mock.token_needs_refresh(path))

    def test_an_unreadable_token_is_replaced(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "admin.jwt"
            path.write_text("not-a-jwt")
            self.assertTrue(oidc_mock.token_needs_refresh(path))


class MintTokensTests(unittest.TestCase):
    """`mint_tokens` is what writes the token volume, so it is where the
    refresh decision becomes real: a token that is still good is left
    alone, an expired one is replaced."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self._original_dir = oidc_mock.TOKENS_DIR
        oidc_mock.TOKENS_DIR = Path(self._tmp.name)
        self.addCleanup(self._restore_tokens_dir)
        self.key = rsa.generate_private_key(public_exponent=65537, key_size=2048)

    def _restore_tokens_dir(self):
        oidc_mock.TOKENS_DIR = self._original_dir

    def test_every_principal_gets_a_token(self):
        oidc_mock.mint_tokens(self.key)
        for sub in oidc_mock.PRINCIPALS:
            self.assertTrue((Path(self._tmp.name) / f"{sub}.jwt").exists(), sub)

    def test_a_token_that_is_still_good_is_left_alone(self):
        oidc_mock.mint_tokens(self.key)
        path = Path(self._tmp.name) / "admin.jwt"
        before = path.read_text()
        oidc_mock.mint_tokens(self.key)
        self.assertEqual(path.read_text(), before)

    def test_an_expired_token_is_replaced(self):
        oidc_mock.mint_tokens(self.key)
        path = Path(self._tmp.name) / "admin.jwt"
        path.write_text(_token_with_exp(int(time.time()) - 10))
        oidc_mock.mint_tokens(self.key)
        self.assertGreater(oidc_mock.token_expiry(path), int(time.time()))

    def test_a_replaced_token_keeps_its_principal(self):
        # The `sub` claim is the identity Lakekeeper's grants were given
        # to, so replacing a token must never change who it is.
        oidc_mock.mint_tokens(self.key)
        path = Path(self._tmp.name) / "admin.jwt"
        path.write_text(_token_with_exp(int(time.time()) - 10))
        oidc_mock.mint_tokens(self.key)
        payload = path.read_text().split(".")[1]
        payload += "=" * (-len(payload) % 4)
        claims = json.loads(base64.urlsafe_b64decode(payload).decode("ascii"))
        self.assertEqual(claims["sub"], "admin")


if __name__ == "__main__":
    unittest.main()
