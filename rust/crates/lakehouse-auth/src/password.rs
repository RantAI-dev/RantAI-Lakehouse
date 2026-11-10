//! Local password authentication: hashing, verification, and the
//! `provider = 'local'` rows in `auth_identity`.
//!
//! # Non-enumeration
//!
//! [`verify`] never lets a caller distinguish "no such email" from "wrong
//! password" — both paths return [`AuthError::InvalidCredentials`] and,
//! critically, both paths run a full `Argon2id` verification: an unknown
//! email verifies against [`DUMMY_HASH`] (a fixed, real `Argon2id` hash of
//! an arbitrary constant) instead of returning early. Returning early on
//! "no such user" would make the response latency itself an oracle — a
//! network observer could time login attempts and learn which email
//! addresses have accounts purely from the elapsed time of the `Argon2`
//! step being skipped. Always doing the hash makes both branches do
//! comparable work.
//!
//! # `must_change_password`
//!
//! `auth_identity.must_change_password` exists so a bootstrapped account
//! (an initial admin created outside any signup flow) cannot stay on its
//! bootstrap credential forever. [`create_local_identity`] takes the flag
//! explicitly at creation time; [`must_change_password`] lets a caller
//! check it after a successful [`verify`]; [`change_password`] always
//! clears it. Nothing in [`verify`] itself refuses a login because the flag
//! is set — that policy decision (allow the login but force a redirect to
//! "change your password", vs. refuse the login outright) belongs to
//! whichever later task wires this into the router, not to the
//! authentication primitive.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use async_trait::async_trait;
use rand::RngExt;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::authenticator::Authenticator;
use crate::credential::Credential;
use crate::error::AuthError;
use crate::principal::Principal;
use crate::repository::{self, PgPool};
use crate::secret::Secret;

/// A real, valid `Argon2id` `PHC` hash of a fixed, non-secret constant.
/// Used only as the comparison target when no `app_user` matches the
/// presented email, so that branch pays the same `Argon2` cost a real
/// verification would — see the module doc comment. This value carries no
/// security weight of its own (the password it hashes is public, right
/// here in the source); its only job is to be a well-formed hash the
/// `Argon2` verifier will spend real time computing against.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$ZHVtbXlkdW1teWR1bW15ZHU$ULriu52Oq084VHtqRsKmm3fk5d5irs4oEOfP2zcDMsY";

/// Hash `password` with `Argon2id` at the crate's chosen parameters
/// ([`Argon2::default`], which is the OWASP-recommended minimum: 19 MiB
/// memory, 2 iterations, 1 degree of parallelism).
///
/// `password-hash` 0.6 removed the explicit `SaltString`-from-`OsRng` call
/// this used to make — `PasswordHasher::hash_password` now generates its
/// own random salt internally via the `getrandom` feature (default-on for
/// `argon2`), which is still OS-backed CSPRNG randomness, just no longer a
/// caller-visible step.
///
/// # Errors
///
/// Returns [`AuthError::Hash`] if hashing fails (in practice this only
/// happens for pathological inputs `Argon2` itself rejects, e.g. an
/// extremely long password).
pub fn hash_password(password: &Secret) -> Result<Secret, AuthError> {
    let hash = Argon2::default()
        .hash_password(password.expose().as_bytes())
        .map_err(|_| AuthError::Hash)?;
    Ok(Secret::new(hash.to_string()))
}

/// Verify `password` against a stored `Argon2id` `PHC` hash. Constant-time
/// by construction — `Argon2`'s own verifier does the comparison, and
/// [`verify`] (the caller that matters for non-enumeration) always calls
/// this exactly once per attempt regardless of whether the identity was
/// found, per the module doc comment.
///
/// `pub` since `BI-19` part A: a public link's password is checked with the
/// same `Argon2id` verifier and the same hash format as a login (plan D4, AGENTS
/// rule 4: one helper, not a second one in `lakehouse-api`).
#[must_use]
pub fn verify_hash(password: &Secret, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.expose().as_bytes(), &parsed)
        .is_ok()
}

