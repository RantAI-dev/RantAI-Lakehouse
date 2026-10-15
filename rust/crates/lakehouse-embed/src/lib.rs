//! Signed embedding (Metabase-style), porting
//! `src/services/clients/embed-jwt.ts`.
//!
//! The host encodes an HS256 JWT carrying a dashboard `resource` plus
//! locked filter `params`, signed with an embedding secret. Our server
//! verifies the signature and expiry, then renders the dashboard with the
//! locked filters (the viewer can't change them). No external JWT library
//! is used — matching the TypeScript, which hand-rolls the same
//! three-segment `header.payload.signature` format with `crypto`.
//!
//! # `SEC-12`: the secret is configuration, never generated
//!
//! This crate used to read or invent the signing secret and keep it as
//! plain text in `console.app_kv` when `EMBED_SECRET` was unset. That path
//! is gone: the secret comes from configuration only (`Config::embed_secret`)
//! and an unset secret means signed embedding is unavailable. An existing
//! `embed_secret` row is no longer read; the operator removes it.
//!
//! # `SEC-12`: a token always expires
//!
//! Before `SEC-12` a token without `exp` was valid forever, so one copied
//! from a page source worked for good. [`verify_embed`] now refuses a token
//! unless it carries both `exp` and `iat` (Unix seconds), its lifetime
//! (`exp - iat`) is at most the configured maximum, `iat` is not in the
//! future and `exp` is not in the past, each with
//! [`CLOCK_TOLERANCE_SECONDS`] of tolerance for a clock difference between
//! the customer's server and ours. There is no grace setting: an old token
//! is refused at once.
//!
//! The signature is checked first, before any claim is read, so an
//! unsigned or forged payload never reaches the claim checks. Callers map
//! every [`TokenError`] to one fixed message so a caller cannot tell which
//! check failed; the variant exists for tests and the log.

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The dashboard resource an embed token grants access to, mirroring the
/// TypeScript's `{ dashboard?: string }`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedResource {
    /// The dashboard slug/id being embedded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<String>,
}

/// Claims carried by a signed embed token: the TypeScript's
/// `{ resource?, params?, exp? }` plus `iat` and `jti` (`SEC-12`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EmbedClaims {
    /// The dashboard resource this token grants access to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<EmbedResource>,
    /// Locked filter parameters the viewer cannot change. Each value is
    /// either a single string or an array of strings, matching
    /// `Record<string, string | string[]>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<HashMap<String, Value>>,
    /// Unix-seconds expiry. Required by [`verify_embed`] (`SEC-12`); an
    /// `Option` only so a token can be built without one in a test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<f64>,
    /// Unix-seconds issue time. Required by [`verify_embed`] (`SEC-12`):
    /// "withdraw all tokens" compares it with the instant of the
    /// withdrawal, and the lifetime limit is `exp - iat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iat: Option<f64>,
    /// Optional token id (1 to 128 characters of `[A-Za-z0-9._-]`). A token
    /// that carries one can be withdrawn on its own; one without cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
}

/// The longest lifetime (`exp - iat`) accepted when no setting overrides
/// it: 24 hours (`SEC-12`, signed by the product owner).
pub const DEFAULT_MAX_LIFETIME_SECONDS: u64 = 86_400;

/// How far a customer's clock may differ from ours, in seconds, before a
/// token's `iat` counts as "in the future" or its `exp` as "in the past".
pub const CLOCK_TOLERANCE_SECONDS: f64 = 60.0;

/// The longest `jti` accepted, in characters.
pub const MAX_JTI_CHARS: usize = 128;

/// Why [`verify_embed`] refused a token. For tests and the log only: a
/// response never says which check failed (`SEC-12`, one fixed message).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// Not three `.`-separated segments, or a segment that does not decode.
    #[error("token is malformed")]
    Malformed,
    /// The signature does not match. Reported before any claim is read.
    #[error("signature does not match")]
    BadSignature,
    /// `exp` is missing or not a number.
    #[error("exp is missing or not a number")]
    MissingExp,
    /// `iat` is missing or not a number.
    #[error("iat is missing or not a number")]
    MissingIat,
    /// `exp` is in the past beyond the clock tolerance.
    #[error("token has expired")]
    Expired,
    /// `iat` is in the future beyond the clock tolerance.
    #[error("token was issued in the future")]
    IssuedInFuture,
    /// `exp - iat` is negative or longer than the maximum lifetime.
    #[error("token lifetime is not within the allowed maximum")]
    Lifetime,
    /// `jti` is present but is not 1 to 128 characters of `[A-Za-z0-9._-]`.
    #[error("jti is malformed")]
    BadJti,
}

