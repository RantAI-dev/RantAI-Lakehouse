//! Drop a `Postgres` logical-replication slot and publication left behind
//! by a deleted CDC connector — R5's mitigation (`docs/plans/
//! LAKEHOUSE-FOUNDATION-PLAN.md`'s risk register: "a stuck or lagging
//! replication slot pins WAL and fills the customer's production database
//! disk").
//!
//! # This makes `ops/debezium/deprovision_connector.sh`'s comment false
//!
//! That script's header says `DELETE /api/connectors/{id}` doesn't call it
//! "because the Rust API has no mechanism to safely resolve an arbitrary
//! connector's secretRef and dial an arbitrary host from inside the
//! product process". That was true when it was written. It no longer is:
//! [`crate::connector_probe`] does exactly that today (real dial, resolved
//! secretRef, SSRF-checked host) for `POST /api/connectors/{id}/test`. This
//! module is the same capability aimed at deprovisioning instead of
//! testing, and `routes::connectors::delete` calls it directly —
//! `ops/debezium/deprovision_connector.sh` stays as the mechanism
//! `ops/g4/g4_test.py` drives from outside the process (it has no Rust
//! process to call into), but the product's own delete path no longer
//! orphans a slot the way it used to.
//!
//! # Ordering — mirrors the shell script exactly
//!
//! 1. `DROP PUBLICATION IF EXISTS` — safe to do regardless of slot state.
//! 2. `pg_terminate_backend` any backend actively consuming the slot — a
//!    slot cannot be dropped while something is consuming it, so whatever
//!    that consumer is (a live `debezium-server` process, most likely)
//!    must be kicked off it first.
//! 3. Poll briefly for the slot to go inactive, then `pg_drop_replication_slot`.
//!
//! Absent slot/publication is success, not an error — this must be
//! idempotent, since a connector that was never fully provisioned (or was
//! already partially deprovisioned by a previous failed attempt) still
//! needs `DELETE` to succeed. [`Deprovisioned`] distinguishes "was there,
//! now dropped" from "was never there" so a caller that wants to know can.

use std::future::Future;
use std::time::Duration;

use lakehouse_core::secret::SecretValue;
use lakehouse_store::cdc::ConnectorSlug;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgSslMode};

/// Bound on every individual query this module issues (connect, drop
/// publication, terminate backend, poll, drop slot) — a hung/unreachable
/// source database must fail the `DELETE` request quickly, never hang it
/// indefinitely. Matches `connector_probe::DIAL_TIMEOUT`'s reasoning
/// exactly (a few seconds: long enough for a healthy LAN-local Postgres,
/// short enough that a firewalled host resolves fast).
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to keep polling for the replication slot to go inactive after
/// terminating whatever backend was consuming it, before giving up and
/// attempting the drop anyway (which will itself fail loudly if the slot
/// is truly still active). Same 5-attempt/1s-apart shape
/// `deprovision_connector.sh` uses.
const SLOT_INACTIVE_POLL_ATTEMPTS: u32 = 5;
const SLOT_INACTIVE_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Everything [`drop_slot_and_publication`] needs to dial the connector's
/// source `Postgres` database directly — resolved and owned by the caller
/// (`routes::connectors::delete`), never a DSN string this module would
/// have to parse. Every field is bound as an individual
/// [`PgConnectOptions`] setter, never interpolated into a connection
/// string, matching `connector_probe::probe_postgres`'s exact reasoning
/// for why that matters.
pub struct PgTarget {
    /// Source database hostname (already SSRF-checked by the caller if
    /// that matters for this deployment — this module does not repeat
    /// that check, since deprovisioning a connector's own registered host
    /// is not the same trust boundary as a caller-chosen probe target).
    pub host: String,
    /// Source database port.
    pub port: u16,
    /// Source database user. Must have permission to `DROP PUBLICATION`
    /// and `pg_drop_replication_slot` (superuser or the `REPLICATION`
    /// attribute plus ownership, standard Postgres logical-replication
    /// administration requirements — not something this module can relax).
    pub user: String,
    /// Resolved credential for `user`.
    pub password: SecretValue,
    /// Source database name the slot/publication live in.
    pub database: String,
}

/// Whether a `DROP ... IF EXISTS` actually found something to drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropOutcome {
    /// The object existed and was dropped by this call.
    Dropped,
    /// The object did not exist — nothing to do, not an error.
    NotPresent,
}