/// The alphabet a generated temporary password is drawn from.
///
/// # Why these characters and not others
///
/// No LOOK-ALIKE characters: `0`/`O`, `1`/`l`/`I` and `L`/`l` are all
/// excluded, because a temporary password is read off one screen and
/// typed into another by a human, and `0`-for-`O` is exactly the mistake
/// that turns an invite into a support ticket. The digits are `2`-`9` and
/// the letters are the rest of the alphabet, so every character in the
/// password is distinguishable from every other at a glance.
///
/// The three symbols (`-`, `_`, `.`) are unambiguous and safe to paste
/// into any field, so the password never needs quoting on the way in.
///
/// 59 characters in total; 24 of them drawn from this alphabet is about
/// 142 bits of entropy (`log2(59) * 24`), far past anything a human
/// chooses and far past any online guessing rate.
const TEMPORARY_PASSWORD_ALPHABET: &[u8] =
    b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKMNPQRSTUVWXYZ23456789-_.";

/// How many characters a generated temporary password has. The plan's
/// floor is 20; this is 24, comfortably above it.
const TEMPORARY_PASSWORD_LENGTH: usize = 24;

/// Generate a fresh temporary password, from the OS random source.
///
/// ADM-3 D1: an invite (and an admin reset) hands the human a password
/// the SERVER chose, so the caller never has to invent one and never
/// picks a weak one. It is drawn from [`rand::rng`] — a `ChaCha20`-based
/// CSPRNG seeded from the operating system and reseeded as it runs (see
/// `rand`'s `rngs::thread` module) — through [`rand::RngExt::random_range`],
/// which samples without modulo bias.
///
/// The alphabet carries no look-alike characters (see
/// [`TEMPORARY_PASSWORD_ALPHABET`]), and the length is
/// [`TEMPORARY_PASSWORD_LENGTH`].
///
/// # The value is a secret
///
/// The returned [`Secret`] never renders itself (`Debug`/`Display` are a
/// fixed redaction marker), so it cannot reach a log line by accident.
/// The caller shows it once, in the one response that carries it, and
/// forgets it — the server keeps only the `Argon2id` hash.
#[must_use]
pub fn generate_temporary_password() -> Secret {
    let mut rng = rand::rng();
    let mut out = String::with_capacity(TEMPORARY_PASSWORD_LENGTH);
    for _ in 0..TEMPORARY_PASSWORD_LENGTH {
        let index = rng.random_range(0..TEMPORARY_PASSWORD_ALPHABET.len());
        // The alphabet is ASCII by construction, so the byte is a char.
        out.push(char::from(TEMPORARY_PASSWORD_ALPHABET[index]));
    }
    Secret::new(out)
}

/// Create a `provider = 'local'` identity for `app_user_id`, storing a
/// hash of `password` (never the plaintext). `external_subject` is the
/// user's own id, as text — a local login has no external subject of its
/// own (see `0019_auth.sql`'s doc comment).
///
/// # Errors
///
/// Returns [`AuthError::Conflict`] if `app_user_id` already has a `local`
/// identity, [`AuthError::Hash`] if hashing fails, or
/// [`AuthError::Database`] on any other failure.
pub async fn create_local_identity(
    pool: &PgPool,
    app_user_id: Uuid,
    password: &Secret,
    must_change_password: bool,
) -> Result<(), AuthError> {
    let mut tx = pool.begin().await?;
    create_local_identity_in_tx(&mut tx, app_user_id, password, must_change_password).await?;
    tx.commit().await?;
    Ok(())
}

