//! Cross-language compatibility: a token signed by the real TypeScript
//! `signEmbed` must still verify against the Rust port. Existing signed
//! embed URLs (already handed out to users) must keep working across the
//! cutover.
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

use lakehouse_embed::verify_embed;

/// Output of the `bun -e '...'` command above, verbatim.
const TS_GENERATED_TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
eyJyZXNvdXJjZSI6eyJkYXNoYm9hcmQiOiJiX3Rlc3QifSwicGFyYW1zIjp7fX0.\
K0SIZjpuvIx_IFDOaiLy4KqFqHsss0g8yHUhq0fayb8";
const SECRET: &str = "test-secret";

#[test]
fn ts_generated_token_verifies_in_rust() {
    let claims =
        verify_embed(TS_GENERATED_TOKEN, SECRET).expect("TS-signed token must verify in Rust");
    assert_eq!(
        claims.resource.and_then(|r| r.dashboard).as_deref(),
        Some("b_test")
    );
    assert_eq!(claims.params, Some(std::collections::HashMap::new()));
    assert_eq!(claims.exp, None);
}

#[test]
fn ts_generated_token_rejects_wrong_secret() {
    assert!(verify_embed(TS_GENERATED_TOKEN, "not-the-secret").is_none());
}
