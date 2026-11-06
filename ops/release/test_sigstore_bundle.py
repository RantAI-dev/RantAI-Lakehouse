#!/usr/bin/env python3
"""Tests for the Sigstore bundle that signs the release manifest.

Why this exists
---------------
REL-2 S2 D3a made the manifest's signature a Sigstore bundle:
`release.json.sigstore.json` replaces the detached `release.json.sig`, and
`lakehousectl` (REL-2 S3) verifies the manifest through it. The recipe
S3 will use is:

  - read `messageSignature.signature`: base64 DER, an ECDSA P-256
    signature over the SHA-256 of `release.json`;
  - check it against the public key `lakehousectl` holds;
  - ignore every transparency-log field the bundle may carry;
  - check that `messageSignature.messageDigest` equals the SHA-256 of the
    `release.json` beside it.

These tests run that recipe NOW, in Python, against a checked-in bundle
made with a throwaway key — so the recipe is proved before the Rust
that depends on it exists. A recipe that only the S3 slice can prove is
a recipe that breaks the first time it is used.

The fixture
-----------
`ops/release/fixtures/sigstore/` holds three files made together by
cosign v3 with a THROWAWAY key that exists nowhere else (it was deleted
when the fixture was written):

  - `release.json` — the manifest that was signed;
  - `release.json.sigstore.json` — the bundle cosign wrote for it, with
    `--use-signing-config=false --tlog-upload=false`, so it carries no
    transparency-log entry;
  - `throwaway.pub` — the public key of that key.

The key is a throwaway: it signs nothing but this fixture, and the
private half is NOT in the repository.

What these tests do NOT do
--------------------------
- They do not run cosign: the pipeline's own dry run does that, and a
  test that needed cosign on the PATH would test the runner, not the
  bundle.
- They do not check that the release key signs anything: that is the
  pipeline's job, and the pipeline verifies every signature it makes.
- They do not prove anything about a bundle with a transparency-log
  entry: the pipeline writes none (REL-2 S2 D3a), and S3's recipe is
  written to ignore such fields anyway.
"""

from __future__ import annotations

import base64
import hashlib
import json
import unittest
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec

FIXTURE_DIR = Path(__file__).resolve().parent / "fixtures" / "sigstore"
BUNDLE_PATH = FIXTURE_DIR / "release.json.sigstore.json"
MANIFEST_PATH = FIXTURE_DIR / "release.json"
PUBLIC_KEY_PATH = FIXTURE_DIR / "throwaway.pub"


def load_bundle() -> dict[str, object]:
    """The bundle, parsed."""
    return json.loads(BUNDLE_PATH.read_text(encoding="utf-8"))


def load_public_key() -> ec.EllipticCurvePublicKey:
    """The fixture's public key."""
    key = serialization.load_pem_public_key(PUBLIC_KEY_PATH.read_bytes())
    if not isinstance(key, ec.EllipticCurvePublicKey):
        raise AssertionError(f"the fixture's key is a {type(key).__name__}, not an EC key")
    return key


class SigstoreBundleTest(unittest.TestCase):
    """The bundle's shape, and the signature S3 will check."""

    def test_the_bundle_is_a_sigstore_bundle(self) -> None:
        bundle = load_bundle()
        self.assertEqual(
            bundle["mediaType"],
            "application/vnd.dev.sigstore.bundle.v0.3+json",
            "the bundle is not the media type cosign v3 writes",
        )

    def test_the_bundle_carries_no_transparency_log_entry(self) -> None:
        """The pipeline signs with no tlog, so the bundle carries none.

        This is the D3a decision: nothing is uploaded to the public
        Rekor log, so there is no log entry to verify and nothing to
        depend on. S3's recipe ignores such fields anyway; this test
        says the pipeline writes none.
        """
        bundle = load_bundle()
        material = bundle["verificationMaterial"]
        self.assertNotIn(
            "tlogEntries",
            material,
            "the bundle carries a transparency-log entry: the pipeline must sign with no tlog",
        )
        self.assertNotIn(
            "certificate",
            material,
            "the bundle carries a certificate: the pipeline signs with a key, not with Fulcio",
        )

    def test_the_digest_is_the_sha256_of_the_manifest_beside_it(self) -> None:
        """The check S3 makes: the bundle's digest names the manifest.

        Without it, a bundle could carry a valid signature over some
        OTHER file, and `lakehousectl` would install a manifest nobody
        signed.
        """
        bundle = load_bundle()
        digest = bundle["messageSignature"]["messageDigest"]
        self.assertEqual(digest["algorithm"], "SHA2_256")
        expected = base64.b64encode(
            hashlib.sha256(MANIFEST_PATH.read_bytes()).digest()
        ).decode("ascii")
        self.assertEqual(
            digest["digest"],
            expected,
            "the bundle's digest is not the SHA-256 of the manifest beside it",
        )

    def test_the_signature_verifies_against_the_public_key(self) -> None:
        """The check S3 makes: the signature is the key's, over the manifest."""
        bundle = load_bundle()
        signature = base64.b64decode(
            bundle["messageSignature"]["signature"].encode("ascii")
        )
        load_public_key().verify(
            signature,
            MANIFEST_PATH.read_bytes(),
            ec.ECDSA(hashes.SHA256()),
        )

    def test_a_manifest_that_was_not_signed_is_refused(self) -> None:
        """The negative case: a bundle does not sign a different file.

        This is what makes the digest check above worth doing — the
        signature is over THIS manifest, so a tampered manifest does not
        verify.
        """
        bundle = load_bundle()
        signature = base64.b64decode(
            bundle["messageSignature"]["signature"].encode("ascii")
        )
        tampered = MANIFEST_PATH.read_bytes() + b"\n"
        with self.assertRaises(InvalidSignature):
            load_public_key().verify(
                signature, tampered, ec.ECDSA(hashes.SHA256())
            )

    def test_the_public_key_hint_names_the_key_the_bundle_was_made_with(self) -> None:
        """The hint is the SHA-256 of the public key, DER-encoded.

        It is how the bundle says which key signed it, so a verifier
        that holds one key can tell whether the bundle is the one it
        expects.
        """
        bundle = load_bundle()
        der = load_public_key().public_bytes(
            serialization.Encoding.DER,
            serialization.PublicFormat.SubjectPublicKeyInfo,
        )
        expected = base64.b64encode(hashlib.sha256(der).digest()).decode("ascii")
        self.assertEqual(
            bundle["verificationMaterial"]["publicKey"]["hint"],
            expected,
        )


if __name__ == "__main__":
    unittest.main()
