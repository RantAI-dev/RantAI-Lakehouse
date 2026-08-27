//! Signed embedding (Metabase-style), porting
//! `src/services/clients/embed-jwt.ts`.
//!
//! The host encodes an HS256 JWT carrying a dashboard `resource` plus
//! locked filter `params`, signed with an embedding secret. Our server
//! verifies the signature and expiry, then renders the dashboard with the
//! locked filters (the viewer can't change them). No external JWT library
//! is used — matching the TypeScript, which hand-rolls the same
//! three-segment `header.payload.signature` format with `crypto`.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use lakehouse_clickhouse::{ChClient, ChError};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use tokio::sync::Mutex;

type HmacSha256 = Hmac<Sha256>;

/// The dashboard resource an embed token grants access to, mirroring the
/// TypeScript's `{ dashboard?: string }`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedResource {
    /// The dashboard slug/id being embedded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<String>,
}

/// Claims carried by a signed embed token, mirroring the TypeScript's
/// `EmbedClaims` type exactly: `{ resource?, params?, exp? }`.
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
    /// Unix-seconds expiry. `None` means the token never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<f64>,
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
    let Ok(mut mac) = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(data.as_bytes());
    let sig = b64url(&mac.finalize().into_bytes());
    format!("{data}.{sig}")
}

/// Verify a token's signature and expiry, matching `verifyEmbed`. Returns
/// the claims when valid, `None` otherwise.
///
/// Every rejection path in the TypeScript is reproduced:
/// - wrong number of `.`-separated segments (not exactly 3),
/// - an undecodable signature segment,
/// - a signature that doesn't match (checked in constant time via
///   [`Mac::verify_slice`], matching `crypto.timingSafeEqual`),
/// - an undecodable/unparsable payload segment,
/// - a numeric `exp` that is in the past.
#[must_use]
pub fn verify_embed(token: &str, secret: &str) -> Option<EmbedClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    let [header, payload, sig] = parts.as_slice() else {
        return None;
    };
    let data = format!("{header}.{payload}");
    let given = from_b64url(sig).ok()?;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(data.as_bytes());
    // Constant-time comparison, matching `crypto.timingSafeEqual`. A
    // length mismatch is also rejected by `verify_slice` (constant-time
    // itself doesn't require equal lengths to be checked separately here,
    // unlike the TS which checks `given.length !== expected.length`
    // first — `verify_slice` folds both checks into one call).
    mac.verify_slice(&given).ok()?;

    let payload_bytes = from_b64url(payload).ok()?;
    let claims: EmbedClaims = serde_json::from_slice(&payload_bytes).ok()?;

    if let Some(exp) = claims.exp {
        let now_ms = now_unix_millis();
        if exp * 1000.0 < now_ms {
            return None;
        }
    }
    Some(claims)
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

