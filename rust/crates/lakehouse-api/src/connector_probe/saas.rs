//! The connection test of a `saas` connector (`SRC-3` T1.4, plan AD2, AD8).
//!
//! One bounded `GET` of the vendor's "who am I" request
//! (`SaasVendor::whoami`), at an address the vendor table builds from the
//! connector's `account`. The same rules as the `rest` test:
//!
//! - the address is checked ([`resolve_checked`]) and pinned
//!   ([`pinned_http_client`]) BEFORE any credential is resolved, so a
//!   refused address makes no call to the secret store;
//! - redirects are reported, never followed, and the system proxy is not used;
//! - every message is fixed text built from the vendor's label and the status
//!   class; nothing the vendor said is echoed (`AGENTS.md` principle 4).
//!
//! # What this slice tests
//!
//! A browser sign-in (refresh token, then an access token from the vendor's
//! token endpoint, the client authenticating as the vendor table says), an integration credential that is a single token, Zendesk's
//! email + API token, and Google Analytics' service-account key (an RS256 JWT-bearer exchange,
//! slice 3 review BLOCKER 10). An
//! integration credential of another shape (a pair, a key file) answers
//! `supported: false` with a reason until its vendor's own task teaches this
//! module the vendor's scheme (plan Vn.3): "unsupported, honestly".

use std::time::Instant;

use lakehouse_core::secret::{DynSecretResolver, SecretValue};
use lakehouse_store::connectors::ConnectorDialInfo;
use lakehouse_store::ingest_spec::{Dial, SaasAuth};
use lakehouse_store::saas_vendors::{self, ClientAuth, SaasVendor};

use super::{
    DIAL_TIMEOUT, Outcome, classify_reqwest_error, parse_endpoint_host_port, pinned_http_client,
    resolve_checked, token_failure_outcome,
};
use crate::connector_oauth::{self, OAuthSettings};
use crate::internal_hosts::InternalHosts;

/// What one test asks: the vendor, the account as the person typed it, the
/// request address and the token address (both built from the vendor table),
/// and how the connector signs in.
struct Request<'a> {
    vendor: &'static SaasVendor,
    account: &'a str,
    url: String,
    token_url: String,
    auth: &'a SaasAuth,
}

/// The connector's credential references and the resolver that reads them.
struct Secrets<'a> {
    primary: &'a str,
    secondary: Option<&'a str>,
    resolver: &'a dyn DynSecretResolver,
}

/// Parses `info.dial` as a `saas` dial, builds the addresses from the vendor
/// table and runs the test.
pub(super) async fn probe_saas_info(
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
    oauth: OAuthSettings,
) -> Outcome {
    let dial = match Dial::parse("saas", &info.dial) {
        Ok(Dial::Saas(dial)) => dial,
        Ok(_) => {
            return Outcome::misconfigured(
                "connector is misconfigured: its dial does not match its own adapter \"saas\"",
            );
        }
        Err(err) => return Outcome::misconfigured(format!("connector's dial is invalid: {err}")),
    };
    let Some(vendor) = saas_vendors::vendor(&dial.vendor) else {
        return Outcome::misconfigured("connector is misconfigured: its vendor is not offered");
    };
    let Ok(parts) = vendor.validate_account(&dial.account) else {
        return Outcome::misconfigured("connector is misconfigured: its account is not valid");
    };
    let Some(whoami) = vendor.whoami else {
        return Outcome::not_testable_with(format!(
            "{} has no generic connection test in this build. A load reports whether the \
             connection worked.",
            vendor.label
        ));
    };
    let request = Request {
        vendor,
        account: &dial.account,
        // Slice 2 review SHOULD-FIX 5: the gate-only mock origin, ignored unless the
        // gate-only switch is on (`connector_oauth::gate_rebase`).
        url: connector_oauth::gate_rebase(
            oauth,
            vendor.id,
            &SaasVendor::render(&format!("{}{}", vendor.api_base, whoami.path), &parts),
        ),
        token_url: connector_oauth::gate_rebase(
            oauth,
            vendor.id,
            &SaasVendor::render(vendor.oauth.token_url, &parts),
        ),
        auth: &dial.auth,
    };
    let secrets = Secrets {
        primary: &info.secret_ref,
        secondary: info.secret_ref_secondary.as_deref(),
        resolver,
    };
    probe_saas_at(&request, &secrets, internal_hosts, oauth).await
}

/// The access token for one test, and the refresh token a rotating vendor
/// issued while it was obtained.
struct Access {
    token: SecretValue,
    rotated: Option<SecretValue>,
    /// `token` is the base64 of an HTTP Basic credential, not a bearer token.
    basic: bool,
}

