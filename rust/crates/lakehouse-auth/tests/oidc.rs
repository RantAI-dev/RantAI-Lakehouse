//! Tests for `lakehouse_auth::oidc` against a locally generated key and a
//! `wiremock` JWKS server — never a real identity provider.
//!
//! # What needs Postgres, and what doesn't
//!
//! Every failure path in [`lakehouse_auth::oidc::OidcAuthenticator::authenticate`]
//! that is decided during token validation (wrong `iss`/`aud`, expired,
//! `nbf` in the future, `alg: none`, a signature from the wrong key, an
//! unknown `kid`) never reaches a database query — [`validate_token`
//! ](lakehouse_auth::oidc) returns before [`resolve_principal`
//! ](lakehouse_auth::oidc) runs. Those tests build their `PgPool` with
//! [`lakehouse_store::connect_lazy`] (no I/O at construction — same
//! contract [`lakehouse_store`] itself documents) and run as plain
//! `#[tokio::test]`s, unconditionally, exactly like every other test in
//! this workspace.
//!
//! Anything that reaches a successful `authenticate` (a correctly signed
//! token, JWKS caching/rotation, JIT provisioning, role mapping) needs a
//! real `auth_identity`/`app_user`/`role` schema, so those tests follow the
//! same `#[sqlx::test(migrations = "../../migrations")]` pattern as
//! `tests/repository.rs` and `tests/session.rs` — see those files' doc
//! comments for how the backing Postgres is provided.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use std::time::Duration;

use base64::Engine;
use jsonwebtoken::jwk::{
    AlgorithmParameters, CommonParameters, Jwk, JwkSet, RSAKeyParameters, RSAKeyType,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use lakehouse_auth::oidc::{JwksClient, OidcAuthenticator};
use lakehouse_auth::{AuthError, Authenticator, Credential, OidcConfig, Secret};
use rsa::RsaPrivateKey;
use rsa::pkcs1::{EncodeRsaPrivateKey, LineEnding};
use rsa::traits::PublicKeyParts;
use serde::Serialize;
use sqlx::PgPool;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ISSUER: &str = "https://issuer.example";
const CLIENT_ID: &str = "client-1";

/// A syntactically valid Postgres URL that is never actually connected to
/// in the tests that use it — see [`lazy_pool`].
const UNREACHABLE_DATABASE_URL: &str = "postgres://lakehouse:lakehouse@localhost:5432/lakehouse";

/// A `PgPool` for tests whose code path never reaches a query — every
/// failure decided inside token validation itself. `connect_lazy` performs
/// no I/O (see `lakehouse_store::connect_lazy`'s doc comment), so this
/// never blocks on, or requires, a live Postgres.
fn lazy_pool() -> PgPool {
    lakehouse_store::connect_lazy(UNREACHABLE_DATABASE_URL).expect("valid postgres URL")
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A freshly generated RSA keypair plus the JWK describing its public half,
/// for signing test tokens without ever touching a real `IdP`.
struct TestKey {
    kid: String,
    encoding_key: EncodingKey,
    jwk: Jwk,
}

impl TestKey {
    fn generate(kid: &str) -> Self {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("keygen");
        let public_key = private_key.to_public_key();
        let pem = private_key.to_pkcs1_pem(LineEnding::LF).expect("pkcs1 pem");
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key");

        let jwk = Jwk {
            common: CommonParameters {
                key_id: Some(kid.to_owned()),
                ..Default::default()
            },
            algorithm: AlgorithmParameters::RSA(RSAKeyParameters {
                key_type: RSAKeyType::RSA,
                n: b64url(&public_key.n().to_bytes_be()),
                e: b64url(&public_key.e().to_bytes_be()),
            }),
        };

        Self {
            kid: kid.to_owned(),
            encoding_key,
            jwk,
        }
    }
}

#[derive(Serialize)]
struct Claims<'a> {
    sub: &'a str,
    iss: &'a str,
    aud: &'a str,
    exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    nbf: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<&'a str>>,
}

impl<'a> Claims<'a> {
    fn valid(sub: &'a str) -> Self {
        Self {
            sub,
            iss: ISSUER,
            aud: CLIENT_ID,
            exp: now() + 3600,
            nbf: None,
            email: None,
            name: None,
            groups: None,
        }
    }
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn sign(claims: &Claims<'_>, key: &TestKey, alg: Algorithm) -> String {
    let mut header = Header::new(alg);
    header.kid = Some(key.kid.clone());
    encode(&header, claims, &key.encoding_key).expect("sign")
}

/// Build an authenticator pointed at `server`'s `/jwks` endpoint, caching
/// for `ttl`.
fn authenticator(server: &MockServer, pool: PgPool, ttl: Duration, jit: bool) -> OidcAuthenticator {
    let jwks_url = format!("{}/jwks", server.uri());
    let mut config = OidcConfig::new(ISSUER, CLIENT_ID, "test", jwks_url.clone());
    config.jit_provisioning = jit;
    let jwks = JwksClient::with_ttl(jwks_url, ttl);
    OidcAuthenticator::with_jwks_client(config, jwks, pool)
}

async fn mount_jwks(server: &MockServer, keys: &[&TestKey]) {
    let set = JwkSet {
        keys: keys.iter().map(|k| k.jwk.clone()).collect(),
    };
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&set))
        .mount(server)
        .await;
}

