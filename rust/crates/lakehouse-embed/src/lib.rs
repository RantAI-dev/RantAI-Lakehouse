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
use std::fmt::Write;

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

/// The payload of `token` once its signature checks out under `key`, before
/// any claim is read (`SEC-12`): three segments, a decodable signature, a
/// constant-time comparison, then the payload parsed. Shared by
/// [`verify_embed`] and [`verify_render`], so the order of those checks has
/// one copy.
fn open_signed(token: &str, key: &str) -> Result<Value, TokenError> {
    let parts: Vec<&str> = token.split('.').collect();
    let [header, payload, sig] = parts.as_slice() else {
        return Err(TokenError::Malformed);
    };
    let data = format!("{header}.{payload}");
    let given = from_b64url(sig).map_err(|_| TokenError::Malformed)?;
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key.as_bytes())
        .map_err(|_| TokenError::BadSignature)?;
    mac.update(data.as_bytes());
    // Constant-time comparison, matching `crypto.timingSafeEqual`.
    // `verify_slice` folds the length check into the same call.
    mac.verify_slice(&given)
        .map_err(|_| TokenError::BadSignature)?;
    let payload_bytes = from_b64url(payload).map_err(|_| TokenError::Malformed)?;
    serde_json::from_slice(&payload_bytes).map_err(|_| TokenError::Malformed)
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
    let raw = open_signed(token, secret)?;
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

// ── render tokens (BI-1, plan decision D2) ────────────────────────────────

/// The audience of a render token.
pub const RENDER_AUDIENCE: &str = "render";

/// How long a render token lives: 2 minutes (feature decision 2). Not
/// configurable: the token is minted and spent inside one PDF request, and a
/// longer life would only be more time to misuse a copy.
pub const RENDER_LIFETIME_SECONDS: u64 = 120;

/// What the PDF renderer is allowed to read, and as whom (`BI-1`, D2).
///
/// The renderer is a headless browser that opens the console's print view with
/// this token in the address. It names one dashboard, one person and the
/// filter and parameter values the person had on screen, so the token is good
/// for that and nothing else, for two minutes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RenderClaims {
    /// Always [`RENDER_AUDIENCE`]; a token with another audience is refused.
    #[serde(default)]
    pub aud: String,
    /// The dashboard to print.
    pub board: String,
    /// The person who asked (`app_user.id`): the data is read as them.
    pub principal: String,
    /// The dashboard's filters, as the JSON list `GET /api/dashboard` takes in
    /// `filters` (one encoding, the page's own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<String>,
    /// The parameter values, as the JSON object `GET /api/dashboard` takes in `params`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    /// The grain switch, as `GET /api/dashboard` takes it in `grain`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<String>,
    /// `a4` or `letter`.
    pub paper: String,
    /// `portrait` or `landscape`.
    pub orientation: String,
    /// Unix-seconds expiry.
    #[serde(default)]
    pub exp: f64,
    /// Unix-seconds issue time.
    #[serde(default)]
    pub iat: f64,
    /// A unique id for the log.
    pub jti: String,
}

/// When a render token issued at `iat` expires.
#[must_use]
pub fn render_expiry(iat: f64) -> f64 {
    iat + seconds_as_f64(RENDER_LIFETIME_SECONDS)
}

/// The label the render key is derived under; changing it retires every token.
const RENDER_KEY_LABEL: &str = "lakehouse/render-token/v1";