/// The result of a full deprovision attempt: what happened to the
/// publication and what happened to the slot, reported separately since
/// a partially-deprovisioned connector (e.g. a previous attempt dropped
/// the publication but failed before reaching the slot) is a real state
/// this must handle idempotently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deprovisioned {
    /// What happened to `"<slug>_pub"`.
    pub publication: DropOutcome,
    /// What happened to `"<slug>_slot"`.
    pub slot: DropOutcome,
}

/// Everything that can stop [`drop_slot_and_publication`] from reaching a
/// clean, idempotent end state. Every variant names the slot/publication
/// involved (never a secret) — the underlying `sqlx::Error` is attached as
/// `source` for diagnosability, and `sqlx::Error`'s own `Display` never
/// includes connection credentials (they are never part of a `Postgres`
/// wire-protocol error message), so nothing here needs the redaction
/// [`CdcSpecError`](lakehouse_store::cdc::CdcSpecError) requires for the
/// values `lakehouse_store::cdc` interpolates into a properties file.
#[derive(Debug, thiserror::Error)]
pub enum DeprovisionError {
    /// Could not open a connection to the source database at all.
    #[error("could not connect to the source database to deprovision its CDC connector: {source}")]
    Connect {
        /// The underlying connection failure.
        #[source]
        source: sqlx::Error,
    },
    /// `DROP PUBLICATION` itself failed (not "didn't exist" — that is
    /// handled by `IF EXISTS` and is not an error).
    #[error("dropping publication {publication:?} failed: {source}")]
    DropPublication {
        /// The publication name this attempt was dropping.
        publication: String,
        /// The underlying database error.
        #[source]
        source: sqlx::Error,
    },
    /// Terminating the backend actively consuming the slot failed.
    #[error("terminating the backend holding replication slot {slot:?} failed: {source}")]
    TerminateBackend {
        /// The slot name this attempt was clearing.
        slot: String,
        /// The underlying database error.
        #[source]
        source: sqlx::Error,
    },
    /// Reading `pg_replication_slots` to check slot state failed.
    #[error("checking replication slot {slot:?} state failed: {source}")]
    CheckSlot {
        /// The slot name this attempt was checking.
        slot: String,
        /// The underlying database error.
        #[source]
        source: sqlx::Error,
    },
    /// `pg_drop_replication_slot` itself failed — most commonly because
    /// the slot is still active (a consumer reconnected after the
    /// terminate/poll step above, or never actually disconnected).
    #[error("dropping replication slot {slot:?} failed: {source}")]
    DropSlot {
        /// The slot name this attempt was dropping.
        slot: String,
        /// The underlying database error.
        #[source]
        source: sqlx::Error,
    },
    /// One of the queries this function issues did not complete within
    /// [`QUERY_TIMEOUT`] — a hung/unreachable source database must fail
    /// fast, not hang the `DELETE` request indefinitely.
    #[error("deprovisioning {label} timed out after {timeout_secs}s")]
    Timeout {
        /// What was being deprovisioned when the timeout hit — the
        /// slot/publication name(s) involved, not a connector slug: since
        /// WS3 plan review X4, a `cdc`-adapter connector's names come from
        /// its `dial`, not from `ConnectorSlug`-deriving its `id`.
        label: String,
        /// The timeout that was exceeded, in seconds.
        timeout_secs: u64,
    },
    /// `slot_name`/`publication_name` failed [`ConnectorSlug`]'s
    /// identifier-safety check before any DDL was attempted. This is a
    /// data-integrity problem, not a database failure: a `dial.slotName`/
    /// `publicationName` value (or a slug-derived legacy name) that isn't
    /// shaped like a safe SQL identifier must never reach the
    /// `DROP PUBLICATION`/`pg_drop_replication_slot` calls below, which
    /// inline it directly (see [`drop_publication`]'s doc comment for why
    /// that inlining is safe only when the value has already been checked
    /// this way).
    #[error("{field} {value:?} is not a valid identifier: {source}")]
    InvalidName {
        /// Which name was rejected (`"slot_name"` or `"publication_name"`).
        field: &'static str,
        /// The rejected value, as given — never a credential, safe to
        /// include (same reasoning as
        /// [`CdcSpecError::InvalidConnectorSlug`](lakehouse_store::cdc::CdcSpecError::InvalidConnectorSlug)).
        value: String,
        /// The underlying validation failure.
        #[source]
        source: lakehouse_store::cdc::CdcSpecError,
    },
}