/// [`create_local_identity`], inside a transaction the CALLER owns.
///
/// ADM-3 D1: an invite writes the user, its memberships AND its
/// credential as ONE unit of work, so a user can never be left behind
/// without the credential the response promised. The store's
/// `app_user` insert and this `auth_identity` insert therefore have to
/// share one transaction, which means the caller opens it and passes it
/// in — this crate owns the credential table, the store owns the user
/// table, and neither one can open a transaction the other joins.
///
/// Dropping `tx` without committing rolls the whole thing back (sqlx's
/// `Transaction` starts a rollback on drop), so an error here leaves no
/// half-created account.
///
/// # Errors
///
/// Returns [`AuthError::Conflict`] if `app_user_id` already has a `local`
/// identity, [`AuthError::Hash`] if hashing fails, or
/// [`AuthError::Database`] on any other failure.
pub async fn create_local_identity_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_user_id: Uuid,
    password: &Secret,
    must_change_password: bool,
) -> Result<(), AuthError> {
    let hash = hash_password(password)?;
    sqlx::query(
        "INSERT INTO auth_identity (provider, external_subject, app_user_id, password_hash, must_change_password) \
         VALUES ('local', $1, $2, $3, $4)",
    )
    .bind(app_user_id.to_string())
    .bind(app_user_id)
    .bind(hash.expose())
    .bind(must_change_password)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Replace `app_user_id`'s `local` credential with a new temporary
/// password and flag it `must_change_password`, inside a transaction the
/// CALLER owns.
///
/// ADM-3 D2/D7: a reset SETS a password rather than only replacing one —
/// an account that signs in through an identity provider and is given a
/// password here has no `local` `auth_identity` row yet, and gets one.
/// The flag is set to `true` unconditionally: a reset hands out a
/// server-chosen password, so the holder must replace it at first
/// sign-in exactly like an invited user must.
///
/// # Errors
///
/// Returns [`AuthError::Hash`] if hashing fails, or
/// [`AuthError::Database`] on any other failure.
pub async fn set_temporary_password_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_user_id: Uuid,
    password: &Secret,
) -> Result<(), AuthError> {
    let hash = hash_password(password)?;
    let affected = sqlx::query(
        "UPDATE auth_identity SET password_hash = $1, must_change_password = true, \
         updated_at = now() WHERE app_user_id = $2 AND provider = 'local'",
    )
    .bind(hash.expose())
    .bind(app_user_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if affected > 0 {
        return Ok(());
    }
    // No `local` identity yet — give the account one (ADM-3 D7).
    sqlx::query(
        "INSERT INTO auth_identity (provider, external_subject, app_user_id, password_hash, must_change_password) \
         VALUES ('local', $1, $2, $3, true)",
    )
    .bind(app_user_id.to_string())
    .bind(app_user_id)
    .bind(hash.expose())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Replace the stored password hash for `app_user_id`'s `local` identity
/// and clear `must_change_password`.
///
/// # Errors
///
/// Returns [`AuthError::NotFound`] if `app_user_id` has no `local`
/// identity, [`AuthError::Hash`] if hashing fails, or
/// [`AuthError::Database`] on any other failure.
pub async fn change_password(
    pool: &PgPool,
    app_user_id: Uuid,
    new_password: &Secret,
) -> Result<(), AuthError> {
    let hash = hash_password(new_password)?;
    let affected = sqlx::query(
        "UPDATE auth_identity SET password_hash = $1, must_change_password = false, \
         updated_at = now() WHERE app_user_id = $2 AND provider = 'local'",
    )
    .bind(hash.expose())
    .bind(app_user_id)
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(AuthError::NotFound);
    }
    Ok(())
}

/// Whether `app_user_id`'s `local` identity is still flagged
/// `must_change_password`.
///
/// # Errors
///
/// Returns [`AuthError::NotFound`] if `app_user_id` has no `local`
/// identity, or [`AuthError::Database`] on any other failure.
pub async fn must_change_password(pool: &PgPool, app_user_id: Uuid) -> Result<bool, AuthError> {
    let row: Option<(bool,)> = sqlx::query_as(
        "SELECT must_change_password FROM auth_identity \
         WHERE app_user_id = $1 AND provider = 'local'",
    )
    .bind(app_user_id)
    .fetch_optional(pool)
    .await?;
    row.map(|(flag,)| flag).ok_or(AuthError::NotFound)
}

// ADM-11 review 1 S6: fixed doc comments (removed orphan doc above MIN_PASSWORD_LENGTH).
/// Minimum length for a user password (ADM-11 D4).
pub const MIN_PASSWORD_LENGTH: usize = 8;
/// Maximum length for a user password (ADM-11 D4).
pub const MAX_PASSWORD_LENGTH: usize = 1024;

/// Errors produced by password rule validation (ADM-11 D4).
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PasswordRuleError {
    /// Password has fewer than [`MIN_PASSWORD_LENGTH`] characters.
    #[error("Password must be at least 8 characters")]
    TooShort,
    /// Password has more than [`MAX_PASSWORD_LENGTH`] characters.
    #[error("Password must be at most 1024 characters")]
    TooLong,
    /// Password is equal to the username (case-insensitive).
    #[error("Password cannot be equal to the username")]
    EqualsUsername,
    /// Password is equal to the email address (case-insensitive).
    #[error("Password cannot be equal to the email address")]
    EqualsEmail,
}

/// Check `password` against the system password rules (ADM-11 D4).
///
/// Rules:
/// - At least 8 characters (counted as Unicode scalar values, `chars()`).
/// - At most 1024 characters.
/// - Not equal (case-insensitive) to `username` (if provided).
/// - Not equal (case-insensitive) to `email` (if provided).
///
/// # Errors
///
/// Returns [`PasswordRuleError`] naming the broken rule.
pub fn check_rules(
    password: &str,
    username: Option<&str>,
    email: Option<&str>,
) -> Result<(), PasswordRuleError> {
    let char_count = password.chars().count();
    if char_count < MIN_PASSWORD_LENGTH {
        return Err(PasswordRuleError::TooShort);
    }
    if char_count > MAX_PASSWORD_LENGTH {
        return Err(PasswordRuleError::TooLong);
    }
    if let Some(u) = username {
        let u_trimmed = u.trim();
        if !u_trimmed.is_empty() && password.eq_ignore_ascii_case(u_trimmed) {
            return Err(PasswordRuleError::EqualsUsername);
        }
    }
    if let Some(e) = email {
        let e_trimmed = e.trim();
        if !e_trimmed.is_empty() && password.eq_ignore_ascii_case(e_trimmed) {
            return Err(PasswordRuleError::EqualsEmail);
        }
    }
    Ok(())
}

/// Verify `identifier` (username or email, case-insensitive) and `password`
/// against `auth_identity`.
///
/// ADM-11 D6: sign-in accepts either identifier in any letter case.
///
/// # Errors
///
/// Returns [`AuthError::InvalidCredentials`] if `identifier` names no user,
/// the user has no `local` identity, matches multiple users, or `password`
/// does not match — these cases are indistinguishable to the caller by design.
/// Returns [`AuthError::Database`] on any other failure.
pub async fn verify(
    pool: &PgPool,
    identifier: &str,
    password: &Secret,
) -> Result<Principal, AuthError> {
    let rows: Vec<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT ai.app_user_id, ai.password_hash, ai.must_change_password FROM auth_identity ai \
         JOIN app_user u ON u.id = ai.app_user_id \
         WHERE (lower(u.username) = lower($1) OR lower(u.email) = lower($1)) \
           AND ai.provider = 'local'",
    )
    .bind(identifier)
    .fetch_all(pool)
    .await?;

    if rows.len() != 1 {
        // D6: with 0 rows, check against DUMMY_HASH as today. With 2 or more
        // (ambiguous identifier), also check against DUMMY_HASH (same timing) and refuse.
        let _ = verify_hash(password, DUMMY_HASH);
        if rows.len() >= 2 {
            let user_ids: Vec<Uuid> = rows.iter().map(|(id, _, _)| *id).collect();
            tracing::warn!(
                ?user_ids,
                "ambiguous login identifier matched multiple users"
            );
        }
        return Err(AuthError::InvalidCredentials);
    }

    let (app_user_id, stored_hash, must_change_password) = &rows[0];

    if !verify_hash(password, stored_hash) {
        return Err(AuthError::InvalidCredentials);
    }

    repository::load_principal_for_user(
        pool,
        *app_user_id,
        "local".to_owned(),
        *must_change_password,
    )
    .await
    .map_err(|_| AuthError::InvalidCredentials)
}

