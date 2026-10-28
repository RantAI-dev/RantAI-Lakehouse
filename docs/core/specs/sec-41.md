# `SEC-41` Connector dials can verify a CA: upgrade `sqlx` to 0.9

| | |
| --- | --- |
| Backlog | `SEC-41` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | M |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

`SEC-14` part B (decision E1) dials connector databases with
`PgSslMode::VerifyCa` when a CA is configured. The dial goes to the
approved IP address, not the name (`SEC-15`), so it must check the
certificate chain and skip the name. `sqlx-core` 0.8.6 means to do that,
but its verifier only recognises rustls' `NotValidForName` error
(`src/net/tls/tls_rustls.rs:309`), and rustls 0.23.46 reports
`NotValidForNameContext`. Every such dial fails: with a CA set, table
discovery and change-capture clean-up stop working (CI on PR #192, gates
G6 and SRC-13). `sqlx-core` 0.9.0 recognises both (its line 313;
`sqlx` changelog #3861). There is no 0.8.7.

## Decisions (product owner, 2026-10-17)

| # | Decision |
| --- | --- |
| D1 | Upgrade `sqlx` to 0.9 rather than dial with `Require` (no certificate check) or patch `sqlx`. |
| D2 | **Blocked by `REL-9`**: `sqlx` 0.9 needs Rust 1.94 or later. |
| D3 | The `mysql-rsa` feature stays off. A MySQL connector whose server has no TLS and asks for `caching_sha2_password` or `sha256_password` is refused with a fixed message, not sent the password with RSA. The `rsa` crate (advisory `RUSTSEC-2023-0071`, no fix) leaves the API binary, and the justification in `rust/deny.toml` and `.github/workflows/security.yml`, wrong today (`sqlx-mysql` 0.8.6 already links `rsa`), is corrected. |
| D4 | `sqlx` 0.9 refuses a query string built at run time unless it is wrapped in `AssertSqlSafe`. Each of the 82 sites is wrapped where it is, with a comment saying why it is safe (constant identifiers; values bound). No shared helper hides the reason. |

## Target

| Capability | Today | Target |
| --- | --- | --- |
| `VerifyCa` dial to an IP with a certificate for the name | Fails | Succeeds |
| `VerifyCa` dial with a certificate from another CA | Fails | Fails (unchanged) |
| Databases already migrated by `sqlx` 0.8 | — | Start under 0.9 with no checksum or version error |
| MySQL server with no TLS and a SHA-2 password plugin | Password sent RSA-encrypted | Refused with a fixed message; documented |
| `rsa` in the API binary | Yes | No |

## Acceptance checklist

- `SEC-41-AC1` A test dials through the production connect-options helper with `VerifyCa` to an IP against a certificate that names a host: it passes; the same with a certificate from another CA: it fails.
- `SEC-41-AC2` A test applies the migrations with 0.8's `_sqlx_migrations` rows already in place (the real upgrade path) and the API starts.
- `SEC-41-AC3` `cargo tree -i rsa -e normal` on `lakehouse-api` is empty; `deny.toml` and `security.yml` say why `RUSTSEC-2023-0071` is still ignored, truthfully.
- `SEC-41-AC4` Every `AssertSqlSafe` has a reason beside it; the reviewer checks each `format!` it wraps puts only constant or validated identifiers in the text.
- `SEC-41-AC5` The full verification block and every CI gate pass, including G4, G6 and SRC-13.

## Not included

`SEC-14` part B itself (it merges after this, on top of it). Other crate
upgrades beyond what `sqlx` 0.9 requires in `Cargo.lock`.
