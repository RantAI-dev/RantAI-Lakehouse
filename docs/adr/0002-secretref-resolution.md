# ADR 0002 — `secretRef` resolution

- **Status:** Accepted
- **Phase:** P1
- **Date:** 2026-08-31

## Context

`lakehouse-store::connectors`'s module doc has carried a guarantee since
Phase 2: a connector's `secret_ref: String` names WHERE a credential lives
(an env var name, a secret-manager path), never the credential itself. That
guarantee was load-bearing but incomplete — nothing in the codebase ever
resolved a `secretRef` to a value, because nothing needed to yet.

P1 changes that. Three things now need to turn a `secretRef` into an actual
credential:

1. `lakehouse-iceberg`'s Lakekeeper catalog client, for Lakekeeper's own
   OAuth2 client-credential (when Lakekeeper authorization is enabled).
2. Lakekeeper's own storage-credential (the access key/secret it uses to
   call RustFS's STS `AssumeRole` and vend per-table credentials) —
   Lakekeeper resolves this itself server-side, but the *value* still has
   to originate somewhere in this deployment's configuration.
3. Two future consumers this ADR is written for even though they don't
   exist yet: Debezium Server's source-database credential (P5) and dlt's
   connection secret (P3). Both are named explicitly in the task brief as
   blocked on this ADR landing.

Without a resolver, every one of these either hardcodes a credential
(unacceptable) or invents its own ad hoc resolution scheme (fragmenting the
one guarantee `connectors.rs` already established). Neither is acceptable,
so this ADR designs the resolver once, for all four callers, present and
future.

## Decision

**A trait, [`SecretResolver`], with one implementation today,
[`EnvSecretResolver`]** (both in `lakehouse-core::secret`, the crate every
other workspace crate already depends on and that depends on nothing else
in the workspace — the correct home for a seam this many crates need).

```rust
pub trait SecretResolver: Debug + Send + Sync {
    fn resolve(&self, secret_ref: &str)
        -> impl Future<Output = Result<SecretValue, SecretError>> + Send;
}
```

- **`SecretValue`** wraps the resolved string. It has a hand-written
  `Debug` that always renders `"<redacted>"`, and deliberately has NO
  `Serialize` impl at all (not even a redacting one) — a resolved secret
  handed to `serde_json::to_value` by mistake fails to compile instead of
  needing a human to remember a runtime redaction step. This does not
  weaken `connectors.rs`'s existing guarantee; it extends the same shape
  (hand-written `Debug`, no accidental serialization path) to the one new
  place a credential *value* now legitimately exists in-process.
- **`secretRef` scheme prefixes.** A reference is `<scheme>:<locator>`,
  e.g. `env:LAKEKEEPER_CREDENTIAL`. The scheme is mandatory from day one —
  an unprefixed reference (`"LAKEKEEPER_CREDENTIAL"`) is rejected — so that
  adding a second scheme later (`file:`) is never a breaking change to
  every `secretRef` already stored in the connector registry or Lakekeeper
  config. `EnvSecretResolver` only accepts `env:`; a reference with any
  other scheme is `SecretError::UnsupportedRef`.
- **`DynSecretResolver`**, an object-safe wrapper `#[async_trait]`
  auto-implemented for every `SecretResolver`, for callers that need to
  hold `Arc<dyn DynSecretResolver>` in application state (native
  `async fn`-in-trait is not object-safe; this workspace's MSRV, 1.88,
  supports native async traits but not dyn dispatch over them without this
  wrapper).
- **What ships in P1b:** only `EnvSecretResolver`, resolving `env:VAR_NAME`
  against the process environment (or an injected map, for tests — same
  pattern `lakehouse_api::config::Config::from_map` already uses).

## What later implementations look like, without a breaking change

- **`FileSecretResolver`** (`file:/run/secrets/lakekeeper-credential`):
  reads a file path, for Docker/Kubernetes secret-mount deployments. Same
  trait, same `SecretValue` return type, new `scheme` string. No caller of
  `SecretResolver::resolve` changes.