/// Verify `password` for `user_id` without an identifier lookup.
///
/// ADM-11 D6: used by `change_password` so an account changing its password
/// does not need to verify by email or username, which would fail for an
/// account with no email or require looking up its username first.
///
/// # Errors
///
/// Returns [`AuthError::InvalidCredentials`] if `user_id` has no `local`
/// identity or `password` does not match, or [`AuthError::Database`] on
/// failure.
pub async fn verify_for_user(
    pool: &PgPool,
    user_id: Uuid,
    password: &Secret,
) -> Result<(), AuthError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT password_hash FROM auth_identity \
         WHERE app_user_id = $1 AND provider = 'local'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let Some((stored_hash,)) = row else {
        let _ = verify_hash(password, DUMMY_HASH);
        return Err(AuthError::InvalidCredentials);
    };

    if !verify_hash(password, &stored_hash) {
        return Err(AuthError::InvalidCredentials);
    }

    Ok(())
}

/// [`Authenticator`] for [`Credential::Password`].
pub struct LocalPasswordAuthenticator {
    pool: PgPool,
}

impl LocalPasswordAuthenticator {
    /// Build an authenticator backed by `pool`.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Authenticator for LocalPasswordAuthenticator {
    // The trait returns `&str` tied to `&self`, not `&'static str`, because a
    // future `OidcAuthenticator` needs to return an owned, per-instance
    // provider label (e.g. "oidc:okta" built from config) -- see
    // `Authenticator`'s doc comment. This impl happens to return a literal.
    #[allow(clippy::unnecessary_literal_bound)]
    fn provider_id(&self) -> &str {
        "local"
    }

