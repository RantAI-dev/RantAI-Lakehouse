# `SEC-30`: update `rustls` past RUSTSEC-2026-0346 to 0349

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `e2663378`. Branch: `fix/sec-30-rustls`. One PR slice.

## 1. The problem

`cargo audit` and `cargo deny` are red on `main` since 2026-10-14: the lock
file pins `rustls` 0.23.45, which has four advisories published that day.

| ID | Title (as RustSec states it) |
| --- | --- |
| RUSTSEC-2026-0346 | `Debug` impls for client session values expose TLS secrets |
| RUSTSEC-2026-0347 | Buffered application data sent to outer server after ECH rejection |
| RUSTSEC-2026-0348 | Brotli certificate decompression can allocate up to 1 GiB before authentication |
| RUSTSEC-2026-0349 | Unauthenticated ServerHello can downgrade later connections to classical key exchange |

The fix RustSec names: `rustls` >= 0.23.46. Until it lands, no PR that
touches dependencies may merge (`AGENTS.md` step 6).

## 2. Decisions

- **D1.** A lock-file update only: `cargo update -p rustls` (or
  `--precise 0.23.46` or the newest 0.23.x). No `Cargo.toml` change unless a
  crate pins `rustls` below 0.23.46; if one does, raise only that floor and
  say why in the commit.
- **D2.** No other crate moves. If `cargo update -p rustls` drags others
  along, list every crate and version that changed in the handoff
  (`git diff rust/Cargo.lock`), and stop and report if anything beyond
  `rustls` and its own `rustls-*` siblings changes a minor version.
- **D3.** No advisory is ignored or waived; `deny.toml` and the
  `cargo audit --ignore` list in `.github/workflows/security.yml` are not
  touched.

## 3. Tasks