/// Resolve one credential reference, or the outcome that ends the test.
async fn resolve(
    resolver: &dyn DynSecretResolver,
    reference: &str,
) -> Result<SecretValue, Outcome> {
    resolver.resolve_dyn(reference).await.map_err(|err| {
        Outcome::misconfigured(format!(
            "could not resolve the connector's credential: {err}"
        ))
    })
}

/// Resolve the credential and obtain an access token, or the outcome that
/// ends the test instead.
async fn access_token(
    request: &Request<'_>,
    secrets: &Secrets<'_>,
    internal_hosts: &InternalHosts,
    oauth: OAuthSettings,
) -> Result<Access, Outcome> {
    match request.auth {
        // Atlassian's browser sign-in is only accepted at `api.atlassian.com/ex/confluence/{cloudId}`,
        // and the cloud id comes from a second request this build does not make: "unsupported,
        // honestly" (slice 4, V-Confluence.3). Nothing is sent to the vendor.
        SaasAuth::Oauth2AuthorizationCode { .. } if request.vendor.oauth.api_base.is_some() => {
            Err(Outcome::not_testable_with(format!(
                "{}'s browser sign-in is used through a gateway address that the connection test \
                 does not reach yet. A load reports whether the connection worked.",
                request.vendor.label
            )))
        }
        SaasAuth::Oauth2AuthorizationCode { client_id } => {
            refreshed_access(request, client_id, secrets, internal_hosts, oauth).await
        }
        SaasAuth::Integration => integration_access(request, secrets, internal_hosts, oauth).await,
    }
}

/// A browser sign-in: exchange the saved refresh token for an access token,
/// the client authenticating as the vendor table says (PR #214 CI clippy:
/// split out of [`access_token`], behaviour unchanged).
async fn refreshed_access(
    request: &Request<'_>,
    client_id: &str,
    secrets: &Secrets<'_>,
    internal_hosts: &InternalHosts,
    oauth: OAuthSettings,
) -> Result<Access, Outcome> {
    let client_secret = resolve(secrets.resolver, secrets.primary).await?;
    // No refresh token means nobody has signed in: nothing is wrong
    // with the connector, so this is `supported: false` with the
    // reason, never a failure that would stop the client secret being
    // saved before the first sign-in (as for `rest`, `SRC-12`).
    let refresh_token = match secrets.secondary {
        Some(secondary) => secrets.resolver.resolve_dyn(secondary).await.ok(),
        None => None,
    };
    let Some(refresh_token) = refresh_token else {
        return Err(Outcome::not_testable_with(
            "Nobody has signed in to this connector yet, so the connection cannot be \
             tested. Use Sign in on the connector page, then test again.",
        ));
    };
    let started = Instant::now();
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.expose_secret()),
    ];
    let tokens = match request.vendor.oauth.client_auth {
        ClientAuth::Post => {
            connector_oauth::token_request(
                &request.token_url,
                &[
                    form[0],
                    form[1],
                    ("client_id", client_id),
                    ("client_secret", client_secret.expose_secret()),
                ],
                internal_hosts,
                oauth,
            )
            .await
        }
        // Slice 2 review BLOCKER 3 (`client_secret_post_json`): as `Post`, a JSON body.
        ClientAuth::PostJson => {
            connector_oauth::token_request_json(
                &request.token_url,
                &[
                    form[0],
                    form[1],
                    ("client_id", client_id),
                    ("client_secret", client_secret.expose_secret()),
                ],
                internal_hosts,
                oauth,
            )
            .await
        }
        // `SRC-3` AD8: the client is in the header, not the body.
        ClientAuth::Basic => {
            connector_oauth::token_request_basic(
                &request.token_url,
                &form,
                client_id,
                client_secret.expose_secret(),
                internal_hosts,
                oauth,
            )
            .await
        }
    }
    .map_err(|failure| token_failure_outcome(&failure, started.elapsed()))?;
    Ok(Access {
        token: tokens.access_token,
        rotated: tokens.refresh_token,
        basic: false,
    })
}