// ── Token-validation failure paths: no Postgres reachability needed ───────

#[tokio::test]
async fn wrong_issuer_is_rejected() {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, lazy_pool(), Duration::from_secs(300), true);

    let mut claims = Claims::valid("user-1");
    claims.iss = "https://not-the-configured-issuer.example";
    let token = sign(&claims, &key, Algorithm::RS256);
    let err = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
}

#[tokio::test]
async fn wrong_audience_is_rejected() {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, lazy_pool(), Duration::from_secs(300), true);

    let mut claims = Claims::valid("user-1");
    claims.aud = "some-other-client";
    let token = sign(&claims, &key, Algorithm::RS256);
    let err = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
}

#[tokio::test]
async fn an_expired_token_is_rejected() {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, lazy_pool(), Duration::from_secs(300), true);

    let mut claims = Claims::valid("user-1");
    claims.exp = now() - 3600;
    let token = sign(&claims, &key, Algorithm::RS256);
    let err = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
}

/// A token not-yet-valid (`nbf` in the future, beyond configured skew) is
/// rejected.
#[tokio::test]
async fn a_not_yet_valid_token_is_rejected() {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, lazy_pool(), Duration::from_secs(300), true);

    let mut claims = Claims::valid("user-1");
    claims.nbf = Some(now() + 3600);
    let token = sign(&claims, &key, Algorithm::RS256);
    let err = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
}

/// `alg: none` — a token with an unsigned/empty signature segment claiming
/// `alg: none` in its header must never validate. `Algorithm` (what
/// `jsonwebtoken` parses the header into) has no `none` variant at all, so
/// this is rejected at header-parsing time, before any key lookup.
#[tokio::test]
async fn alg_none_is_rejected() {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, lazy_pool(), Duration::from_secs(300), true);

    let header =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        r#"{{"sub":"user-1","iss":"{ISSUER}","aud":"{CLIENT_ID}","exp":9999999999}}"#
    ));
    let forged = format!("{header}.{payload}.");

    let err = auth
        .authenticate(&Credential::Bearer(Secret::new(forged)))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
}

/// A token signed by a key that is NOT the one published in the JWKS (the
/// classic "attacker has their own keypair" case) must be rejected even
/// though its `kid` matches a real entry — the signature itself has to
/// verify, not just the `kid` lookup succeed.
#[tokio::test]
async fn a_signature_from_the_wrong_key_is_rejected() {
    let server = MockServer::start().await;
    let published_key = TestKey::generate("kid-1");
    let attacker_key = TestKey::generate("kid-1"); // same kid, different keypair
    mount_jwks(&server, &[&published_key]).await;
    let auth = authenticator(&server, lazy_pool(), Duration::from_secs(300), true);

    let token = sign(&Claims::valid("user-1"), &attacker_key, Algorithm::RS256);
    let err = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
}

/// A `kid` this service has never seen (and the refetched JWKS still
/// doesn't contain) is rejected.
#[tokio::test]
async fn an_unknown_kid_is_rejected() {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    let unknown_key = TestKey::generate("kid-does-not-exist");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, lazy_pool(), Duration::from_secs(300), true);

    let token = sign(&Claims::valid("user-1"), &unknown_key, Algorithm::RS256);
    let err = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
}

/// A token whose header carries no `kid` at all is rejected rather than
/// guessing at a key.
#[tokio::test]
async fn a_missing_kid_is_rejected() {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, lazy_pool(), Duration::from_secs(300), true);

    let header = Header::new(Algorithm::RS256); // no `kid` set
    let token = encode(&header, &Claims::valid("user-1"), &key.encoding_key).unwrap();
    let err = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
}

/// A non-`Bearer` credential is rejected as unsupported, per the same
/// `Authenticator` contract every other implementor follows.
#[tokio::test]
async fn a_non_bearer_credential_is_unsupported() {
    let server = MockServer::start().await;
    let auth = authenticator(&server, lazy_pool(), Duration::from_secs(300), true);
    let err = auth
        .authenticate(&Credential::ServiceToken(Secret::new("whatever")))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::UnsupportedCredential));
}

