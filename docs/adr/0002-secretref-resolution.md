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