/// The vendor's integration credential: one token, or Zendesk's email + API
/// token (PR #214 CI clippy: split out of [`access_token`], behaviour unchanged).
async fn integration_access(
    request: &Request<'_>,
    secrets: &Secrets<'_>,
    internal_hosts: &InternalHosts,
    oauth: OAuthSettings,
) -> Result<Access, Outcome> {
    let fields = request
        .vendor
        .integration
        .map_or(&[][..], |integration| integration.secret_fields);
    // Slice 2 (V-Zendesk.3): Zendesk's email + API token is HTTP Basic with `<email>/token` as
    // the user name
    // (<https://developer.zendesk.com/api-reference/introduction/security-and-auth/>); slice 4
    // (V-Confluence.3): Atlassian's is HTTP Basic with the email as the user name
    // (<https://developer.atlassian.com/cloud/confluence/basic-auth-for-rest-apis/>).
    if matches!(request.vendor.id, "zendesk" | "confluence") && fields == ["email", "apiToken"] {
        let email = resolve(secrets.resolver, secrets.primary).await?;
        let Some(secondary) = secrets.secondary else {
            return Err(Outcome::misconfigured(format!(
                "connector is misconfigured: the {} API token is not saved",
                request.vendor.label
            )));
        };
        let api_token = resolve(secrets.resolver, secondary).await?;
        let user = if request.vendor.id == "zendesk" {
            format!("{}/token", email.expose_secret())
        } else {
            email.expose_secret().to_owned()
        };
        let pair = format!("{user}:{}", api_token.expose_secret());
        return Ok(Access {
            token: SecretValue::new(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                pair,
            )),
            rotated: None,
            basic: true,
        });
    }
    // Slice 4 (V-ServiceNow.3, decision 3): ServiceNow's client credentials are exchanged for a token.
    if request.vendor.id == "servicenow" && fields == ["clientId", "clientSecret"] {
        return client_credentials_access(request, secrets, internal_hosts, oauth).await;
    }
    // Slice 3 review BLOCKER 10 (decision 3): Google's service-account key is exchanged for a token.
    if request.vendor.id == "ga4" && fields == ["serviceAccountJson"] {
        return service_account_access(request, secrets, internal_hosts, oauth).await;
    }
    if fields != ["token"] {
        return Err(Outcome::not_testable_with(format!(
            "This build cannot test {}'s integration credential from the console yet. \
             A load reports whether the connection worked.",
            request.vendor.label
        )));
    }
    Ok(Access {
        token: resolve(secrets.resolver, secrets.primary).await?,
        rotated: None,
        basic: false,
    })
}

/// ServiceNow's client-credentials grant (<https://www.servicenow.com/docs/r/platform-security/authentication/client-credentials-grant-workflow.html>):
/// the client id and secret are resolved only now (after the address check) and sent in the
/// form body of the token request, the address being the vendor table's for this instance.
async fn client_credentials_access(
    request: &Request<'_>,
    secrets: &Secrets<'_>,
    internal_hosts: &InternalHosts,
    oauth: OAuthSettings,
) -> Result<Access, Outcome> {
    let client_id = resolve(secrets.resolver, secrets.primary).await?;
    let Some(secondary) = secrets.secondary else {
        return Err(Outcome::misconfigured(format!(
            "connector is misconfigured: the {} client secret is not saved",
            request.vendor.label
        )));
    };
    let client_secret = resolve(secrets.resolver, secondary).await?;
    let started = Instant::now();
    let form = [
        ("grant_type", "client_credentials"),
        ("client_id", client_id.expose_secret()),
        ("client_secret", client_secret.expose_secret()),
    ];
    let tokens = connector_oauth::token_request(&request.token_url, &form, internal_hosts, oauth)
        .await
        .map_err(|failure| token_failure_outcome(&failure, started.elapsed()))?;
    Ok(Access {
        token: tokens.access_token,
        rotated: None,
        basic: false,
    })
}

/// The claims of Google's JWT-bearer assertion
/// (<https://developers.google.com/identity/protocols/oauth2/service-account>):
/// `iss` the service account, `scope`, `aud` always Google's token address, `iat`, and
/// `exp` at most one hour after `iat`.
#[derive(serde::Serialize)]
struct AssertionClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
}

/// An assertion lives 50 minutes: under Google's one-hour maximum, with room for a clock
/// that runs fast.
const ASSERTION_SECONDS: u64 = 3000;

/// `(client_email, private_key)` of a service-account key file, the key kept as a
/// [`SecretValue`] so it is never printed. `None` for anything else.
fn parse_service_account(file: &SecretValue) -> Option<(String, SecretValue)> {
    let document: serde_json::Value = serde_json::from_str(file.expose_secret()).ok()?;
    if document.get("type")?.as_str()? != "service_account" {
        return None;
    }
    Some((
        document.get("client_email")?.as_str()?.to_owned(),
        SecretValue::new(document.get("private_key")?.as_str()?.to_owned()),
    ))
}