// ── JWKS caching, rotation, identity resolution, JIT, role mapping ────────
// (every test below reaches a real database query and needs live Postgres)

const RINA: &str = "33333333-3333-4333-8333-000000000001";

#[sqlx::test(migrations = "../../migrations")]
async fn a_correctly_signed_token_validates(pool: PgPool) -> sqlx::Result<()> {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, pool, Duration::from_secs(300), true);

    let token = sign(&Claims::valid("user-1"), &key, Algorithm::RS256);
    let credential = Credential::Bearer(Secret::new(token));
    let principal = auth.authenticate(&credential).await.unwrap();
    assert_eq!(principal.provider, "oidc:test");
    Ok(())
}

/// A token whose `nbf` is only slightly in the future (within the
/// configured clock-skew leeway) is accepted — this is what "small
/// configurable clock skew" means in practice.
#[sqlx::test(migrations = "../../migrations")]
async fn a_token_within_the_configured_clock_skew_is_accepted(pool: PgPool) -> sqlx::Result<()> {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let jwks_url = format!("{}/jwks", server.uri());
    let mut config = OidcConfig::new(ISSUER, CLIENT_ID, "test", jwks_url.clone());
    config.clock_skew_seconds = 120;
    config.jit_provisioning = true;
    let jwks = JwksClient::with_ttl(jwks_url, Duration::from_secs(300));
    let auth = OidcAuthenticator::with_jwks_client(config, jwks, pool);

    let mut claims = Claims::valid("user-within-skew");
    claims.nbf = Some(now() + 30);
    let token = sign(&claims, &key, Algorithm::RS256);
    let principal = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap();
    assert_eq!(principal.provider, "oidc:test");
    Ok(())
}

/// JWKS caching: two validations against the same `kid` within the cache
/// TTL must hit the network exactly once.
#[sqlx::test(migrations = "../../migrations")]
async fn a_second_validation_within_ttl_does_not_refetch_the_jwks(
    pool: PgPool,
) -> sqlx::Result<()> {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, pool, Duration::from_secs(300), true);

    let token = sign(&Claims::valid("user-1"), &key, Algorithm::RS256);
    auth.authenticate(&Credential::Bearer(Secret::new(token.clone())))
        .await
        .unwrap();
    auth.authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "expected exactly one JWKS fetch across two validations of the same kid"
    );
    Ok(())
}

/// Key rotation: a `kid` the cache doesn't recognize triggers exactly one
/// refetch, and the newly rotated key is accepted immediately — no waiting
/// out the cache TTL.
#[sqlx::test(migrations = "../../migrations")]
async fn a_rotated_key_is_picked_up_on_first_use_without_waiting_out_the_ttl(
    pool: PgPool,
) -> sqlx::Result<()> {
    let server = MockServer::start().await;
    let old_key = TestKey::generate("kid-old");
    mount_jwks(&server, &[&old_key]).await;
    // Long TTL: if rotation relied on TTL expiry, this test would fail.
    let auth = authenticator(&server, pool, Duration::from_secs(3600), true);

    let old_token = sign(&Claims::valid("user-1"), &old_key, Algorithm::RS256);
    auth.authenticate(&Credential::Bearer(Secret::new(old_token)))
        .await
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);

    // Provider rotates: publish a JWKS with both the old and a new key
    // (typical rotation practice — old key stays valid briefly).
    server.reset().await;
    let new_key = TestKey::generate("kid-new");
    mount_jwks(&server, &[&old_key, &new_key]).await;

    let new_token = sign(&Claims::valid("user-1"), &new_key, Algorithm::RS256);
    let principal = auth
        .authenticate(&Credential::Bearer(Secret::new(new_token)))
        .await
        .unwrap();
    assert_eq!(principal.provider, "oidc:test");
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "the unknown kid must trigger exactly one refetch against the new mock"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_subject_is_rejected_when_jit_is_off(pool: PgPool) -> sqlx::Result<()> {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, pool, Duration::from_secs(300), false);

    let token = sign(&Claims::valid("brand-new-subject"), &key, Algorithm::RS256);
    let err = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_subject_is_provisioned_and_linked_when_jit_is_on(
    pool: PgPool,
) -> sqlx::Result<()> {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, pool.clone(), Duration::from_secs(300), true);

    let mut claims = Claims::valid("brand-new-subject");
    claims.email = Some("new.person@example.com");
    claims.name = Some("New Person");
    let token = sign(&claims, &key, Algorithm::RS256);
    let principal = auth
        .authenticate(&Credential::Bearer(Secret::new(token.clone())))
        .await
        .unwrap();
    assert_eq!(principal.display_name, "New Person");
    assert_eq!(principal.provider, "oidc:test");

    // A second login with the same subject finds the linked identity
    // rather than provisioning a duplicate account.
    let principal_again = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap();
    assert_eq!(principal_again.id, principal.id);

    let user_count: (i64,) = sqlx::query_as("SELECT count(*) FROM app_user WHERE email = $1")
        .bind("new.person@example.com")
        .fetch_one(&pool)
        .await?;
    assert_eq!(user_count.0, 1);
    Ok(())
}