/// The key render tokens are signed with: `HMAC-SHA256(EMBED_SECRET, label)`
/// in hex. A render token therefore never verifies as an embed token, whose
/// key is the secret itself, and an embed token never verifies as a render
/// token (tested both ways).
fn render_key(secret: &str) -> String {
    let Ok(mut mac) = <HmacSha256 as KeyInit>::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(RENDER_KEY_LABEL.as_bytes());
    let mut hex = String::new();
    for byte in mac.finalize().into_bytes() {
        // Writing to a `String` cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Sign `claims` as a render token under the key derived from `secret`.
/// Returns an empty string only if the HMAC key cannot be set up, which no
/// key length causes (see [`sign_embed`]).
#[must_use]
pub fn sign_render(claims: &RenderClaims, secret: &str) -> String {
    let header = b64url_json(&serde_json::json!({ "alg": "HS256", "typ": "JWT" }))
        .unwrap_or_else(|_| "e30".to_owned());
    let payload = b64url_json(claims).unwrap_or_else(|_| "e30".to_owned());
    let data = format!("{header}.{payload}");
    let Ok(mut mac) = <HmacSha256 as KeyInit>::new_from_slice(render_key(secret).as_bytes()) else {
        return String::new();
    };
    mac.update(data.as_bytes());
    format!("{data}.{}", b64url(&mac.finalize().into_bytes()))
}

/// Verify a render token at `now_unix_seconds`: the signature first, then the
/// audience, the lifetime (at most [`RENDER_LIFETIME_SECONDS`]), the clock
/// window (as [`verify_embed`]) and the `jti`.
///
/// # Errors
///
/// A [`TokenError`] naming the first check that failed; a caller answers with
/// one fixed message.
pub fn verify_render(
    token: &str,
    secret: &str,
    now_unix_seconds: f64,
) -> Result<RenderClaims, TokenError> {
    let raw = open_signed(token, &render_key(secret))?;
    let exp = finite_number(&raw, "exp").ok_or(TokenError::MissingExp)?;
    let iat = finite_number(&raw, "iat").ok_or(TokenError::MissingIat)?;
    let claims: RenderClaims = serde_json::from_value(raw).map_err(|_| TokenError::Malformed)?;
    if claims.aud != RENDER_AUDIENCE {
        return Err(TokenError::Malformed);
    }
    let lifetime = exp - iat;
    if lifetime < 0.0 || lifetime > seconds_as_f64(RENDER_LIFETIME_SECONDS) {
        return Err(TokenError::Lifetime);
    }
    if iat > now_unix_seconds + CLOCK_TOLERANCE_SECONDS {
        return Err(TokenError::IssuedInFuture);
    }
    if exp < now_unix_seconds - CLOCK_TOLERANCE_SECONDS {
        return Err(TokenError::Expired);
    }
    if !is_valid_jti(&claims.jti) {
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

    fn render_claims(iat: f64, exp: f64) -> RenderClaims {
        RenderClaims {
            aud: RENDER_AUDIENCE.to_owned(),
            board: "b_test".to_owned(),
            principal: "7d1e1c52-2f6b-4f3a-9c58-0c5d1d3f9a11".to_owned(),
            filters: Some(r#"[{"column":"region","values":["north"]}]"#.to_owned()),
            params: Some(r#"{"p":1}"#.to_owned()),
            grain: None,
            paper: "a4".to_owned(),
            orientation: "landscape".to_owned(),
            exp,
            iat,
            jti: "r-1".to_owned(),
        }
    }

    #[test]
    fn a_render_token_round_trips_and_lives_two_minutes() {
        let claims = render_claims(NOW, NOW + 120.0);
        let token = sign_render(&claims, SECRET);
        assert_eq!(verify_render(&token, SECRET, NOW), Ok(claims));
        let long = sign_render(&render_claims(NOW, NOW + 121.0), SECRET);
        assert_eq!(verify_render(&long, SECRET, NOW), Err(TokenError::Lifetime));
    }

    #[test]
    fn an_embed_token_is_not_a_render_token_and_the_reverse() {
        let embed = sign_embed(&claims(NOW, NOW + 60.0), SECRET);
        assert_eq!(
            verify_render(&embed, SECRET, NOW),
            Err(TokenError::BadSignature)
        );
        let render = sign_render(&render_claims(NOW, NOW + 60.0), SECRET);
        assert_eq!(verify(&render), Err(TokenError::BadSignature));
        // Even a render payload signed with the embed secret itself is not a render token.
        let forged = sign_payload(&serde_json::to_value(render_claims(NOW, NOW + 60.0)).unwrap());
        assert_eq!(
            verify_render(&forged, SECRET, NOW),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn an_expired_or_future_or_wrong_audience_render_token_is_refused() {
        let at = |iat: f64, exp: f64| sign_render(&render_claims(iat, exp), SECRET);
        assert_eq!(
            verify_render(&at(NOW - 400.0, NOW - 280.0), SECRET, NOW),
            Err(TokenError::Expired)
        );
        assert_eq!(
            verify_render(&at(NOW + 400.0, NOW + 500.0), SECRET, NOW),
            Err(TokenError::IssuedInFuture)
        );
        let mut other = render_claims(NOW, NOW + 60.0);
        other.aud = "embed".to_owned();
        assert_eq!(
            verify_render(&sign_render(&other, SECRET), SECRET, NOW),
            Err(TokenError::Malformed)
        );
        let mut bad_jti = render_claims(NOW, NOW + 60.0);
        bad_jti.jti = "has space".to_owned();
        assert_eq!(
            verify_render(&sign_render(&bad_jti, SECRET), SECRET, NOW),
            Err(TokenError::BadJti)
        );
        assert_eq!(
            verify_render(
                &sign_render(&render_claims(NOW, NOW + 60.0), "other"),
                SECRET,
                NOW
            ),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn a_tampered_render_payload_is_refused() {
        let token = sign_render(&render_claims(NOW, NOW + 60.0), SECRET);
        let parts: Vec<&str> = token.split('.').collect();
        let mut evil = render_claims(NOW, NOW + 60.0);
        evil.board = "b_evil".to_owned();
        let tampered = format!("{}.{}.{}", parts[0], b64url_json(&evil).unwrap(), parts[2]);
        assert_eq!(
            verify_render(&tampered, SECRET, NOW),
            Err(TokenError::BadSignature)
        );
    }
}