- **T1.** The update, one commit "SEC-30 T1: update rustls to <version>
  (RUSTSEC-2026-0346 to 0349)". Check: `cargo tree -i rustls` shows only the
  new version; `cargo fmt --check`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo test --workspace --no-fail-fast`.
  `cargo audit` / `cargo deny` are not installed on this machine; CI is
  their check, and the handoff says so.
- **T2.** `CHANGELOG.md` entry a customer can read.

## 4. Out of scope

`quinn-proto` (`SEC-27`), the `gitleaks` findings (`SEC-13`), every
Dependabot PR.

## 5. Handoff

### `fix/sec-30-rustls` (developer, 2026-10-14)

#### Commits

- `9e00101` — `SEC-30 docs(plan): update rustls past RUSTSEC-2026-0346 to 0349` (plan commit)
- `f4d92d1` — `SEC-30 T1: update rustls to 0.23.46 (RUSTSEC-2026-0346 to 0349)`
- `da97bdf` — `SEC-30 T2: changelog entry for rustls security update`

#### `git diff origin/main -- rust/Cargo.lock` summary

- `rustls`: `0.23.45` -> `0.23.46` (patch update resolving RUSTSEC-2026-0346 to 0349)
- `tempfile` 3.27.0: `getrandom` dependency pointer re-locked to `getrandom 0.3.4` (from `0.4.3`); both `getrandom 0.3.4` and `0.4.3` were already present in `Cargo.lock` and neither crate version itself was bumped or changed.
- No other crate moved; no minor version changes beyond rustls.

#### `cargo tree -i rustls`

```text
rustls v0.23.46
├── bollard v0.20.2
│   └── testcontainers v0.27.3
│       ├── lakehouse-test-support v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-test-support)
│       │   [dev-dependencies]
│       │   ├── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│       │   ├── lakehouse-auth v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-auth)
│       │   │   └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│       │   └── lakehouse-store v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-store)
│       │       ├── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│       │       └── lakehouse-auth v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-auth) (*)
│       └── testcontainers-modules v0.15.0
│           └── lakehouse-test-support v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-test-support) (*)
│           [dev-dependencies]
│           └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│       [dev-dependencies]
│       └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
├── hyper-rustls v0.27.9
│   ├── bollard v0.20.2 (*)
│   ├── reqwest v0.12.28
│   │   ├── iceberg v0.10.1
│   │   │   ├── iceberg-catalog-rest v0.10.1
│   │   │   │   └── lakehouse-iceberg v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-iceberg)
│   │   │   │       └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   │   ├── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   │   └── lakehouse-iceberg v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-iceberg) (*)
│   │   ├── iceberg-catalog-rest v0.10.1 (*)
│   │   ├── lakehouse-alerts v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-alerts)
│   │   │   └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   ├── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   ├── lakehouse-auth v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-auth) (*)
│   │   ├── lakehouse-clickhouse v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-clickhouse)
│   │   │   ├── lakehouse-alerts v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-alerts) (*)
│   │   │   ├── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   │   ├── lakehouse-bi v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-bi)
│   │   │   │   ├── lakehouse-alerts v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-alerts) (*)
│   │   │   │   └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   │   └── lakehouse-embed v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-embed)
│   │   │       └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   ├── lakehouse-dagster v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-dagster)
│   │   │   └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   ├── lakehouse-llm v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-llm)
│   │   │   └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   ├── lakehouse-notify v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-notify)
│   │   │   ├── lakehouse-alerts v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-alerts) (*)
│   │   │   └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   └── lakehouse-trino v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-trino)
│   │       └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   [dev-dependencies]
│   │   └── lakehouse-iceberg v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-iceberg) (*)
│   └── reqwest v0.13.4
│       ├── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│       ├── object_store v0.14.2
│       │   ├── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│       │   └── lakehouse-iceberg v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-iceberg) (*)
│       └── testcontainers v0.27.3 (*)
├── lettre v0.11.23
│   └── lakehouse-notify v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-notify) (*)
├── reqwest v0.12.28 (*)
├── reqwest v0.13.4 (*)
├── rustls-platform-verifier v0.7.0
│   └── reqwest v0.13.4 (*)
├── sqlx-core v0.8.6
│   ├── sqlx v0.8.6
│   │   ├── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
│   │   ├── lakehouse-auth v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-auth) (*)
│   │   └── lakehouse-store v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-store) (*)
│   ├── sqlx-macros v0.8.6 (proc-macro)
│   │   └── sqlx v0.8.6 (*)
│   ├── sqlx-macros-core v0.8.6
│   │   └── sqlx-macros v0.8.6 (proc-macro) (*)
│   ├── sqlx-mysql v0.8.6
│   │   └── sqlx v0.8.6 (*)
│   ├── sqlx-mysql v0.8.6
│   │   └── sqlx-macros-core v0.8.6 (*)
│   ├── sqlx-postgres v0.8.6
│   │   └── sqlx v0.8.6 (*)
│   └── sqlx-postgres v0.8.6
│       └── sqlx-macros-core v0.8.6 (*)
├── tokio-rustls v0.26.4
│   ├── hyper-rustls v0.27.9 (*)
│   ├── lettre v0.11.23 (*)
│   ├── reqwest v0.12.28 (*)
│   ├── reqwest v0.13.4 (*)
│   └── tiberius v0.13.0
│       └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt-sec-30/rust/crates/lakehouse-api)
└── ureq v3.4.0
    └── bollard-buildkit-proto v0.7.0
        ├── bollard v0.20.2 (*)
        └── bollard-stubs v1.52.1-rc.29.1.3
            └── bollard v0.20.2 (*)