/// A subject whose email already belongs to an existing local account links
/// to it instead of creating a duplicate.
#[sqlx::test(migrations = "../../migrations")]
async fn jit_links_to_an_existing_user_with_the_same_email(pool: PgPool) -> sqlx::Result<()> {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;
    let auth = authenticator(&server, pool, Duration::from_secs(300), true);

    // Rina Wijaya (seeded, `0002_seed_identity.sql`) already has a `local`
    // identity under rina@meridian.example.
    let mut claims = Claims::valid("rina-oidc-subject");
    claims.email = Some("rina@meridian.example");
    let token = sign(&claims, &key, Algorithm::RS256);
    let principal = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap();

    let rina_id = uuid::Uuid::parse_str(RINA).unwrap();
    assert_eq!(principal.id.uuid(), rina_id);
    Ok(())
}

/// A mapped `IdP` group grants its role's permissions on top of whatever the
/// user's locally-assigned roles already grant (union, not replacement) —
/// see `OidcAuthenticator::mapped_permissions`'s doc comment for why.
#[sqlx::test(migrations = "../../migrations")]
async fn a_mapped_group_grants_its_roles_permissions(pool: PgPool) -> sqlx::Result<()> {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;

    let jwks_url = format!("{}/jwks", server.uri());
    let mut config = OidcConfig::new(ISSUER, CLIENT_ID, "test", jwks_url.clone());
    config.jit_provisioning = false;
    config
        .role_map
        .insert("lakehouse-admins".to_owned(), "Platform Admin".to_owned());
    // Link an OIDC identity to Rina (seeded, has a `local` identity already)
    // directly, so this test doesn't need JIT.
    sqlx::query(
        "INSERT INTO auth_identity (provider, external_subject, app_user_id, password_hash) \
         VALUES ('oidc:test', 'rina-oidc-subject-2', $1, NULL)",
    )
    .bind(uuid::Uuid::parse_str(RINA).unwrap())
    .execute(&pool)
    .await?;

    let jwks = JwksClient::with_ttl(jwks_url, Duration::from_secs(300));
    let auth = OidcAuthenticator::with_jwks_client(config, jwks, pool);

    let mut claims = Claims::valid("rina-oidc-subject-2");
    claims.groups = Some(vec!["lakehouse-admins"]);
    let token = sign(&claims, &key, Algorithm::RS256);
    let principal = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap();

    // Rina's local roles (Analyst + Approver) never grant this; only the
    // mapped Platform Admin role does.
    assert!(principal.has("identity:write"));
    // Her local-only grants are still present (union, not replacement).
    assert!(principal.has("query:read"));
    Ok(())
}

/// An unmapped group is ignored, not fatal.
#[sqlx::test(migrations = "../../migrations")]
async fn an_unmapped_group_grants_nothing_extra(pool: PgPool) -> sqlx::Result<()> {
    let server = MockServer::start().await;
    let key = TestKey::generate("kid-1");
    mount_jwks(&server, &[&key]).await;

    let jwks_url = format!("{}/jwks", server.uri());
    let mut config = OidcConfig::new(ISSUER, CLIENT_ID, "test", jwks_url.clone());
    config.jit_provisioning = false;
    config
        .role_map
        .insert("lakehouse-admins".to_owned(), "Platform Admin".to_owned());
    sqlx::query(
        "INSERT INTO auth_identity (provider, external_subject, app_user_id, password_hash) \
         VALUES ('oidc:test', 'rina-oidc-subject-3', $1, NULL)",
    )
    .bind(uuid::Uuid::parse_str(RINA).unwrap())
    .execute(&pool)
    .await?;

    let jwks = JwksClient::with_ttl(jwks_url, Duration::from_secs(300));
    let auth = OidcAuthenticator::with_jwks_client(config, jwks, pool);

    let mut claims = Claims::valid("rina-oidc-subject-3");
    claims.groups = Some(vec!["some-unrelated-group"]);
    let token = sign(&claims, &key, Algorithm::RS256);
    let principal = auth
        .authenticate(&Credential::Bearer(Secret::new(token)))
        .await
        .unwrap();

    assert!(!principal.has("identity:write"));
    assert!(principal.has("query:read"));
    Ok(())
}