/// 32 random bytes, lower-hex encoded, matching
/// `crypto.randomBytes(32).toString("hex")`.
fn generate_hex_secret() -> String {
    use std::fmt::Write as _;

    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().fold(String::with_capacity(64), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

/// SQL that creates the `console.app_kv` table if it doesn't already
/// exist, matching `ensureKv` in the TypeScript.
const CREATE_APP_KV_TABLE: &str = "CREATE TABLE IF NOT EXISTS console.app_kv (\
       k String, v String, updated_at DateTime DEFAULT now()\
     ) ENGINE = ReplacingMergeTree(updated_at) ORDER BY k";

/// Resolves and caches the embedding secret, matching `getEmbedSecret`.
///
/// Preference order: an explicit `EMBED_SECRET` (config); otherwise
/// read/generate a secret persisted in `console.app_kv` (a
/// `ReplacingMergeTree` keyed on `k`), cached in-process afterward so
/// repeated calls don't re-hit `ClickHouse`.
pub struct EmbedSecretResolver {
    /// `EMBED_SECRET` from config, when set — always preferred, and never
    /// touches `ClickHouse` when present.
    env_secret: Option<String>,
    ch: Arc<ChClient>,
    /// In-process cache of a `ClickHouse`-backed secret, matching the
    /// TypeScript module-level `let secretCache: string | null = null`.
    cache: Mutex<Option<String>>,
}

impl EmbedSecretResolver {
    /// Build a resolver. `env_secret` should be `Config::embed_secret`;
    /// `ch` is used only when `env_secret` is `None`.
    #[must_use]
    pub fn new(env_secret: Option<String>, ch: Arc<ChClient>) -> Self {
        Self {
            env_secret,
            ch,
            cache: Mutex::new(None),
        }
    }

    /// Resolve the embedding secret.
    ///
    /// # Errors
    ///
    /// Returns [`ChError`] if `ClickHouse` is unreachable or rejects the
    /// `CREATE`/`SELECT`/`INSERT` statements used to read or persist a
    /// generated secret. Never fails when `env_secret` is set.
    pub async fn get_embed_secret(&self) -> Result<String, ChError> {
        if let Some(secret) = &self.env_secret {
            return Ok(secret.clone());
        }
        {
            let cached = self.cache.lock().await;
            if let Some(secret) = cached.as_ref() {
                return Ok(secret.clone());
            }
        }

        self.ch
            .exec("CREATE DATABASE IF NOT EXISTS console", None)
            .await?;
        self.ch.exec(CREATE_APP_KV_TABLE, None).await?;

        let rows = self
            .ch
            .rows(
                "SELECT v FROM console.app_kv FINAL WHERE k='embed_secret' LIMIT 1",
                None,
            )
            .await?;
        if let Some(existing) = rows
            .first()
            .and_then(|r| r.get("v"))
            .and_then(Value::as_str)
        {
            let secret = existing.to_owned();
            *self.cache.lock().await = Some(secret.clone());
            return Ok(secret);
        }

        let generated = generate_hex_secret();
        self.ch
            .exec(
                &format!(
                    "INSERT INTO console.app_kv (k, v) VALUES ('embed_secret', '{generated}')"
                ),
                None,
            )
            .await?;
        *self.cache.lock().await = Some(generated.clone());
        Ok(generated)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn claims_with_exp(exp_offset_secs: f64) -> EmbedClaims {
        EmbedClaims {
            resource: Some(EmbedResource {
                dashboard: Some("b_test".to_owned()),
            }),
            params: Some(HashMap::from([("tenant".to_owned(), json!("dispar-dki"))])),
            exp: Some(now_unix_millis() / 1000.0 + exp_offset_secs),
        }
    }

    #[test]
    fn round_trips_claims() {
        let claims = claims_with_exp(3600.0);
        let token = sign_embed(&claims, "s3cret");
        let verified = verify_embed(&token, "s3cret").expect("valid token");
        assert_eq!(verified, claims);
    }

    #[test]
    fn rejects_wrong_secret() {
        let claims = claims_with_exp(3600.0);
        let token = sign_embed(&claims, "s3cret");
        assert!(verify_embed(&token, "wrong-secret").is_none());
    }

    #[test]
    fn rejects_wrong_segment_count() {
        assert!(verify_embed("only.two", "s3cret").is_none());
        assert!(verify_embed("a.b.c.d", "s3cret").is_none());
        assert!(verify_embed("nodots", "s3cret").is_none());
    }

    #[test]
    fn rejects_expired() {
        let claims = claims_with_exp(-10.0);
        let token = sign_embed(&claims, "s3cret");
        assert!(verify_embed(&token, "s3cret").is_none());
    }

    #[test]
    fn accepts_far_future_expiry() {
        let claims = claims_with_exp(3600.0 * 24.0 * 365.0 * 10.0);
        let token = sign_embed(&claims, "s3cret");
        assert!(verify_embed(&token, "s3cret").is_some());
    }

    #[test]
    fn accepts_no_expiry() {
        let claims = EmbedClaims {
            resource: Some(EmbedResource {
                dashboard: Some("b_no_exp".to_owned()),
            }),
            params: None,
            exp: None,
        };
        let token = sign_embed(&claims, "s3cret");
        assert_eq!(verify_embed(&token, "s3cret"), Some(claims));
    }

    #[test]
    fn rejects_tampered_payload() {
        let claims = claims_with_exp(3600.0);
        let token = sign_embed(&claims, "s3cret");
        let parts: Vec<&str> = token.split('.').collect();
        let tampered_claims = EmbedClaims {
            resource: Some(EmbedResource {
                dashboard: Some("b_evil".to_owned()),
            }),
            ..claims
        };
        let tampered_payload = b64url_json(&tampered_claims).unwrap();
        let tampered = format!("{}.{}.{}", parts[0], tampered_payload, parts[2]);
        assert!(verify_embed(&tampered, "s3cret").is_none());
    }

    #[test]
    fn rejects_tampered_signature() {
        let claims = claims_with_exp(3600.0);
        let token = sign_embed(&claims, "s3cret");
        let parts: Vec<&str> = token.split('.').collect();
        // Flip the signature to something else decodable but wrong.
        let bogus_sig = b64url(b"not-the-real-signature-bytes!!!!");
        let tampered = format!("{}.{}.{}", parts[0], parts[1], bogus_sig);
        assert!(verify_embed(&tampered, "s3cret").is_none());
    }

    #[test]
    fn get_embed_secret_prefers_env_secret_without_touching_clickhouse() {
        let ch = Arc::new(ChClient::new(
            "http://127.0.0.1:1".to_owned(), // unreachable — proves CH is never called
            "default".to_owned(),
            String::new(),
        ));
        let resolver = EmbedSecretResolver::new(Some("env-secret".to_owned()), ch);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let secret = rt.block_on(resolver.get_embed_secret()).unwrap();
        assert_eq!(secret, "env-secret");
    }

    #[tokio::test]
    async fn get_embed_secret_reads_existing_kv_row() {
        let server = MockServer::start().await;
        // CREATE DATABASE / CREATE TABLE (exec — plain body, no FORMAT JSON).
        Mock::given(method("POST"))
            .and(body_string_contains("CREATE DATABASE"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok.\n"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("CREATE TABLE"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok.\n"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("SELECT v FROM console.app_kv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"meta":[{"name":"v","type":"String"}],"data":[{"v":"stored-secret"}],"rows":1}"#,
            ))
            .mount(&server)
            .await;

        let ch = Arc::new(ChClient::new(
            server.uri(),
            "default".to_owned(),
            String::new(),
        ));
        let resolver = EmbedSecretResolver::new(None, ch);
        let secret = resolver.get_embed_secret().await.unwrap();
        assert_eq!(secret, "stored-secret");

        // Second call is served from cache — no new mocks needed, and the
        // formerly-mounted mocks would panic if hit unexpectedly-often only
        // in `.expect()`-style verifiers, so instead just check the value
        // is the same without adding fresh expectations.
        let secret_again = resolver.get_embed_secret().await.unwrap();
        assert_eq!(secret_again, "stored-secret");
    }

    #[tokio::test]
    async fn get_embed_secret_generates_and_persists_when_absent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("CREATE DATABASE"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok.\n"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("CREATE TABLE"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok.\n"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("SELECT v FROM console.app_kv"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    r#"{"meta":[{"name":"v","type":"String"}],"data":[],"rows":0}"#,
                ),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("INSERT INTO console.app_kv"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok.\n"))
            .mount(&server)
            .await;

        let ch = Arc::new(ChClient::new(
            server.uri(),
            "default".to_owned(),
            String::new(),
        ));
        let resolver = EmbedSecretResolver::new(None, ch);
        let secret = resolver.get_embed_secret().await.unwrap();
        // 32 bytes -> 64 lowercase hex characters.
        assert_eq!(secret.len(), 64);
        assert!(
            secret
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }
}