/// The signed assertion (RS256, with the workspace's `jsonwebtoken`) for `vendor`'s first
/// scope, addressed to the vendor table's token address and never to anything in the key
/// file. `None` when the key is not an RSA private key or the clock is unreadable.
fn sign_assertion(
    vendor: &SaasVendor,
    email: &str,
    key: &SecretValue,
    now: u64,
) -> Option<SecretValue> {
    let claims = AssertionClaims {
        iss: email,
        scope: vendor.oauth.scopes.first()?,
        aud: vendor.oauth.token_url,
        iat: now,
        exp: now + ASSERTION_SECONDS,
    };
    let encoding = jsonwebtoken::EncodingKey::from_rsa_pem(key.expose_secret().as_bytes()).ok()?;
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    jsonwebtoken::encode(&header, &claims, &encoding)
        .ok()
        .map(SecretValue::new)
}

/// Google's service-account key (slice 3 review BLOCKER 10, decision 3): the key file is
/// resolved only now (after the address check), signed into an RS256 assertion and
/// exchanged at the token address with `grant_type=...:jwt-bearer` for a bearer token.
/// A key file that is not one, or a key that cannot sign, is a fixed misconfiguration
/// message that quotes nothing of the file.
async fn service_account_access(
    request: &Request<'_>,
    secrets: &Secrets<'_>,
    internal_hosts: &InternalHosts,
    oauth: OAuthSettings,
) -> Result<Access, Outcome> {
    let file = resolve(secrets.resolver, secrets.primary).await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let assertion = parse_service_account(&file)
        .and_then(|(email, key)| sign_assertion(request.vendor, &email, &key, now))
        .ok_or_else(|| {
            Outcome::misconfigured(
                "connector is misconfigured: the Google service-account key is not a valid key file",
            )
        })?;
    let started = Instant::now();
    let form = [
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", assertion.expose_secret()),
    ];
    let tokens = connector_oauth::token_request(&request.token_url, &form, internal_hosts, oauth)
        .await
        .map_err(|failure| token_failure_outcome(&failure, started.elapsed()))?;
    Ok(Access {
        token: tokens.access_token,
        rotated: None,
        basic: false,
    })
}

/// The fixed sentence for a status the vendor answered with.
fn rejected(label: &str, status: u16) -> String {
    match status {
        401 => format!("{label} refused the sign-in. Sign in again or check the credential."),
        403 => format!(
            "{label} accepted the sign-in but the account lacks a permission the connector needs."
        ),
        429 => format!("{label} limited the rate of requests. Try again later."),
        other => format!("{label} request rejected: HTTP {other}"),
    }
}

