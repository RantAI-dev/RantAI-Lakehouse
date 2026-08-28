//! Task 3.1: the authentication core.
//!
//! # Why this crate exists
//!
//! Before this task, `lakehouse-api` authenticated nobody: every route,
//! including writes to Postgres-backed storage and the real
//! `service_identity`/`app_user` directory, was open. This crate builds
//! the primitive that a later task (3.2) wires into the router — it does
//! NOT touch `lakehouse-api` itself, and defines no route.
//!
//! # The one requirement every decision here is judged against
//!
//! Connecting a real identity provider later (Okta, Google, Entra,
//! Keycloak, generic `OIDC`/`SAML`) must not require rewriting a handler or
//! reshaping the schema. Two things make that true:
//!
//! 1. [`Principal`] — every [`Authenticator`], no matter how it verified
//!    the caller, produces this same normalized shape. A handler that
//!    reads `principal.has("catalog:write")` cannot tell, and never needs
//!    to ask, whether the caller typed a password, presented a session
//!    cookie, or arrived via an `OIDC` id token.
//! 2. `auth_identity` (`../../migrations/0019_auth.sql`) — a local
//!    password is not special-cased on `app_user`, it is one row with
//!    `provider = 'local'`. Adding Okta means inserting rows with
//!    `provider = 'oidc:okta'` into the SAME table. No migration.
//!
//! See [`Authenticator`]'s doc comment for the concrete, file-by-file
//! answer to "what does adding Okta require".
//!
//! # What is (and is NOT) implemented here
//!
//! Three [`Authenticator`]s: [`password::LocalPasswordAuthenticator`],
//! [`session::SessionAuthenticator`], and
//! [`service_token::ServiceTokenAuthenticator`]. `OIDC` is deliberately
//! absent — that is Task 3.5, and its clean arrival on top of the seam
//! built here is the proof the seam works. [`credential::Credential`]
//! already reserves a [`credential::Credential::Bearer`] variant for it.

pub mod authenticator;
pub mod credential;
pub mod error;
pub mod password;
pub mod permissions;
pub mod principal;
pub mod repository;
pub mod secret;
pub mod service_token;
pub mod session;
mod token;

pub use authenticator::Authenticator;
pub use credential::Credential;
pub use error::AuthError;
pub use password::LocalPasswordAuthenticator;
pub use permissions::PermissionSet;
pub use principal::{Principal, PrincipalId};
pub use repository::PgPool;
pub use secret::Secret;
pub use service_token::ServiceTokenAuthenticator;
pub use session::SessionAuthenticator;