/// Bound a query future by [`QUERY_TIMEOUT`], collapsing "timed out" and
/// the query's own error into [`DeprovisionError`] via `on_timeout`/`on_err`
/// so every call site stays a single `.await?`-shaped line rather than
/// repeating the same `match` five times. `label` names whatever is being
/// deprovisioned, for [`DeprovisionError::Timeout`]'s message only.
async fn with_timeout<T, E>(
    label: &str,
    future: impl Future<Output = Result<T, E>>,
    on_err: impl FnOnce(E) -> DeprovisionError,
) -> Result<T, DeprovisionError> {
    match tokio::time::timeout(QUERY_TIMEOUT, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(on_err(err)),
        Err(_) => Err(DeprovisionError::Timeout {
            label: label.to_owned(),
            timeout_secs: QUERY_TIMEOUT.as_secs(),
        }),
    }
}

/// Drop the replication slot named `slot_name` and the publication named
/// `publication_name` on the source database dialed via `target`.
/// Idempotent: dropping an already-absent slot/publication is success, not
/// an error — see the module doc comment's ordering section for why each
/// step happens in this order.
///
/// `slot_name`/`publication_name` are taken directly rather than derived
/// from a [`ConnectorSlug`], because a `cdc`-adapter connector's names come
/// from its own `dial.slotName`/`dial.publicationName` (WS3 plan review
/// X4) — they no longer always equal `"{slug}_slot"`/`"{slug}_pub"`. The
/// guarantee this module still gives is that whatever name reaches a
/// `DROP`/`pg_drop_replication_slot` call is always shaped like a safe SQL
/// identifier: [`drop_publication`] and [`drop_slot`] each validate their
/// name via [`ConnectorSlug::new`] before using it, so the property that
/// used to be "always `{slug}_pub`" is now "always validated-identifier-
/// shaped" instead.
///
/// # Errors
///
/// See [`DeprovisionError`]'s variants. Every query is bounded by
/// [`QUERY_TIMEOUT`], so a hung source database surfaces as
/// [`DeprovisionError::Timeout`] rather than hanging the caller.
pub async fn drop_slot_and_publication(
    target: &PgTarget,
    slot_name: &str,
    publication_name: &str,
) -> Result<Deprovisioned, DeprovisionError> {
    let options = PgConnectOptions::new()
        .host(&target.host)
        .port(target.port)
        .username(&target.user)
        .password(target.password.expose_secret())
        .database(&target.database)
        // Matches `connector_probe::probe_postgres`'s exact posture: this
        // deployment's compose-network Postgres does not terminate TLS, so
        // `Prefer` (attempt TLS, fall back to plaintext) is the correct
        // default rather than leaving it to sqlx's own default.
        .ssl_mode(PgSslMode::Prefer);

    let connect_label = format!("slot {slot_name:?} / publication {publication_name:?}");
    let mut conn = with_timeout(
        &connect_label,
        PgConnection::connect_with(&options),
        |source| DeprovisionError::Connect { source },
    )
    .await?;

    let publication = drop_publication(&mut conn, publication_name).await?;
    let slot = drop_slot(&mut conn, slot_name).await?;

    Ok(Deprovisioned { publication, slot })
}

/// `DROP PUBLICATION IF EXISTS "<publication_name>"` — the publication
/// name is inlined directly into the SQL text, not bound as a `$1`
/// parameter, because Postgres' DDL grammar does not accept a bind
/// parameter in place of an identifier (`DROP PUBLICATION $1` is a syntax
/// error, not a safer version of this statement). Inlining an identifier
/// is safe ONLY because `publication_name` is validated against
/// [`ConnectorSlug`]'s shape (`^[a-z0-9][a-z0-9_]{0,62}$`) at the top of
/// this function, before anything below touches it — this is no longer
/// "always `{slug}_pub`" (WS3 plan review X4: a `cdc`-adapter connector's
/// publication name comes from its own `dial.publicationName`, which need
/// not match its `id`-derived slug at all), so the safety property this
/// function now guarantees is "always validated-identifier-shaped", not
/// "always slug-derived".
///
/// Reports [`DropOutcome::NotPresent`] when the publication did not exist
/// before this call, distinguishing that from actually having dropped one.
///
/// # Errors
///
/// Returns [`DeprovisionError::InvalidName`] if `publication_name` is not
/// shaped like a valid [`ConnectorSlug`]. See [`DeprovisionError`]'s other
/// variants for the database-level failures.
async fn drop_publication(
    conn: &mut PgConnection,
    publication_name: &str,
) -> Result<DropOutcome, DeprovisionError> {
    ConnectorSlug::new(publication_name).map_err(|source| DeprovisionError::InvalidName {
        field: "publication_name",
        value: publication_name.to_owned(),
        source,
    })?;
    let existed: Option<(i32,)> = with_timeout(
        publication_name,
        sqlx::query_as("SELECT 1 FROM pg_publication WHERE pubname = $1")
            .bind(publication_name)
            .fetch_optional(&mut *conn),
        |source| DeprovisionError::DropPublication {
            publication: publication_name.to_owned(),
            source,
        },
    )
    .await?;

    let drop_sql = format!("DROP PUBLICATION IF EXISTS \"{publication_name}\"");
    with_timeout(
        publication_name,
        sqlx::query(&drop_sql).execute(&mut *conn),
        |source| DeprovisionError::DropPublication {
            publication: publication_name.to_owned(),
            source,
        },
    )
    .await?;

    Ok(if existed.is_some() {
        DropOutcome::Dropped
    } else {
        DropOutcome::NotPresent
    })
}