/// Base64url-encode (no padding), matching the TypeScript's hand-rolled
/// `b64url` helper (`base64` with `+`/`/`/`=` translated to
/// URL-safe/no-pad).
fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Base64url-decode (no padding). Returns `Err` for malformed input,
/// matching the TypeScript's `fromB64url`, which can throw inside
/// `Buffer.from(..., "base64")` for sufficiently malformed strings (caught
/// by `verifyEmbed`'s `try { ... } catch { return null; }`).
fn from_b64url(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    URL_SAFE_NO_PAD.decode(s)
}

/// JSON-serialize `value` then base64url-encode it, matching
/// `b64urlJson`.
fn b64url_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    Ok(b64url(serde_json::to_vec(value)?.as_slice()))
}

/// Sign `claims` as an HS256 JWT, matching `signEmbed`. Used by the host
/// (and for preview/example purposes in the UI).
///
/// Returns an empty string in the (unreachable in practice) case that
/// HMAC key setup fails — `HmacSha256::new_from_slice` only errors for a
/// key length HMAC-SHA256 rejects, which does not exist (RFC 2104 hashes
/// keys of any length internally), so this never actually happens with a
/// real `secret`.
#[must_use]
pub fn sign_embed(claims: &EmbedClaims, secret: &str) -> String {
    let header = b64url_json(&serde_json::json!({ "alg": "HS256", "typ": "JWT" }))
        .unwrap_or_else(|_| "e30".to_owned());
    let payload = b64url_json(claims).unwrap_or_else(|_| "e30".to_owned());
    let data = format!("{header}.{payload}");
    let Ok(mut mac) = <HmacSha256 as KeyInit>::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(data.as_bytes());
    let sig = b64url(&mac.finalize().into_bytes());
    format!("{data}.{sig}")
}

/// Verify a token's signature and claims (`SEC-12`). Returns the claims when
/// the token is acceptable at `now_unix_seconds`.
///
/// The checks run in this order and stop at the first failure:
/// 1. three segments and a decodable signature ([`TokenError::Malformed`]);
/// 2. the signature, compared in constant time ([`Mac::verify_slice`]),
///    before the payload is looked at ([`TokenError::BadSignature`]);
/// 3. the payload parses ([`TokenError::Malformed`]);
/// 4. `exp` and `iat` are numbers ([`TokenError::MissingExp`],
///    [`TokenError::MissingIat`]);
/// 5. `0 <= exp - iat <= max_lifetime_seconds` ([`TokenError::Lifetime`]);
///    a lifetime exactly at the maximum is accepted;
/// 6. `iat <= now + 60` ([`TokenError::IssuedInFuture`]) and
///    `exp >= now - 60` ([`TokenError::Expired`]);
/// 7. `jti`, when present, is well formed ([`TokenError::BadJti`]).
///
/// `now_unix_seconds` is a parameter, not read here, so tests need no
/// sleeping; callers pass [`unix_now`].
///
/// # Errors
///
/// A [`TokenError`] naming the first check that failed.
pub fn verify_embed(
    token: &str,
    secret: &str,
    max_lifetime_seconds: u64,
    now_unix_seconds: f64,
) -> Result<EmbedClaims, TokenError> {
    let parts: Vec<&str> = token.split('.').collect();
    let [header, payload, sig] = parts.as_slice() else {
        return Err(TokenError::Malformed);
    };
    let data = format!("{header}.{payload}");
    let given = from_b64url(sig).map_err(|_| TokenError::Malformed)?;
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(secret.as_bytes())
        .map_err(|_| TokenError::BadSignature)?;
    mac.update(data.as_bytes());
    // Constant-time comparison, matching `crypto.timingSafeEqual`.
    // `verify_slice` folds the length check into the same call.
    mac.verify_slice(&given)
        .map_err(|_| TokenError::BadSignature)?;

    let payload_bytes = from_b64url(payload).map_err(|_| TokenError::Malformed)?;
    let raw: Value = serde_json::from_slice(&payload_bytes).map_err(|_| TokenError::Malformed)?;
    // `exp` and `iat` are read from the raw value so a string or null is
    // reported as missing, not as a payload that failed to parse.
    let exp = finite_number(&raw, "exp").ok_or(TokenError::MissingExp)?;
    let iat = finite_number(&raw, "iat").ok_or(TokenError::MissingIat)?;
    let claims: EmbedClaims = serde_json::from_value(raw).map_err(|_| TokenError::Malformed)?;

    let lifetime = exp - iat;
    if lifetime < 0.0 || lifetime > seconds_as_f64(max_lifetime_seconds) {
        return Err(TokenError::Lifetime);
    }
    if iat > now_unix_seconds + CLOCK_TOLERANCE_SECONDS {
        return Err(TokenError::IssuedInFuture);
    }
    if exp < now_unix_seconds - CLOCK_TOLERANCE_SECONDS {
        return Err(TokenError::Expired);
    }
    if let Some(jti) = &claims.jti
        && !is_valid_jti(jti)
    {
        return Err(TokenError::BadJti);
    }
    Ok(claims)
}