```

#### Verification

- `cargo fmt --check`: passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed with 0 warnings.
- `cargo test --workspace --no-fail-fast`: 97 suites, all `ok`: **4742 passed, 0 failed, 8 ignored**.
- cargo audit / cargo deny: not installed here, CI is the check.

Exact suites:

- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_alerts-f2de269bcf01ea83)`: test result: ok. 86 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_api-87c39f3cf1c6dae4)`: test result: ok. 1646 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 125.53s
- `Running unittests src/main.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_api-263e8357e554da03)`: test result: ok. 1692 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 127.68s
- `Running tests/agents_approval.rs (/home/shiro/rantai/cargo-target/debug/deps/agents_approval-01f624447210a79f)`: test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.66s
- `Running tests/agents_run.rs (/home/shiro/rantai/cargo-target/debug/deps/agents_run-932c7aed1018d3a0)`: test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.55s
- `Running tests/ai_ask_back_turn.rs (/home/shiro/rantai/cargo-target/debug/deps/ai_ask_back_turn-e3d94ec3e8896dfc)`: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.50s
- `Running tests/ai_chat_stream.rs (/home/shiro/rantai/cargo-target/debug/deps/ai_chat_stream-877d84241ddf40b5)`: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.38s
- `Running tests/ai_citations.rs (/home/shiro/rantai/cargo-target/debug/deps/ai_citations-11812e222c1c242d)`: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.37s
- `Running tests/ai_sql_help.rs (/home/shiro/rantai/cargo-target/debug/deps/ai_sql_help-d0a1ce34a3ee7eff)`: test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.10s
- `Running tests/ai_terms.rs (/home/shiro/rantai/cargo-target/debug/deps/ai_terms-a98ccb2ba80a9c5c)`: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.91s
- `Running tests/ai_tool.rs (/home/shiro/rantai/cargo-target/debug/deps/ai_tool-624d6c52471a7b77)`: test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.46s
- `Running tests/connector_delete_deprovision.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_delete_deprovision-d8d64a4f37379dac)`: test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.59s
- `Running tests/connector_not_testable.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_not_testable-2b18f43c60cefd65)`: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s
- `Running tests/connector_probe_history_route.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_probe_history_route-ff571c25e89dfe9c)`: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.25s
- `Running tests/connector_repoint.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_repoint-b93d392fec2ab38f)`: test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.59s
- `Running tests/connector_secret_rotate.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_secret_rotate-bcf94c987e7e0e88)`: test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.42s
- `Running tests/connector_tenant_access.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_tenant_access-b6ab6024597b5cfc)`: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.37s
- `Running tests/connector_upload_table.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_upload_table-221aa5b96a3d2e1e)`: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.50s
- `Running tests/debezium_properties.rs (/home/shiro/rantai/cargo-target/debug/deps/debezium_properties-89a9e4115085e950)`: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s
- `Running tests/gold_export_auth.rs (/home/shiro/rantai/cargo-target/debug/deps/gold_export_auth-768895daa7bbd2ed)`: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.15s
- `Running tests/gold_publication.rs (/home/shiro/rantai/cargo-target/debug/deps/gold_publication-4ca69ffb90ace3dd)`: test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.59s
- `Running tests/home_layout.rs (/home/shiro/rantai/cargo-target/debug/deps/home_layout-39205dd09b623bbb)`: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.38s
- `Running tests/identity_user_tenants.rs (/home/shiro/rantai/cargo-target/debug/deps/identity_user_tenants-91e76c3501f07eb7)`: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.44s
- `Running tests/ingest_run.rs (/home/shiro/rantai/cargo-target/debug/deps/ingest_run-8ca1575cd7571c18)`: test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.06s
- `Running tests/lakehouse_maintenance.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_maintenance-ce9f1da27e53ac64)`: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.31s
- `Running tests/login_throttle.rs (/home/shiro/rantai/cargo-target/debug/deps/login_throttle-29a7e57dacf4f7da)`: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.01s
- `Running tests/parity.rs (/home/shiro/rantai/cargo-target/debug/deps/parity-aad162a90f646682)`: test result: ok. 0 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Running tests/pipeline_routing.rs (/home/shiro/rantai/cargo-target/debug/deps/pipeline_routing-46c3bf726fb3d003)`: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.21s
- `Running tests/query_download.rs (/home/shiro/rantai/cargo-target/debug/deps/query_download-35a4ac3909590ddf)`: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.34s
- `Running tests/route_auth.rs (/home/shiro/rantai/cargo-target/debug/deps/route_auth-46cbfb0a3403900a)`: test result: ok. 31 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.75s
- `Running tests/security_regressions.rs (/home/shiro/rantai/cargo-target/debug/deps/security_regressions-5f0006839834f9ce)`: test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.94s
- `Running tests/semantic_api.rs (/home/shiro/rantai/cargo-target/debug/deps/semantic_api-b008150f538e3779)`: test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.03s
- `Running tests/tenant_provisioning.rs (/home/shiro/rantai/cargo-target/debug/deps/tenant_provisioning-4b0cd86b82c9f9f0)`: test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.62s
- `Running tests/test_connection_route.rs (/home/shiro/rantai/cargo-target/debug/deps/test_connection_route-68bc0b1ed95a2cb5)`: test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s
- `Running tests/tier1_saved_query_guard.rs (/home/shiro/rantai/cargo-target/debug/deps/tier1_saved_query_guard-d02eeacb461c8a47)`: test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.26s
- `Running tests/tier1_writehigh_approval.rs (/home/shiro/rantai/cargo-target/debug/deps/tier1_writehigh_approval-c619119e5d4e124f)`: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.28s
- `Running tests/tier2_governance_drafts.rs (/home/shiro/rantai/cargo-target/debug/deps/tier2_governance_drafts-9ba9c3b309c72f87)`: test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.27s
- `Running tests/tier2_writehigh_approval.rs (/home/shiro/rantai/cargo-target/debug/deps/tier2_writehigh_approval-c4b77a5b95598ce3)`: test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.26s
- `Running tests/upload_routes.rs (/home/shiro/rantai/cargo-target/debug/deps/upload_routes-aa1621a365170d3e)`: test result: ok. 86 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 34.40s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_auth-46a20aa5c7778d69)`: test result: ok. 56 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.06s
- `Running tests/oidc.rs (/home/shiro/rantai/cargo-target/debug/deps/oidc-3c810bc833f69687)`: test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 49.31s
- `Running tests/openfga.rs (/home/shiro/rantai/cargo-target/debug/deps/openfga-0e6c9992615b871b)`: test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
- `Running tests/password.rs (/home/shiro/rantai/cargo-target/debug/deps/password-bab71ddb9c816e99)`: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.93s
- `Running tests/repository.rs (/home/shiro/rantai/cargo-target/debug/deps/repository-30046d79be1ef644)`: test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.24s
- `Running tests/service_token.rs (/home/shiro/rantai/cargo-target/debug/deps/service_token-0a0136f5e1651dd7)`: test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.37s
- `Running tests/session.rs (/home/shiro/rantai/cargo-target/debug/deps/session-a93d7807b7f3c902)`: test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.48s
- `Running tests/throttle.rs (/home/shiro/rantai/cargo-target/debug/deps/throttle-f26740721ba609ba)`: test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.34s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_bi-dbec9bfb72b55692)`: test result: ok. 74 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
- `Running tests/specs_match_typescript.rs (/home/shiro/rantai/cargo-target/debug/deps/specs_match_typescript-1c698a54a8451923)`: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_clickhouse-fcaa54535ff74688)`: test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_core-6bda010b0b723d51)`: test result: ok. 46 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_dagster-635930cd267d67dd)`: test result: ok. 67 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_embed-8c20da62849fa76b)`: test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
- `Running tests/cross_language.rs (/home/shiro/rantai/cargo-target/debug/deps/cross_language-97794b9b161a8c79)`: test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_iceberg-f957d066505ebbeb)`: test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
- `Running tests/g1_lakekeeper.rs (/home/shiro/rantai/cargo-target/debug/deps/g1_lakekeeper-c5427bb30d5c0508)`: test result: ok. 0 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_llm-5eb65b9fd61f65ae)`: test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_notify-8ea92a3cf4bc1610)`: test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_store-af7f40bab28a1ac3)`: test result: ok. 199 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.12s
- `Running tests/agents.rs (/home/shiro/rantai/cargo-target/debug/deps/agents-50596f907dd3fbe7)`: test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.84s
- `Running tests/annotation.rs (/home/shiro/rantai/cargo-target/debug/deps/annotation-f76590634197bb12)`: test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.35s
- `Running tests/audit.rs (/home/shiro/rantai/cargo-target/debug/deps/audit-e00d82a58f7724aa)`: test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.44s
- `Running tests/chat_term.rs (/home/shiro/rantai/cargo-target/debug/deps/chat_term-a917148ddc9b5cc3)`: test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.31s
- `Running tests/connector_ingest_spec.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_ingest_spec-dafeb2f1c27cd14c)`: test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.64s
- `Running tests/connector_ingest_spec_seed.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_ingest_spec_seed-29aeccc569fb7bf9)`: test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.99s
- `Running tests/connector_type.rs (/home/shiro/rantai/cargo-target/debug/deps/connector_type-bcb70d56bb5adcab)`: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.03s
- `Running tests/connectors.rs (/home/shiro/rantai/cargo-target/debug/deps/connectors-a9f66431ff53fcd9)`: test result: ok. 49 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.96s
- `Running tests/gold_publication.rs (/home/shiro/rantai/cargo-target/debug/deps/gold_publication-1695bc87b066f2a6)`: test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.09s
- `Running tests/governance.rs (/home/shiro/rantai/cargo-target/debug/deps/governance-6d65311dc383e1ee)`: test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.71s
- `Running tests/home_layout.rs (/home/shiro/rantai/cargo-target/debug/deps/home_layout-8b669db8e0ee5f80)`: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.19s
- `Running tests/identity.rs (/home/shiro/rantai/cargo-target/debug/deps/identity-5b720fa72927290d)`: test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.81s
- `Running tests/knowledge.rs (/home/shiro/rantai/cargo-target/debug/deps/knowledge-005ae4ad908ce53d)`: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.16s
- `Running tests/maintenance_policy.rs (/home/shiro/rantai/cargo-target/debug/deps/maintenance_policy-e87e9b2e5b57bd76)`: test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.40s
- `Running tests/overview.rs (/home/shiro/rantai/cargo-target/debug/deps/overview-5bf3a3093c5ff18f)`: test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.27s
- `Running tests/pipelines.rs (/home/shiro/rantai/cargo-target/debug/deps/pipelines-b6b320526e8ae639)`: test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.91s
- `Running tests/queries.rs (/home/shiro/rantai/cargo-target/debug/deps/queries-75e0738b68e667b9)`: test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.40s
- `Running tests/schema.rs (/home/shiro/rantai/cargo-target/debug/deps/schema-15eb76506b912552)`: test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.32s
- `Running tests/schema_change.rs (/home/shiro/rantai/cargo-target/debug/deps/schema_change-ce9d8ddce49e3df4)`: test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.81s
- `Running tests/semantic.rs (/home/shiro/rantai/cargo-target/debug/deps/semantic-eeaf2c459bad4026)`: test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.60s
- `Running tests/sessions.rs (/home/shiro/rantai/cargo-target/debug/deps/sessions-486f02d5e2fec0bb)`: test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.46s
- `Running tests/uploads.rs (/home/shiro/rantai/cargo-target/debug/deps/uploads-72a1b64ae74d5abb)`: test result: ok. 36 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.77s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_test_support-c3b9c8f27ef560f1)`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Running unittests src/lib.rs (/home/shiro/rantai/cargo-target/debug/deps/lakehouse_trino-a33aba485585cde0)`: test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
- `Doc-tests lakehouse_alerts`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_api`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_auth`: test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_bi`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_clickhouse`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_core`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_dagster`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_embed`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_iceberg`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_llm`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_notify`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_store`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_test_support`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- `Doc-tests lakehouse_trino`: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


## 6. Review

(Reviewer.)