    async fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError> {
        let Credential::Password {
            identifier,
            password,
        } = credential
        else {
            return Err(AuthError::UnsupportedCredential);
        };
        verify(&self.pool, identifier, password).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_freshly_hashed_password_verifies_against_its_own_hash() {
        let password = Secret::new("correct horse battery staple");
        let hash = hash_password(&password).unwrap();
        assert!(verify_hash(&password, hash.expose()));
    }

    #[test]
    fn the_wrong_password_does_not_verify() {
        let password = Secret::new("correct horse battery staple");
        let hash = hash_password(&password).unwrap();
        assert!(!verify_hash(&Secret::new("wrong password"), hash.expose()));
    }

    #[test]
    fn a_malformed_stored_hash_fails_closed_rather_than_panicking() {
        assert!(!verify_hash(
            &Secret::new("anything"),
            "not-a-valid-phc-hash"
        ));
    }

    #[test]
    fn hashing_the_same_password_twice_produces_different_hashes() {
        // Distinct random salts per call — this is what actually stops a
        // rainbow-table / equal-hash-implies-equal-password attack.
        let password = Secret::new("correct horse battery staple");
        let first = hash_password(&password).unwrap();
        let second = hash_password(&password).unwrap();
        assert_ne!(first.expose(), second.expose());
        assert!(verify_hash(&password, first.expose()));
        assert!(verify_hash(&password, second.expose()));
    }

    #[test]
    fn the_dummy_hash_constant_is_a_well_formed_argon2_phc_string() {
        // If this ever fails to parse, the non-enumeration guarantee in
        // `verify` silently degrades to "unknown user fails fast" — pin the
        // constant down explicitly rather than relying on `verify_hash`
        // returning `false` for both a bad hash and a bad password to hide
        // the difference.
        assert!(PasswordHash::new(DUMMY_HASH).is_ok());
    }

    /// A real `Argon2id` `PHC` hash of `PRE_BUMP_PASSWORD`, generated with
    /// argon2 0.5.3 (the version this crate used before the #21 dependency
    /// bump to 0.6). Pinned here so the bump is proven not to have changed
    /// what a stored hash from before it verifies against — a password
    /// hashing library bump that silently stops verifying pre-existing
    /// rows would lock out every user who hasn't since changed their
    /// password.
    const PRE_BUMP_PASSWORD: &str = "pinned-regression-test-password-v0.5";
    const PRE_BUMP_ARGON2_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$php75+C3qAii2F/vDtY4nw$64yTZpHPjDVICcfrr0H/AJh/D9e0Yw82Ch8AE3XRvYA";

    #[test]
    fn a_hash_from_before_the_argon2_0_6_bump_still_verifies() {
        assert!(verify_hash(
            &Secret::new(PRE_BUMP_PASSWORD),
            PRE_BUMP_ARGON2_HASH
        ));
        // And the non-enumeration guarantee still holds: a wrong password
        // against that same pre-existing hash still fails.
        assert!(!verify_hash(&Secret::new("wrong"), PRE_BUMP_ARGON2_HASH));
    }

    #[test]
    fn hash_never_appears_in_a_debug_render_of_the_secret_wrapping_it() {
        let password = Secret::new("correct horse battery staple");
        let hash = hash_password(&password).unwrap();
        let rendered = format!("{hash:?}");
        assert!(!rendered.contains("argon2id"));
    }

    /// ADM-3 D1: a temporary password is at least 20 characters, and is
    /// drawn only from the alphabet the module documents.
    #[test]
    fn a_generated_temporary_password_is_long_enough_and_uses_the_documented_alphabet() {
        let password = generate_temporary_password();
        let value = password.expose();
        assert!(
            value.chars().count() >= 20,
            "a temporary password must be at least 20 characters, got {}",
            value.chars().count()
        );
        for character in value.chars() {
            assert!(
                TEMPORARY_PASSWORD_ALPHABET.contains(&(character as u8)),
                "the temporary password alphabet must not carry {character:?}"
            );
        }
    }

    /// ADM-3 D1: no look-alike characters — the whole point of the
    /// alphabet. `0`/`O`, `1`/`l`/`I` and `L`/`l` are the pairs a human
    /// mistypes when reading a password off one screen.
    #[test]
    fn a_generated_temporary_password_carries_no_look_alike_characters() {
        for _ in 0..64 {
            let value = generate_temporary_password();
            for forbidden in ['0', 'O', '1', 'l', 'I', 'L'] {
                assert!(
                    !value.expose().contains(forbidden),
                    "a temporary password must not carry {forbidden:?}"
                );
            }
        }
    }

    /// Two generated passwords must never be the same — the whole value of
    /// the generator is that it is random.
    #[test]
    fn two_generated_temporary_passwords_differ() {
        let first = generate_temporary_password();
        let second = generate_temporary_password();
        assert_ne!(first.expose(), second.expose());
    }

    /// ADM-11 review 1 S6 / ADM-11 D4: `check_rules` verifies password length
    /// and ensures password is not equal to username or email.
    #[test]
    fn password_rules_checks_length_and_equality() {
        assert_eq!(
            check_rules("short", Some("username"), Some("user@example.com")),
            Err(PasswordRuleError::TooShort)
        );
        assert_eq!(
            check_rules("1234567", Some("username"), Some("user@example.com")),
            Err(PasswordRuleError::TooShort)
        );
        assert_eq!(
            check_rules("12345678", Some("username"), Some("user@example.com")),
            Ok(())
        );
        let long_pass = "a".repeat(1025);
        assert_eq!(
            check_rules(&long_pass, Some("username"), Some("user@example.com")),
            Err(PasswordRuleError::TooLong)
        );
        assert_eq!(
            check_rules("Username", Some("username"), Some("user@example.com")),
            Err(PasswordRuleError::EqualsUsername)
        );
        assert_eq!(
            check_rules(
                "USER@EXAMPLE.COM",
                Some("username"),
                Some("user@example.com")
            ),
            Err(PasswordRuleError::EqualsEmail)
        );
        assert_eq!(check_rules("valid-pass-123", None, None), Ok(()));
    }

    /// The generated password is a [`Secret`]: it cannot be logged by
    /// accident, which is half of ADM-3 D1's "never logged" promise (ADM-11 review 1 S6).
    #[test]
    fn a_generated_temporary_password_never_renders_itself() {
        let password = generate_temporary_password();
        let rendered = format!("{password:?} {password}");
        assert_eq!(rendered, "Secret(REDACTED) REDACTED");
        assert!(!rendered.contains(password.expose()));
    }
}