/// Whether `jti` is 1 to [`MAX_JTI_CHARS`] characters of `[A-Za-z0-9._-]`.
#[must_use]
pub fn is_valid_jti(jti: &str) -> bool {
    !jti.is_empty()
        && jti.chars().count() <= MAX_JTI_CHARS
        && jti
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn finite_number(raw: &Value, key: &str) -> Option<f64> {
    raw.get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}

#[allow(
    clippy::cast_precision_loss,
    reason = "a lifetime limit in seconds; a u64 above 2^53 seconds is far beyond any real limit"
)]
fn seconds_as_f64(seconds: u64) -> f64 {
    seconds as f64
}

/// The current time in Unix seconds, for passing to [`verify_embed`].
#[must_use]
pub fn unix_now() -> f64 {
    now_unix_millis() / 1000.0
}

/// `Date.now()` — current Unix time in milliseconds.
#[allow(
    clippy::cast_precision_loss,
    reason = "millisecond-precision expiry comparison; precision loss at \
              this magnitude is inconsequential"
)]
fn now_unix_millis() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;

    const SECRET: &str = "s3cret";
    /// A fixed "now" so no test reads the clock or sleeps.
    const NOW: f64 = 1_800_000_000.0;
    const MAX: u64 = 86_400;

    fn claims(iat: f64, exp: f64) -> EmbedClaims {
        EmbedClaims {
            resource: Some(EmbedResource {
                dashboard: Some("b_test".to_owned()),
            }),
            params: Some(HashMap::from([("tenant".to_owned(), json!("dispar-dki"))])),
            exp: Some(exp),
            iat: Some(iat),
            jti: None,
        }
    }

    /// Signs an arbitrary payload, so a test can build a token that
    /// `sign_embed` would never produce (a string `exp`, no `iat`).
    fn sign_payload(payload: &Value) -> String {
        let header = b64url_json(&json!({ "alg": "HS256", "typ": "JWT" })).unwrap();
        let data = format!("{header}.{}", b64url_json(payload).unwrap());
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(data.as_bytes());
        format!("{data}.{}", b64url(&mac.finalize().into_bytes()))
    }

    fn verify(token: &str) -> Result<EmbedClaims, TokenError> {
        verify_embed(token, SECRET, MAX, NOW)
    }

    #[test]
    fn round_trips_claims() {
        let claims = claims(NOW, NOW + 3600.0);
        let token = sign_embed(&claims, SECRET);
        assert_eq!(verify(&token), Ok(claims));
    }

    #[test]
    fn rejects_wrong_secret() {
        let token = sign_embed(&claims(NOW, NOW + 3600.0), SECRET);
        assert_eq!(
            verify_embed(&token, "wrong-secret", MAX, NOW),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn rejects_wrong_segment_count() {
        assert_eq!(verify("only.two"), Err(TokenError::Malformed));
        assert_eq!(verify("a.b.c.d"), Err(TokenError::Malformed));
        assert_eq!(verify("nodots"), Err(TokenError::Malformed));
    }

    #[test]
    fn a_wrong_signature_is_refused_before_any_claim_is_read() {
        // The payload has no `exp` and no `iat`, which would be refused on
        // its own, but the signature is wrong: that is what must be
        // reported, proving the claims were never looked at.
        let token = sign_payload(&json!({ "resource": { "dashboard": "b_test" } }));
        let parts: Vec<&str> = token.split('.').collect();
        let forged = format!("{}.{}.{}", parts[0], parts[1], b64url(b"not-the-signature"));
        assert_eq!(verify(&forged), Err(TokenError::BadSignature));
    }

    #[test]
    fn rejects_a_token_with_no_exp() {
        // `SEC-12`: this used to be accepted ("a token without exp never
        // expires"); that was the bug.
        let token = sign_payload(&json!({ "iat": NOW, "resource": { "dashboard": "b" } }));
        assert_eq!(verify(&token), Err(TokenError::MissingExp));
    }

    #[test]
    fn rejects_a_token_with_no_iat() {
        let token = sign_payload(&json!({ "exp": NOW + 60.0 }));
        assert_eq!(verify(&token), Err(TokenError::MissingIat));
    }

    #[test]
    fn rejects_a_non_numeric_exp_or_iat() {
        let exp = sign_payload(&json!({ "iat": NOW, "exp": "soon" }));
        assert_eq!(verify(&exp), Err(TokenError::MissingExp));
        let iat = sign_payload(&json!({ "iat": null, "exp": NOW + 60.0 }));
        assert_eq!(verify(&iat), Err(TokenError::MissingIat));
        let boolean = sign_payload(&json!({ "iat": true, "exp": NOW + 60.0 }));
        assert_eq!(verify(&boolean), Err(TokenError::MissingIat));
    }

    #[test]
    fn rejects_an_expired_token_but_allows_sixty_seconds_of_clock_difference() {
        let at = |exp: f64| sign_embed(&claims(exp - 600.0, exp), SECRET);
        // Expired 61 s ago: refused. Expired exactly 60 s ago: still accepted.
        assert_eq!(verify(&at(NOW - 61.0)), Err(TokenError::Expired));
        assert!(verify(&at(NOW - 60.0)).is_ok());
        assert!(verify(&at(NOW + 1.0)).is_ok());
    }

    #[test]
    fn rejects_a_token_issued_in_the_future_but_allows_sixty_seconds_of_clock_difference() {
        let at = |iat: f64| sign_embed(&claims(iat, iat + 600.0), SECRET);
        assert_eq!(verify(&at(NOW + 61.0)), Err(TokenError::IssuedInFuture));
        assert!(verify(&at(NOW + 60.0)).is_ok());
        assert!(verify(&at(NOW - 5.0)).is_ok());
    }

    #[test]
    fn a_lifetime_exactly_at_the_maximum_is_accepted_and_one_second_more_is_not() {
        let at = |lifetime: f64| sign_embed(&claims(NOW, NOW + lifetime), SECRET);
        assert!(verify(&at(86_400.0)).is_ok());
        assert_eq!(verify(&at(86_401.0)), Err(TokenError::Lifetime));
        // The limit is a parameter, not a constant.
        assert_eq!(
            verify_embed(&at(3601.0), SECRET, 3600, NOW),
            Err(TokenError::Lifetime)
        );
        assert!(verify_embed(&at(3600.0), SECRET, 3600, NOW).is_ok());
    }

    #[test]
    fn a_token_that_expires_before_it_was_issued_is_refused() {
        let token = sign_embed(&claims(NOW, NOW - 10.0), SECRET);
        assert_eq!(verify(&token), Err(TokenError::Lifetime));
    }

    #[test]
    fn a_ten_year_token_is_refused() {
        // Replaces `accepts_far_future_expiry`, which pinned the bug.
        let token = sign_embed(&claims(NOW, NOW + 3600.0 * 24.0 * 365.0 * 10.0), SECRET);
        assert_eq!(verify(&token), Err(TokenError::Lifetime));
    }

    #[test]
    fn jti_is_optional_but_must_be_well_formed() {
        let with = |jti: &str| {
            let mut c = claims(NOW, NOW + 600.0);
            c.jti = Some(jti.to_owned());
            sign_embed(&c, SECRET)
        };
        assert_eq!(
            verify(&with("a.B_c-1")).map(|c| c.jti),
            Ok(Some("a.B_c-1".to_owned()))
        );
        assert!(verify(&with(&"x".repeat(128))).is_ok());
        for bad in [
            "",
            "has space",
            "slash/inside",
            "uni\u{e9}",
            &"x".repeat(129),
        ] {
            assert_eq!(verify(&with(bad)), Err(TokenError::BadJti), "{bad:?}");
        }
        let no_jti = sign_embed(&claims(NOW, NOW + 600.0), SECRET);
        assert_eq!(verify(&no_jti).map(|c| c.jti), Ok(None));
        // A jti that is not a string does not parse as a token at all.
        let numeric = sign_payload(&json!({ "iat": NOW, "exp": NOW + 60.0, "jti": 7 }));
        assert_eq!(verify(&numeric), Err(TokenError::Malformed));
    }

    #[test]
    fn rejects_tampered_payload() {
        let claims = claims(NOW, NOW + 3600.0);
        let token = sign_embed(&claims, SECRET);
        let parts: Vec<&str> = token.split('.').collect();
        let tampered_claims = EmbedClaims {
            resource: Some(EmbedResource {
                dashboard: Some("b_evil".to_owned()),
            }),
            ..claims
        };
        let tampered_payload = b64url_json(&tampered_claims).unwrap();
        let tampered = format!("{}.{}.{}", parts[0], tampered_payload, parts[2]);
        assert_eq!(verify(&tampered), Err(TokenError::BadSignature));
    }

    #[test]
    fn rejects_tampered_signature() {
        let token = sign_embed(&claims(NOW, NOW + 3600.0), SECRET);
        let parts: Vec<&str> = token.split('.').collect();
        // Flip the signature to something else decodable but wrong.
        let bogus_sig = b64url(b"not-the-real-signature-bytes!!!!");
        let tampered = format!("{}.{}.{}", parts[0], parts[1], bogus_sig);
        assert_eq!(verify(&tampered), Err(TokenError::BadSignature));
    }
}