/// Terminate any backend actively consuming `slot_name`, poll briefly for
/// it to go inactive, then drop it if it still exists. Every identifier
/// here is bound as a `$1` VALUE (`pg_replication_slots.slot_name`,
/// `pg_terminate_backend`'s pid argument, `pg_drop_replication_slot`'s name
/// argument all accept a bind parameter — none of these are DDL identifier
/// positions), so injection is not the risk `slot_name` is validated
/// against below. It is still validated via [`ConnectorSlug::new`] anyway
/// (WS3 plan review X4): `slot_name` now comes straight from a `cdc`-
/// adapter connector's `dial.slotName`, and rejecting anything not shaped
/// like a safe identifier up front keeps that value held to the same bar
/// everywhere this crate accepts a slot/publication name, rather than
/// depending on this one query-parameter-binding fact never changing.
///
/// # Errors
///
/// Returns [`DeprovisionError::InvalidName`] if `slot_name` is not shaped
/// like a valid [`ConnectorSlug`]. See [`DeprovisionError`]'s other
/// variants for the database-level failures.
async fn drop_slot(
    conn: &mut PgConnection,
    slot_name: &str,
) -> Result<DropOutcome, DeprovisionError> {
    ConnectorSlug::new(slot_name).map_err(|source| DeprovisionError::InvalidName {
        field: "slot_name",
        value: slot_name.to_owned(),
        source,
    })?;
    // A slot must be inactive before it can be dropped — if some process
    // (most likely a live `debezium-server`) is still consuming it,
    // terminate that backend's replication connection first. Absent slot
    // means this simply affects zero rows, which is fine.
    with_timeout(
        slot_name,
        sqlx::query(
            "SELECT pg_terminate_backend(active_pid) FROM pg_replication_slots \
             WHERE slot_name = $1 AND active_pid IS NOT NULL",
        )
        .bind(slot_name)
        .execute(&mut *conn),
        |source| DeprovisionError::TerminateBackend {
            slot: slot_name.to_owned(),
            source,
        },
    )
    .await?;

    // Give Postgres a moment to mark the slot inactive after the terminate
    // above, same 5-attempt/1s-apart shape as
    // `ops/debezium/deprovision_connector.sh`.
    for _ in 0..SLOT_INACTIVE_POLL_ATTEMPTS {
        let active: Option<(bool,)> = with_timeout(
            slot_name,
            sqlx::query_as("SELECT active FROM pg_replication_slots WHERE slot_name = $1")
                .bind(slot_name)
                .fetch_optional(&mut *conn),
            |source| DeprovisionError::CheckSlot {
                slot: slot_name.to_owned(),
                source,
            },
        )
        .await?;
        match active {
            Some((true,)) => tokio::time::sleep(SLOT_INACTIVE_POLL_INTERVAL).await,
            _ => break,
        }
    }

    let exists: Option<(bool,)> = with_timeout(
        slot_name,
        sqlx::query_as("SELECT active FROM pg_replication_slots WHERE slot_name = $1")
            .bind(slot_name)
            .fetch_optional(&mut *conn),
        |source| DeprovisionError::CheckSlot {
            slot: slot_name.to_owned(),
            source,
        },
    )
    .await?;

    let Some(_) = exists else {
        return Ok(DropOutcome::NotPresent);
    };

    with_timeout(
        slot_name,
        sqlx::query("SELECT pg_drop_replication_slot($1)")
            .bind(slot_name)
            .execute(&mut *conn),
        |source| DeprovisionError::DropSlot {
            slot: slot_name.to_owned(),
            source,
        },
    )
    .await?;

    Ok(DropOutcome::Dropped)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_test_support as _;
    use sqlx::PgPool;

    use super::*;

    /// Build a [`PgTarget`] dialing the SAME per-test Postgres database
    /// `#[sqlx::test]` already handed us via `pool` — extracting
    /// host/port/user/database from the pool's own connect options rather
    /// than hardcoding them, so this keeps working if
    /// `lakehouse-test-support` ever changes its fixed superuser
    /// credentials.
    fn target_for(pool: &PgPool) -> PgTarget {
        let options = pool.connect_options();
        PgTarget {
            host: options.get_host().to_owned(),
            port: options.get_port(),
            user: options.get_username().to_owned(),
            // `lakehouse-test-support` always starts the container with
            // the official Postgres image's default superuser password.
            password: SecretValue::new("postgres"),
            database: options
                .get_database()
                .expect("#[sqlx::test] always targets a named database")
                .to_owned(),
        }
    }

    #[sqlx::test(migrations = false)]
    async fn absent_slot_and_publication_is_success_not_an_error(pool: PgPool) -> sqlx::Result<()> {
        let target = target_for(&pool);
        let result = drop_slot_and_publication(
            &target,
            "nope_never_provisioned_slot",
            "nope_never_provisioned_pub",
        )
        .await
        .unwrap();
        assert_eq!(result.publication, DropOutcome::NotPresent);
        assert_eq!(result.slot, DropOutcome::NotPresent);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn existing_publication_is_dropped_and_reported_as_dropped(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        sqlx::query("CREATE PUBLICATION \"realpub_test_pub\" FOR ALL TABLES")
            .execute(&pool)
            .await?;

        let target = target_for(&pool);
        let result = drop_slot_and_publication(&target, "realpub_test_slot", "realpub_test_pub")
            .await
            .unwrap();
        assert_eq!(result.publication, DropOutcome::Dropped);
        assert_eq!(result.slot, DropOutcome::NotPresent);

        let still_there: Option<(i32,)> =
            sqlx::query_as("SELECT 1 FROM pg_publication WHERE pubname = 'realpub_test_pub'")
                .fetch_optional(&pool)
                .await?;
        assert!(still_there.is_none(), "publication must actually be gone");

        // Idempotent: calling again on an already-clean connector must
        // still succeed and report NotPresent, not fail.
        let target = target_for(&pool);
        let second = drop_slot_and_publication(&target, "realpub_test_slot", "realpub_test_pub")
            .await
            .unwrap();
        assert_eq!(second.publication, DropOutcome::NotPresent);
        Ok(())
    }

    /// A `slot_name`/`publication_name` that is not shaped like a valid
    /// [`ConnectorSlug`] (e.g. carries a double quote, which would let it
    /// escape the `DROP PUBLICATION IF EXISTS "<name>"` identifier
    /// quoting) must be rejected before any query runs, not merely produce
    /// a Postgres-side syntax error — the whole point of WS3 plan review
    /// X4's "always validated-identifier-shaped" guarantee.
    #[sqlx::test(migrations = false)]
    async fn invalid_publication_name_is_rejected_before_any_query(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let target = target_for(&pool);
        let err = drop_slot_and_publication(&target, "safe_slot", "not a valid slug\"; --")
            .await
            .expect_err("a non-slug-shaped publication name must be rejected");
        assert!(
            matches!(err, DeprovisionError::InvalidName { field, .. } if field == "publication_name")
        );
        Ok(())
    }

    #[tokio::test]
    async fn unreachable_host_surfaces_as_connect_error_not_a_hang() {
        let target = PgTarget {
            host: "127.0.0.1".to_owned(),
            port: 1,
            user: "postgres".to_owned(),
            password: SecretValue::new("postgres"),
            database: "postgres".to_owned(),
        };
        let started = std::time::Instant::now();
        let err =
            drop_slot_and_publication(&target, "unreachable_test_slot", "unreachable_test_pub")
                .await
                .expect_err("nothing listens on port 1");
        assert!(started.elapsed() < QUERY_TIMEOUT + Duration::from_secs(2));
        assert!(matches!(
            err,
            DeprovisionError::Connect { .. } | DeprovisionError::Timeout { .. }
        ));
    }
}
