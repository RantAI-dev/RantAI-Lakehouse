//! Cross-language compatibility: a token signed by the real TypeScript
//! `signEmbed` still has a signature the Rust port verifies. Since `SEC-12`
//! its claims (no `exp`, no `iat`) are refused, by design.
//!
//! Approach chosen: a FIXTURE token, generated once from the TypeScript
//! and committed here verbatim, rather than shelling out to `bun` from the
//! test itself. Invoking `bun` at test time would make this test flaky in
//! any environment without a working `bun`/Node toolchain and TS
//! dependencies installed (this crate's CI job has neither), so a fixture
//! is the more robust choice while still proving real interop — the token
//! below is unedited output from the real `signEmbed`.
//!
//! Regenerated with (from the repo root, `RantAI-Lakehouse/`):
//! ```sh
//! bun -e 'import {signEmbed} from "./src/services/clients/embed-jwt.ts"; \
//!   console.log(signEmbed({resource:{dashboard:"b_test"},params:{}}, "test-secret"))'
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_embed::{TokenError, verify_embed};

/// Output of the `bun -e '...'` command above, verbatim.
const TS_GENERATED_TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
eyJyZXNvdXJjZSI6eyJkYXNoYm9hcmQiOiJiX3Rlc3QifSwicGFyYW1zIjp7fX0.\
K0SIZjpuvIx_IFDOaiLy4KqFqHsss0g8yHUhq0fayb8";
const SECRET: &str = "test-secret";

const NOW: f64 = 1_800_000_000.0;

/// `SEC-12`: the fixture carries no `exp` and no `iat`, so it is refused
/// now. The refusal is `MissingExp`, not `BadSignature`: the signature of a
/// token signed by the TypeScript still verifies here, and only the claims
/// are refused. (This test used to assert the token was accepted with
/// `exp == None`, which pinned the "never expires" bug.)
#[test]
fn ts_generated_token_has_a_valid_signature_but_is_refused_for_its_claims() {
    assert_eq!(
        verify_embed(TS_GENERATED_TOKEN, SECRET, 86_400, NOW),
        Err(TokenError::MissingExp)
    );
}

#[test]
fn ts_generated_token_rejects_wrong_secret() {
    assert_eq!(
        verify_embed(TS_GENERATED_TOKEN, "not-the-secret", 86_400, NOW),
        Err(TokenError::BadSignature)
    );
}