- **An external-provider resolver** (Vault, AWS Secrets Manager, GCP Secret
  Manager): `vault:secret/data/lakekeeper#credential`-shaped references,
  resolved via a network call — exactly why `resolve` is `async` from day
  one even though `EnvSecretResolver` never awaits anything. Making
  `resolve` synchronous now to match today's one implementation would force
  a breaking signature change the day a network-backed resolver landed;
  that cost is paid once, now, instead of on every future caller.
- **A resolver chain**, if a deployment needs `env:` for some refs and
  `vault:` for others simultaneously: a `ChainedSecretResolver` that
  dispatches on scheme prefix and delegates to the right inner resolver.
  Nothing about the trait shape prevents this; it is out of scope for P1b
  because nothing needs it yet.

## Consequences

- `lakehouse-core::secret` gains `SecretResolver`, `DynSecretResolver`,
  `SecretValue`, `SecretError`, `EnvSecretResolver`, and the
  `ENV_SECRET_REF_PREFIX` constant.
- `lakehouse-api::config::Config` gains three `secretRef`-shaped fields
  (`lakekeeper_credential_secret_ref`, `rustfs_access_key_secret_ref`,
  `rustfs_secret_key_secret_ref`) — references, resolved lazily by whatever
  eventually consumes them, not resolved at config-load time. This mirrors
  `database_url`'s existing "store the string, fail lazily at first use"
  posture rather than `PORT`'s "fail at boot" posture: a secret being
  temporarily unresolvable should not be a harder failure mode than
  Postgres being temporarily unreachable already is.
- **Operator guidance, stated plainly:** `env:`-scheme resolution is the
  least trustworthy option long-term — env vars are visible to the whole
  process, appear in `/proc/<pid>/environ`, and are easy to leak into a
  crash dump or a support bundle. It exists in P1b because it is what
  unblocks the G1 test and Lakekeeper's storage-credential configuration
  today, not because it is the recommended production shape. A production
  deployment handling real customer credentials should move to
  `FileSecretResolver` (secret-mounted files) or an external-provider
  resolver as soon as one exists; this ADR's whole design point is that
  doing so later costs a new trait implementation, not a rewrite.
- This ADR does not itself wire `SecretResolver` into
  `lakehouse-iceberg`'s `IcebergClientConfig` — that struct takes an
  already-resolved `Option<SecretValue>` for `catalog_credential`
  (resolution happens at the call site, e.g. in `lakehouse-api`, before
  constructing the config), keeping `lakehouse-iceberg` from needing to
  depend on a specific resolver implementation at all.

## Verification

`cargo test -p lakehouse-core` — `secret::tests` covers: `Debug` never
renders a resolved value or an injected override map; `env:`-prefixed refs
resolve from an injected map; unprefixed and empty-suffix refs are
rejected; an unset variable is `NotFound`, not a silent empty string; the
`DynSecretResolver` wrapper delegates correctly. `cargo clippy --all-targets
--all-features --locked -- -D warnings` is clean for the new module (see
the P1b report for actual captured output).

## Addendum (P6 hardening): restricting connector secretRefs

**Status:** Accepted
**Date:** 2026-09-07

### Context

This ADR's design was written for callers where the resolved secret's
DESTINATION is fixed and operator-controlled — Lakekeeper's own storage
credential, dialing RustFS; a future Debezium source credential, dialing a
database an operator configured. Under that assumption, "which env var can
this caller name" was never a meaningful attack surface, because the
caller choosing a `secretRef` was the same operator who configured the
destination it would be used against.

P6 (`lakehouse-api::connector_probe`) broke that assumption without this
ADR being revisited at the time. `POST /api/connectors` lets a
`connector:manage` principal set BOTH `host` AND `secretRef` on a
connector, and `POST /api/connectors/{id}/test` then resolves `secretRef`
and dials `host` with it. Handing that code path the unrestricted
`EnvSecretResolver` — resolving ANY `env:NAME` in the process environment
— meant a `connector:manage` principal could set `secretRef:
"env:DATABASE_URL"` (or `CH_PASSWORD`, `OIDC_CLIENT_SECRET`,
`EMBED_SECRET`, `ALERTS_RUN_TOKEN`, `SMTP_PASS`, `LLM_KEY`, ...) and `host`
pointed at infrastructure they control, press "Test", and have this
service authenticate to their host with the resolved secret —
`sqlx`'s Postgres wire protocol negotiates cleartext-password auth by
default, so the value would arrive at the attacker's host in the clear.
This was flagged in PR #34 code review as a Blocker.

