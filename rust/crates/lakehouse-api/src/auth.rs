//! Task 3.2: wiring `lakehouse-auth`'s `Principal`/`Authenticator` primitives
//! into this axum router.
//!
//! # The one rule every handler is judged against
//!
//! No handler, and nothing in this module either, branches on *how* a
//! caller authenticated (cookie vs. bearer token vs., later, an OIDC id
//! token) — only on the resulting [`lakehouse_auth::Principal`] and its
//! `permissions`. [`AuthenticatedPrincipal`] is the one seam a handler (or
//! `crate::policy`'s gate) ever touches.
//!
//! # Cookie vs. bearer, and why both try every authenticator shape they can
//!
//! A browser session presents [`SESSION_COOKIE_NAME`] and is checked
//! against [`lakehouse_auth::SessionAuthenticator`] via
//! [`lakehouse_auth::Credential::SessionToken`]. A service caller presents
//! `Authorization: Bearer <token>`; today that can only mean a
//! [`lakehouse_auth::Credential::ServiceToken`]
//! ([`lakehouse_auth::ServiceTokenAuthenticator`] is the only authenticator
//! this crate wires that accepts a bearer-shaped credential), but this
//! extractor also tries the same raw token as
//! [`lakehouse_auth::Credential::Bearer`] — unused today, but exactly the
//! shape a future `OIDC` authenticator (Task 3.5) will accept. Adding that
//! authenticator to [`crate::state::AuthState`] and this module's bearer
//! loop is the ONLY change Task 3.5 needs here; no handler changes.

use axum::extract::FromRequestParts;
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use lakehouse_auth::{Authenticator, Credential, Principal, Secret};
use lakehouse_core::ApiError;

use crate::error::ApiRejection;
use crate::state::AppState;

/// The name of the cookie a browser session is carried in.
pub const SESSION_COOKIE_NAME: &str = "lh_session";

/// The normalized caller of an authenticated request, extracted from
/// either the session cookie or an `Authorization: Bearer` header. See the
/// module doc comment for why a handler never needs to know which.
#[derive(Debug, Clone)]
pub struct AuthenticatedPrincipal(pub Principal);

impl FromRequestParts<AppState> for AuthenticatedPrincipal {
    type Rejection = ApiRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(auth) = state.auth.as_ref() else {
            return Err(ApiError::Unavailable(
                "authentication unavailable: no Postgres pool is configured (set DATABASE_URL)"
                    .to_owned(),
            )
            .into());
        };

        if let Some(token) = session_cookie(parts) {
            let credential = Credential::SessionToken(Secret::new(token));
            if let Ok(principal) = auth.session.authenticate(&credential).await {
                return Ok(Self(principal));
            }
            // A present-but-invalid/expired/revoked cookie does not fall
            // through to the bearer path — a browser sending a stale
            // cookie is not also sending a service token, and treating an
            // explicitly-rejected credential as "try the next kind" would
            // blur a revoked-session signal into an ordinary
            // no-credential-presented one in logs. It still ends in the
            // same 401 either way.
            return Err(unauthenticated());
        }

        if let Some(token) = bearer_token(parts) {
            // Try every bearer-shaped credential this crate's configured
            // authenticators can consume, in order — see the module doc
            // comment. Only `ServiceToken` matches anything today.
            for credential in [
                Credential::ServiceToken(Secret::new(token.clone())),
                Credential::Bearer(Secret::new(token)),
            ] {
                if let Ok(principal) = auth.service.authenticate(&credential).await {
                    return Ok(Self(principal));
                }
            }
            return Err(unauthenticated());
        }

        Err(unauthenticated())
    }
}

/// A structured, internals-free 401 — never leaks which of "no credential",
/// "bad credential", or "expired credential" occurred, matching
/// `lakehouse_auth::error`'s own non-enumeration stance.
fn unauthenticated() -> ApiRejection {
    ApiError::unauthorized().into()
}

/// Read [`SESSION_COOKIE_NAME`]'s value out of the `Cookie` request header.
/// No cookie-parsing crate: the request-side `Cookie` header is always a
/// flat `name=value; name2=value2` list (RFC 6265 §5.4), which a manual
/// split handles completely — pulling in a crate for this one line is not
/// worth the dependency.
fn session_cookie(parts: &Parts) -> Option<String> {
    session_cookie_from_headers(&parts.headers)
}

/// Same lookup as [`session_cookie`], taking a bare [`axum::http::HeaderMap`]
/// so `routes::auth` can reuse it (`logout` needs the raw token to revoke,
/// which [`Principal`] deliberately never carries).
pub(crate) fn session_cookie_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    let header = headers.get(COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == SESSION_COOKIE_NAME).then(|| value.to_owned())
    })
}

/// Read the raw token out of an `Authorization: Bearer <token>` header.
fn bearer_token(parts: &Parts) -> Option<String> {
    let header = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;
    header
        .strip_prefix("Bearer ")
        .map(|token| token.trim().to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use axum::http::HeaderValue;
    use axum::http::header::COOKIE as COOKIE_HEADER;

    use super::*;

    fn parts_with_cookie(value: &str) -> Parts {
        let mut req = axum::http::Request::builder().body(()).unwrap();
        req.headers_mut()
            .insert(COOKIE_HEADER, HeaderValue::from_str(value).unwrap());
        req.into_parts().0
    }

    #[test]
    fn session_cookie_finds_the_named_cookie_among_others() {
        let parts = parts_with_cookie("other=1; lh_session=abc123; third=2");
        assert_eq!(session_cookie(&parts).as_deref(), Some("abc123"));
    }

    #[test]
    fn session_cookie_is_none_when_absent() {
        let parts = parts_with_cookie("other=1; third=2");
        assert_eq!(session_cookie(&parts), None);
    }

    #[test]
    fn bearer_token_strips_the_prefix() {
        let mut req = axum::http::Request::builder().body(()).unwrap();
        req.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str("Bearer tok-123").unwrap(),
        );
        let (parts, ()) = req.into_parts();
        assert_eq!(bearer_token(&parts).as_deref(), Some("tok-123"));
    }
}