/// Run the test for `request`. Split from [`probe_saas_info`] so a test can
/// point the vendor's address at a local server.
async fn probe_saas_at(
    request: &Request<'_>,
    secrets: &Secrets<'_>,
    internal_hosts: &InternalHosts,
    oauth: OAuthSettings,
) -> Outcome {
    let label = request.vendor.label;
    let Some((host, port)) = parse_endpoint_host_port(&request.url) else {
        return Outcome::misconfigured("connector is misconfigured: its address is not valid");
    };
    let approved = match resolve_checked(host, port, internal_hosts).await {
        Ok(approved) => approved,
        Err(message) => return Outcome::misconfigured(message),
    };
    let client = match pinned_http_client(&request.url, host, port, &approved) {
        Ok(client) => client,
        Err(message) => return Outcome::misconfigured(message),
    };
    // Only now are credentials read (AGENTS.md, `SEC-15`).
    let access = match access_token(request, secrets, internal_hosts, oauth).await {
        Ok(access) => access,
        Err(outcome) => return outcome,
    };
    let started = Instant::now();
    let get = client.get(&request.url);
    let get = if access.basic {
        get.header(
            reqwest::header::AUTHORIZATION,
            format!("Basic {}", access.token.expose_secret()),
        )
    } else {
        get.bearer_auth(access.token.expose_secret())
    };
    let attempt = tokio::time::timeout(DIAL_TIMEOUT, get.send()).await;
    let elapsed = started.elapsed();
    let mut outcome = match attempt {
        // A redirect is reported, never followed; the `Location` is the
        // vendor's text and may name an address.
        Ok(Ok(response)) if response.status().is_redirection() => Outcome::failure(
            elapsed,
            format!(
                "{label} answered with a redirect (HTTP {}); connection tests do not follow \
                 redirects",
                response.status().as_u16()
            ),
        ),
        Ok(Ok(response)) if response.status().is_success() => Outcome::success(
            elapsed,
            format!("Connected to {label} account {}.", request.account),
        ),
        Ok(Ok(response)) => Outcome::failure(elapsed, rejected(label, response.status().as_u16())),
        Ok(Err(err)) => Outcome::failure(
            elapsed,
            format!("{label} request failed: {}", classify_reqwest_error(&err)),
        ),
        Err(_) => Outcome::failure(
            elapsed,
            format!(
                "{label} request timed out after {}s",
                DIAL_TIMEOUT.as_secs()
            ),
        ),
    };
    // A vendor that rotated the refresh token has retired the old one:
    // hand the new one back whatever the request answered.
    outcome.rotated_refresh_token = access.rotated;
    outcome
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_core::secret::EnvSecretResolver;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// A resolver that panics when asked for anything: proves an address
    /// refusal happens before any credential is read.
    #[derive(Debug)]
    struct PanicIfCalledResolver;

    #[async_trait::async_trait]
    impl DynSecretResolver for PanicIfCalledResolver {
        async fn resolve_dyn(
            &self,
            _secret_ref: &str,
        ) -> Result<SecretValue, lakehouse_core::secret::SecretError> {
            panic!("a credential was resolved before the address was approved");
        }
    }

    fn resolver() -> EnvSecretResolver {
        let mut map = HashMap::new();
        map.insert("SAAS_TEST_TOKEN".to_owned(), "token-value".to_owned());
        map.insert("SAAS_TEST_SECRET".to_owned(), "client-secret".to_owned());
        EnvSecretResolver::with_map(map)
    }

    fn request<'a>(
        vendor: &str,
        url: String,
        token_url: String,
        auth: &'a SaasAuth,
    ) -> Request<'a> {
        Request {
            vendor: saas_vendors::vendor(vendor).unwrap(),
            account: "acme",
            url,
            token_url,
            auth,
        }
    }

    async fn answer(status: u16) -> (MockServer, Outcome) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/who"))
            .and(header("authorization", "Bearer token-value"))
            .respond_with(ResponseTemplate::new(status).set_body_string("vendor secret text"))
            .mount(&server)
            .await;
        let resolver = resolver();
        let auth = SaasAuth::Integration;
        // HubSpot's integration credential is one token.
        let outcome = probe_saas_at(
            &request(
                "hubspot",
                format!("{}/who", server.uri()),
                String::new(),
                &auth,
            ),
            &Secrets {
                primary: "env:SAAS_TEST_TOKEN",
                secondary: None,
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings::default(),
        )
        .await;
        (server, outcome)
    }

    #[tokio::test]
    async fn a_200_is_a_success_that_names_the_vendor_and_account() {
        let (_server, outcome) = answer(200).await;
        assert!(outcome.ok && outcome.supported, "{}", outcome.message);
        assert!(outcome.latency_ms.is_some());
        assert_eq!(outcome.message, "Connected to HubSpot account acme.");
    }

    #[tokio::test]
    async fn a_401_a_403_and_a_429_are_each_their_own_fixed_sentence() {
        for (status, needle) in [
            (401, "refused the sign-in"),
            (403, "lacks a permission"),
            (429, "limited the rate"),
            (500, "HTTP 500"),
        ] {
            let (_server, outcome) = answer(status).await;
            assert!(!outcome.ok && outcome.supported, "{status}");
            assert!(
                outcome.message.contains(needle),
                "{status}: {}",
                outcome.message
            );
            assert!(
                !outcome.message.contains("vendor secret text"),
                "the vendor's body must not be echoed: {}",
                outcome.message
            );
        }
    }

    #[tokio::test]
    async fn a_redirect_is_reported_and_never_followed() {
        let target = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&target)
            .await;
        let origin = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/internal", target.uri())),
            )
            .mount(&origin)
            .await;
        let resolver = resolver();
        let auth = SaasAuth::Integration;
        let outcome = probe_saas_at(
            &request(
                "hubspot",
                format!("{}/who", origin.uri()),
                String::new(),
                &auth,
            ),
            &Secrets {
                primary: "env:SAAS_TEST_TOKEN",
                secondary: None,
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings::default(),
        )
        .await;
        assert!(!outcome.ok && outcome.supported);
        assert!(outcome.message.contains("redirect"), "{}", outcome.message);
        assert!(
            !outcome
                .message
                .contains(&target.address().port().to_string()),
            "{}",
            outcome.message
        );
        assert!(target.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_internal_address_is_refused_before_any_credential_is_read() {
        let auth = SaasAuth::Integration;
        let outcome = probe_saas_at(
            &request(
                "hubspot",
                "http://127.0.0.1:9/who".to_owned(),
                String::new(),
                &auth,
            ),
            &Secrets {
                primary: "env:UNUSED",
                secondary: None,
                resolver: &PanicIfCalledResolver,
            },
            &InternalHosts::NONE,
            OAuthSettings::default(),
        )
        .await;
        assert!(!outcome.ok);
        assert!(outcome.latency_ms.is_none());
    }

    #[tokio::test]
    async fn a_sign_in_connector_nobody_has_signed_in_to_is_not_testable_not_failed() {
        let server = MockServer::start().await;
        let resolver = resolver();
        let auth = SaasAuth::Oauth2AuthorizationCode {
            client_id: "app".to_owned(),
        };
        let outcome = probe_saas_at(
            &request(
                "zendesk",
                format!("{}/who", server.uri()),
                String::new(),
                &auth,
            ),
            &Secrets {
                primary: "env:SAAS_TEST_SECRET",
                secondary: None,
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings::default(),
        )
        .await;
        assert!(!outcome.ok && !outcome.supported);
        assert!(outcome.message.contains("Nobody has signed in"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refresh_token_is_exchanged_and_its_replacement_handed_back() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-1", "refresh_token": "refresh-2"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/who"))
            .and(header("authorization", "Bearer access-1"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut map = HashMap::new();
        map.insert("S".to_owned(), "client-secret".to_owned());
        map.insert("R".to_owned(), "refresh-1".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let auth = SaasAuth::Oauth2AuthorizationCode {
            client_id: "app".to_owned(),
        };
        let outcome = probe_saas_at(
            &request(
                "zendesk",
                format!("{}/who", server.uri()),
                format!("{}/token", server.uri()),
                &auth,
            ),
            &Secrets {
                primary: "env:S",
                secondary: Some("env:R"),
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings { allow_http: true },
        )
        .await;
        assert!(outcome.ok, "{}", outcome.message);
        assert_eq!(
            outcome.rotated_refresh_token.unwrap().expose_secret(),
            "refresh-2"
        );
    }

    /// `SRC-3` AD8: a vendor that takes the client in a Basic header (`NetSuite`)
    /// is tested with exactly that, and the body carries no client secret.
    #[tokio::test]
    async fn a_basic_client_vendor_is_tested_with_the_client_in_the_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(header("authorization", "Basic YXBwOmNsaWVudC1zZWNyZXQ="))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-1"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/who"))
            .and(header("authorization", "Bearer access-1"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut map = HashMap::new();
        map.insert("S".to_owned(), "client-secret".to_owned());
        map.insert("R".to_owned(), "refresh-1".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let auth = SaasAuth::Oauth2AuthorizationCode {
            client_id: "app".to_owned(),
        };
        let outcome = probe_saas_at(
            &request(
                "netsuite",
                format!("{}/who", server.uri()),
                format!("{}/token", server.uri()),
                &auth,
            ),
            &Secrets {
                primary: "env:S",
                secondary: Some("env:R"),
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings { allow_http: true },
        )
        .await;
        assert!(outcome.ok, "{}", outcome.message);
        let body = String::from_utf8_lossy(&server.received_requests().await.unwrap()[0].body)
            .into_owned();
        assert!(!body.contains("client_secret"), "{body}");
    }

    #[tokio::test]
    async fn zendesks_email_and_api_token_are_sent_as_basic_with_email_slash_token() {
        // `ZD_EMAIL`/`ZD_TOKEN` are placeholders; base64 of `agent@example.invalid/token:zd-token`.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/who"))
            .and(header(
                "authorization",
                "Basic YWdlbnRAZXhhbXBsZS5pbnZhbGlkL3Rva2VuOnpkLXRva2Vu",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let mut map = HashMap::new();
        map.insert("ZD_EMAIL".to_owned(), "agent@example.invalid".to_owned());
        map.insert("ZD_TOKEN".to_owned(), "zd-token".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let auth = SaasAuth::Integration;
        let outcome = probe_saas_at(
            &request(
                "zendesk",
                format!("{}/who", server.uri()),
                String::new(),
                &auth,
            ),
            &Secrets {
                primary: "env:ZD_EMAIL",
                secondary: Some("env:ZD_TOKEN"),
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings::default(),
        )
        .await;
        assert!(outcome.ok && outcome.supported, "{}", outcome.message);
        assert!(!outcome.message.contains("zd-token"));
    }

    /// `SRC-3` slice 4: Atlassian's email + API token is Basic with the plain email as the user
    /// (base64 of `agent@example.invalid:cf-token`).
    #[tokio::test]
    async fn confluences_email_and_api_token_are_sent_as_basic_with_the_plain_email() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/who"))
            .and(header(
                "authorization",
                "Basic YWdlbnRAZXhhbXBsZS5pbnZhbGlkOmNmLXRva2Vu",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let mut map = HashMap::new();
        map.insert("CF_EMAIL".to_owned(), "agent@example.invalid".to_owned());
        map.insert("CF_TOKEN".to_owned(), "cf-token".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let auth = SaasAuth::Integration;
        let outcome = probe_saas_at(
            &request(
                "confluence",
                format!("{}/who", server.uri()),
                String::new(),
                &auth,
            ),
            &Secrets {
                primary: "env:CF_EMAIL",
                secondary: Some("env:CF_TOKEN"),
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings::default(),
        )
        .await;
        assert!(outcome.ok && outcome.supported, "{}", outcome.message);
        assert!(!outcome.message.contains("cf-token"));
    }

    /// `SRC-3` slice 4: ServiceNow's client credentials go to the token address in the form body
    /// and the access token it returns is the bearer of the who-am-i request.
    #[tokio::test]
    async fn servicenows_client_credentials_are_exchanged_for_a_bearer_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-1", "expires_in": 1800
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/who"))
            .and(header("authorization", "Bearer access-1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let mut map = HashMap::new();
        map.insert("SN_ID".to_owned(), "client".to_owned());
        map.insert("SN_SECRET".to_owned(), "sn-secret".to_owned());
        let resolver = EnvSecretResolver::with_map(map);
        let auth = SaasAuth::Integration;
        let outcome = probe_saas_at(
            &request(
                "servicenow",
                format!("{}/who", server.uri()),
                format!("{}/token", server.uri()),
                &auth,
            ),
            &Secrets {
                primary: "env:SN_ID",
                secondary: Some("env:SN_SECRET"),
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings { allow_http: true },
        )
        .await;
        assert!(outcome.ok && outcome.supported, "{}", outcome.message);
        let body = String::from_utf8_lossy(&server.received_requests().await.unwrap()[0].body)
            .into_owned();
        assert!(body.contains("grant_type=client_credentials"), "{body}");
        assert!(!outcome.message.contains("sn-secret"));
    }

    /// `SRC-3` slice 4: Atlassian's browser sign-in is not testable here, and nothing is sent.
    #[tokio::test]
    async fn confluences_browser_sign_in_is_not_testable_and_sends_nothing() {
        let server = MockServer::start().await;
        let resolver = resolver();
        let auth = SaasAuth::Oauth2AuthorizationCode {
            client_id: "app".to_owned(),
        };
        let outcome = probe_saas_at(
            &request(
                "confluence",
                format!("{}/who", server.uri()),
                String::new(),
                &auth,
            ),
            &Secrets {
                primary: "env:SAAS_TEST_SECRET",
                secondary: None,
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings::default(),
        )
        .await;
        assert!(!outcome.ok && !outcome.supported);
        assert!(outcome.message.contains("gateway address"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_integration_credential_that_is_not_one_token_is_not_testable_yet() {
        let auth = SaasAuth::Integration;
        let outcome = probe_saas_at(
            &request(
                "workday",
                "http://127.0.0.1:9/who".to_owned(),
                String::new(),
                &auth,
            ),
            &Secrets {
                primary: "env:UNUSED",
                secondary: None,
                resolver: &PanicIfCalledResolver,
            },
            &InternalHosts::ALL,
            OAuthSettings::default(),
        )
        .await;
        assert!(!outcome.ok && !outcome.supported);
        assert!(outcome.message.contains("integration credential"));
    }

    #[tokio::test]
    async fn a_vendor_without_a_who_am_i_request_is_not_testable_and_reads_no_secret() {
        let info = ConnectorDialInfo {
            kind: "Workday Reports".to_owned(),
            host: String::new(),
            secret_ref: "env:UNUSED".to_owned(),
            secret_ref_secondary: None,
            adapter: Some("saas".to_owned()),
            dial: serde_json::json!({
                "vendor": "workday",
                "account": "wd2.workday.com/acme",
                "auth": {"type": "integration"}
            }),
        };
        let outcome = probe_saas_info(
            &info,
            &PanicIfCalledResolver,
            &InternalHosts::NONE,
            OAuthSettings::default(),
        )
        .await;
        assert!(!outcome.ok && !outcome.supported);
        assert!(outcome.latency_ms.is_none());
    }

    #[tokio::test]
    async fn a_dial_the_table_refuses_is_misconfigured_not_dialled() {
        let info = ConnectorDialInfo {
            kind: "Zendesk".to_owned(),
            host: String::new(),
            secret_ref: "env:UNUSED".to_owned(),
            secret_ref_secondary: None,
            adapter: Some("saas".to_owned()),
            dial: serde_json::json!({
                "vendor": "zendesk",
                "account": "evil.example.invalid",
                "auth": {"type": "integration"}
            }),
        };
        let outcome = probe_saas_info(
            &info,
            &PanicIfCalledResolver,
            &InternalHosts::ALL,
            OAuthSettings::default(),
        )
        .await;
        assert!(!outcome.ok);
        assert!(outcome.supported);
        assert!(outcome.latency_ms.is_none());
    }

    /// Slice 3 review BLOCKER 10: Google's service-account key is signed into an RS256
    /// assertion (checked here with the public half), exchanged for a bearer token, and
    /// that token reaches the property.
    #[tokio::test]
    async fn a_service_account_key_is_exchanged_for_a_token_with_a_signed_assertion() {
        use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey, LineEnding};

        let mut rng = rsa::rand_core::OsRng;
        let private = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let private_pem = private.to_pkcs1_pem(LineEnding::LF).unwrap();
        let public_pem = private
            .to_public_key()
            .to_pkcs1_pem(LineEnding::LF)
            .unwrap();
        let key_file = serde_json::json!({
            "type": "service_account",
            "client_email": "reader@example.invalid",
            "private_key": private_pem.as_str(),
            "token_uri": "https://evil.example.invalid/token"
        })
        .to_string();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "sa-access", "expires_in": 3599, "token_type": "Bearer"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/who"))
            .and(header("authorization", "Bearer sa-access"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut map = HashMap::new();
        map.insert("KEY".to_owned(), key_file);
        let resolver = EnvSecretResolver::with_map(map);
        let auth = SaasAuth::Integration;
        let outcome = probe_saas_at(
            &request(
                "ga4",
                format!("{}/who", server.uri()),
                format!("{}/token", server.uri()),
                &auth,
            ),
            &Secrets {
                primary: "env:KEY",
                secondary: None,
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings { allow_http: true },
        )
        .await;
        assert!(outcome.ok && outcome.supported, "{}", outcome.message);
        let received = server.received_requests().await.unwrap();
        let body = String::from_utf8(received[0].body.clone()).unwrap();
        let form: HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(
            form["grant_type"],
            "urn:ietf:params:oauth:grant-type:jwt-bearer"
        );
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
        validation.set_audience(&["https://oauth2.googleapis.com/token"]);
        let decoded = jsonwebtoken::decode::<serde_json::Value>(
            &form["assertion"],
            &jsonwebtoken::DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap(),
            &validation,
        )
        .expect("the assertion verifies against the public half");
        assert_eq!(decoded.claims["iss"], "reader@example.invalid");
        assert_eq!(
            decoded.claims["scope"],
            "https://www.googleapis.com/auth/analytics.readonly"
        );
        let span =
            decoded.claims["exp"].as_u64().unwrap() - decoded.claims["iat"].as_u64().unwrap();
        assert!(span <= 3600, "Google: at most one hour after iat");
        assert!(
            !outcome.message.contains("evil.example"),
            "{}",
            outcome.message
        );
    }

    /// A key file that is not one is a fixed message that quotes nothing, and no request goes out.
    #[tokio::test]
    async fn a_service_account_key_that_is_not_one_is_a_fixed_misconfiguration() {
        let server = MockServer::start().await;
        let mut map = HashMap::new();
        map.insert(
            "KEY".to_owned(),
            "not a key file, secret-looking-text".to_owned(),
        );
        let resolver = EnvSecretResolver::with_map(map);
        let auth = SaasAuth::Integration;
        let outcome = probe_saas_at(
            &request(
                "ga4",
                format!("{}/who", server.uri()),
                format!("{}/token", server.uri()),
                &auth,
            ),
            &Secrets {
                primary: "env:KEY",
                secondary: None,
                resolver: &resolver,
            },
            &InternalHosts::ALL,
            OAuthSettings { allow_http: true },
        )
        .await;
        assert!(!outcome.ok, "{}", outcome.message);
        assert!(outcome.message.contains("not a valid key file"));
        assert!(!outcome.message.contains("secret-looking-text"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