### Decision

**A caller whose destination is caller-controlled must be handed a
resolver restricted to an explicit allowlist, never the general-purpose
one.** Concretely:

- `lakehouse_core::secret::AllowlistedSecretResolver<R>` wraps any
  `SecretResolver` with an explicit `HashSet<String>` of permitted
  `secretRef` strings. A reference outside the set is rejected with the
  new `SecretError::NotAllowed` variant — BEFORE the inner resolver is
  ever consulted, never a silent fall-through and never confusable with
  `NotFound` (a correctly-scoped but misspelled reference).
- `lakehouse_api::state::AppState` now carries
  `connector_secret_resolver: Arc<dyn DynSecretResolver>` — always an
  `AllowlistedSecretResolver` wrapping `EnvSecretResolver`, scoped to the
  fixed constant `CONNECTOR_ALLOWED_SECRET_REFS` (`env:POSTGRES_PASSWORD`,
  `env:RUSTFS_ACCESS_KEY`, `env:RUSTFS_SECRET_KEY` — exactly the refs
  `0022_prune_connector_seed.sql`'s two dialable seeded connectors use).
  `connector_probe::probe` is handed this resolver, never the general
  `EnvSecretResolver` any other part of the process might use.
- The allowlist is a hardcoded Rust constant, not derived from
  configuration or a database row: widening it is a deliberate code
  change subject to review, not something a `connector:manage` principal
  (or an operator fat-fingering an env var) can expand at runtime.

This does not change anything about `SecretResolver`/`EnvSecretResolver`
themselves, or any other caller of this ADR's original design — Lakekeeper's
storage-credential resolution, and any future Debezium/dlt caller, are
still free to use the unrestricted resolver, because their destination is
NOT caller-controlled. `AllowlistedSecretResolver` is an opt-in wrapper for
the one shape of caller (today: exactly `connector_probe`) where it is.

### Consequences

- `lakehouse-core::secret` gains `AllowlistedSecretResolver` and
  `SecretError::NotAllowed`.
- `lakehouse-api::state::AppState::secret_resolver` is renamed
  `connector_secret_resolver` and its type's construction changes from
  `Arc::new(EnvSecretResolver::new())` to an `AllowlistedSecretResolver`
  wrapping the same. No other caller existed to migrate.
- **Operator guidance, added to the existing "Consequences" section
  above:** any FUTURE caller of `SecretResolver` where the caller who
  chooses the `secretRef` also controls (or influences) where the
  resolved value is used — a webhook target, a user-supplied endpoint,
  anything not fixed at deploy time by an operator — MUST go through
  `AllowlistedSecretResolver` (or an equivalent explicit scoping
  mechanism), never the bare resolver. This is now the litmus test this
  ADR uses for "does a new caller need scoping": ask whether the entity
  choosing the `secretRef` is the same entity that controls where it's
  used. If yes (Lakekeeper's own config), the bare resolver is fine. If
  no (a caller-registered connector, a caller-registered webhook), it is
  not.

### Verification

`cargo test -p lakehouse-core` — `secret::tests` adds:
`out_of_scope_env_ref_is_rejected_not_silently_resolved` (the exact
Blocker 1 scenario: `env:DATABASE_URL` is on the inner resolver but not
the allowlist, and is refused, not silently resolved),
`allowlisted_ref_still_resolves_through_the_inner_resolver`, and
`empty_allowlist_rejects_everything`. `cargo test -p lakehouse-api` covers
`connector_probe`'s use of the scoped resolver via its existing
credential-resolution tests (unchanged: they already exercise `env:`
lookups through whatever resolver `probe` is handed).
