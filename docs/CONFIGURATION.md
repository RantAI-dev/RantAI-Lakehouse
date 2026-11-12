# Configuration

Every setting the stack reads, in one place. An install made by
`lakehousectl` writes the values it needs into `/opt/rantai-lakehouse/.env`;
`.env.example` is the short list of what a fresh install must or may decide
(26 set lines, plus one commented block per optional feature — `REL-19`).
The last column of every table says where a setting lives:

- **yes** — set in `.env.example`;
- **commented** — in one of its commented feature blocks, off by default;
- **no** — has a default in `docker-compose.yml` or the code, and is
  documented only here.

To switch on the features that are off by default (AI chat, email, single
sign-on, embeds), start with [OPERATIONS.md](OPERATIONS.md), "Features
unavailable in the local stack".

Every environment variable below is read in
[`rust/crates/lakehouse-api/src/config.rs`](../rust/crates/lakehouse-api/src/config.rs),
which is the authoritative source — this table was generated from it, not
guessed. A setting read only by compose, the frontend, an acceptance gate or
the installer says so in its row.

### Where each optional feature's settings are

Each commented block in `.env.example` ends by pointing here:

- <a id="email"></a>**Email** — `SMTP_*`, in the first table below.
- <a id="assistant"></a>**Assistant** — `LLM_*`, in the first table below.
- <a id="single-sign-on"></a>**Single sign-on** — `OIDC_*`, in the first table below.
- <a id="secret-managers"></a>**Secret managers** — "Secret managers and OAuth for connectors" below.
- <a id="backups"></a>**Backups to S3** — "Backups" below.
- <a id="tenant-storage"></a>**Tenant storage on outside S3** — `TENANT_WAREHOUSE_S3_*`, in the P1b table below.
- <a id="networks"></a>**Networks you allow** — `*_ALLOW_INTERNAL_HOSTS` and `*_ALLOWED_CIDRS`, throughout the tables below.

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `CH_URL` | ClickHouse HTTP interface URL | `http://localhost:18123` | No | no |
| `CH_USER` | ClickHouse basic-auth user | `default` | No | no |
| `CH_PASSWORD` | ClickHouse basic-auth password | `""` | No | no |
| `DAGSTER_URL` | Dagster GraphQL endpoint | `http://localhost:13030/graphql` (the code's default for a host-dev process); `http://dagster-webserver:3000/graphql` in compose, the in-network service name (`REL-19` review R1) | No | no |
| `DAGSTER_REPO` | Dagster repository name | `__repository__` | No | no |
| `DAGSTER_LOCATION` | Dagster repository location | `dispar_orchestrate.definitions` | No | no |
| `LLM_URL` | LLM chat-completions base URL (OpenAI-compatible). Settable in the console (Settings → Assistant) only when the environment sets neither this variable nor a key — see "Settings the console owns" below | `https://api.minimax.io/v1` | No | commented |
| `LLM_MODEL` | LLM model name. Settable in the console (Settings → Assistant) only when the environment sets neither this variable nor a key | `MiniMax-M3` | No | commented |
| `LLM_KEY` | LLM API key. Falls back to `MINIMAX_API_KEY` if unset **or empty** (`||` semantics, not `??`). Settable in the console (Settings → Assistant) only when the environment holds none; the console's own copy is sealed with `CONNECTOR_SECRETS_KEY` | `""` | No (but AI features won't work without it) | commented |
| `MINIMAX_API_KEY` | Fallback for `LLM_KEY` | — | No | commented |
| `AI_DEFAULT_REPLY_LANGUAGE` | Language the copilot answers in when a chat message is too short to detect its language: `id` (Indonesian) or `en` (English); any other non-empty value fails startup | — | No | no |
| `AI_SEMANTIC_LAYER` | Semantic layer switch: the model drafts a description of each table and column in the background and the copilot reads them. When on, the chat's system prompt also gets the rules for counting in a table grouped by several columns. `false` turns it off; `true`, empty or unset leaves it on; any other value fails startup | on | No | no |
| `AI_ASK_BACK` | Ask-back switch: the copilot may ask once, with options, which reading of an unclear word the person means (the `ask_user` tool), and reads the words that person answered before. `false` turns it off; `true`, empty or unset leaves it on; any other value fails startup | on | No | no |
| `AI_RELEVANT_TABLES` | Relevant-tables switch: the copilot's data map writes in full only the tables a question names and the other tables on one line each. `false` turns it off; `true`, empty or unset leaves it on; any other value fails startup | on | No | no |
| `EMBED_SECRET` | HMAC signing secret for signed dashboard embeds. The only source: none is generated or stored. Unset or empty: signed embedding is unavailable and says so | unset (signed embedding unavailable) | No | no |
| `EMBED_TOKEN_MAX_LIFETIME_SECONDS` | Longest `exp - iat` of a signed embed token, in seconds; a token must carry `exp` and `iat`. A value that is not a positive whole number fails startup. Settable in the console (Settings → Sign-in) only when this is unset | `86400` | No | no |
| `GOTENBERG_URL` | Address of the Gotenberg container that makes dashboard PDFs (`BI-1`). Compose sets it to the `gotenberg` service, which has no published port | unset (`http://gotenberg:3000` in compose) | No, but PDF export is off without it | no |
| `CONSOLE_INTERNAL_URL` | The console as the PDF renderer reaches it on the compose network, `http://console:3000` in this stack. It is also the only address the renderer may open. Unset or empty both mean the `console` service (`REL-18`), so PDF export is on by default; a deployment that turns downloads off for a role does so with `downloadDeniedRoles` | `http://console:3000` (compose) | No | no |
| `RENDER_TOKEN_SECRET` | Server-only key of the 2-minute token the PDF print view reads with. Its own secret, never `EMBED_SECRET`. `sh ops/init-env.sh` generates one | unset (PDF export off) | No, but PDF export is off without it | yes |
| `CONSOLE_PUBLIC_URL` | The console's address as people open it; a scheduled report's email links to `<this>/dashboards/<id>` (`RPT-1`). Reports are sent only when `SMTP_HOST` is set. Also settable in the console when this is unset | unset (the email has no link) | No | yes |
| `ALERTS_RUN_TOKEN` | Shared bearer token required to call `POST /api/alerts/run`; when set, also seeds the scope-less `alerts-run-scheduler` service identity Dagster's `alerts_run_schedule` authenticates as | unset | No, but unset means the endpoint replies 503 to anyone except an authenticated service-identity principal (not 401) — see Security notes below | no |
| `LAKEHOUSE_MAINTENANCE_TOKEN` | Shared bearer token Dagster's maintenance job uses to authenticate against `GET /api/lakehouse/maintenance-policies`; when set, also seeds the scope-less `lakehouse-maintenance-policy-reader` service identity from it | unset | No, but unset means the API seeds no service identity and the maintenance job's policy fetch fails visibly with 401 | no |
| `INGEST_SERVICE_TOKEN` | Shared bearer token Dagster's ingest schedule factory uses to authenticate against `GET /api/connectors/ingestible` and a CDC connector's `dial`; when set, also seeds an `ingest-run-service` identity scoped to `ingest:read` ONLY (never `connector:manage`) from it | unset | No, but unset means the API seeds no service identity and the ingest schedule factory's connector fetch fails visibly with 401 | no |
| `PIPELINE_RUN_TOKEN` | Shared bearer token Dagster's authored-pipeline schedule factory (`authored_factory.py`, Phase E) uses to authenticate against `lakehouse-api`; when set, also seeds an `authored-pipeline-scheduler` identity scoped to `pipeline:write` ONLY (never `*:*`) from it | unset | No, but unset means the API seeds no service identity and the schedule factory's pipeline calls fail visibly with 401 | no |
| `TRINO_URL` | Base URL of the Trino coordinator `POST /api/query/run`'s `engine: "trino"` path talks to | `http://trino:8080` | No, but unreachable means that path replies 503 | no |
| `TRINO_MAX_ROWS` | Hard cap on rows a single Trino statement may accumulate across pages before the query is cancelled and refused | `10000` | No | no |
| `SMTP_HOST` | SMTP host for alert/digest email delivery. Settable in the console (Settings → Email) only when the whole connection is unset in the environment — see "Settings the console owns" below | unset (email disabled) | No | commented |
| `SMTP_PORT` | SMTP port. Invalid values log a warning and fall back to the default rather than failing boot. Part of the connection, so it is settable in the console only when the whole connection is unset in the environment | `587` | No | commented |
| `SMTP_SECURE` | Force implicit TLS (`"true"`). Effective value is also `true` whenever `SMTP_PORT` is `465`, even if this is unset. Part of the connection, so it is settable in the console only when the whole connection is unset in the environment | `false` | No | commented |
| `SMTP_USER` | SMTP auth username. Part of the connection, so it is settable in the console only when the whole connection is unset in the environment | unset (no SMTP auth) | No | commented |
| `SMTP_PASS` | SMTP auth password. Part of the connection, so it is settable in the console only when the whole connection is unset in the environment; the console's own copy is sealed with `CONNECTOR_SECRETS_KEY` | `""` | No | commented |
| `SMTP_FROM` | `From` header for outgoing email. Falls back to `SMTP_USER`, then to `rantai-lake@localhost`. Also settable in the console when this is unset | `rantai-lake@localhost` | No | commented |
| `PORT` | Port `lakehouse-api` listens on | `8080` | No (but an invalid value fails to boot — the one field that does) | no |
| `APP_ENV` | `"development"` or `"local"` relaxes the session cookie's `Secure` attribute for local HTTP dev. Falls back to `NODE_ENV`. Never bypasses authentication | unset (defaults closed/`Secure`) | No | no |
| `NODE_ENV` | Fallback for `APP_ENV` | — | No | no |
| `AUTH_BOOTSTRAP_EMAIL` | Email for the idempotent bootstrap admin account created at startup | unset (no bootstrap admin) | No, but recommended for first run | yes |
| `AUTH_BOOTSTRAP_PASSWORD` | Password for the bootstrap admin account | unset | No, but required alongside `AUTH_BOOTSTRAP_EMAIL` to actually create one | yes |
| `LOGIN_MAX_FAILURES` | Failed password login attempts tolerated per email within `LOGIN_FAILURE_WINDOW_SECS` before lockout. There is no off switch: `0`, negative, and unparseable values all fall back to the default, so an operator can never accidentally silence the throttle. Settable in the console (Settings → Sign-in) only when this is unset | `5` | No | no |
| `LOGIN_FAILURE_WINDOW_SECS` | Rolling window (seconds) counted for login failures. Invalid values fall back to the default. Settable in the console (Settings → Sign-in) only when this is unset | `900` (15 min) | No | no |
| `LOGIN_LOCKOUT_SECS` | Duration (seconds) a locked-out email stays locked. Invalid values fall back to the default. Settable in the console (Settings → Sign-in) only when this is unset | `300` (5 min) | No | no |
| `AUTH_RETENTION_DAYS` | How many days to keep expired sessions and revoked credentials before the background cleanup job purges them. Throttle rows are not gated by this — an unlocked row with a lapsed window is purged as soon as its window lapses. Settable in the console (Settings → Sign-in) only when this is unset | `30` | No | no |
| `OIDC_ISSUER` | OIDC provider issuer URL | unset | No — OIDC requires both this and `OIDC_CLIENT_ID` | commented |
| `OIDC_CLIENT_ID` | This app's client id as registered with the OIDC provider | unset | No — see above | commented |
| `OIDC_CLIENT_SECRET` | Client secret sent as `client_secret` in the authorization-code exchange at `OIDC_TOKEN_URL` (`routes/auth.rs`); leave unset for a public client | unset | No | commented |
| `OIDC_PROVIDER_NAME` | Short label combined with `oidc:` to form `Principal::provider` / `auth_identity.provider` | `default` | No | commented |
| `OIDC_JWKS_URL` | Explicit JWKS endpoint override. Unset means the `jwks_uri` from `{OIDC_ISSUER}/.well-known/openid-configuration` (`SEC-38`) | discovery | No | commented |
| `OIDC_JIT_PROVISIONING` | `"true"` to auto-create an `app_user` for a validating token with no linked identity yet | `false` | No | commented |
| `OIDC_TRUST_EMAIL` | `"true"` joins an existing account by email even when the token does not carry `email_verified: true` (`SEC-65`). Off by default; only for a provider that never sends the claim | `false` | No | commented |
| `OIDC_ROLE_MAP` | `"group1=Role One,group2=Role Two"` — maps an IdP group/role claim to a local role name | empty | No | commented |
| `OIDC_GROUPS_CLAIM` | Which token claim carries the caller's groups/roles | `groups` | No | commented |
| `OIDC_CLOCK_SKEW_SECONDS` | Clock-skew tolerance for `exp`/`nbf` validation. Invalid values fall back to the default | `60` | No | no |
| `OIDC_AUTHORIZE_URL` | Authorization endpoint of the OIDC provider, for the browser-redirect login flow. Read ONLY from this var, never from a request | unset (browser SSO login stays off) | No | commented |
| `OIDC_TOKEN_URL` | Token endpoint of the OIDC provider, used to exchange an authorization code for tokens. Read ONLY from this var, never from a request | unset (browser SSO login stays off) | No | commented |
| `OIDC_REDIRECT_URI` | This deployment's fixed callback URL, registered with the OIDC provider ahead of time. Read ONLY from this var, never from a request, so a caller can never redirect a login or override the callback URL | unset (browser SSO login stays off) | No | commented |
| `DATABASE_URL` | Postgres connection string for Phase 2 OLTP storage | unset (no Postgres pool; every Phase 2 route returns 503) | No (but a wrong/unreachable value means every Phase 2 route fails — see "The service does not refuse to boot when Postgres is down" in [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md)) | no |

These are also read by `lakehouse-api` (`config.rs`, and `routes/mod.rs` for
the chat timeout); they were missing from the table above until 2026-10-15
(`DOC-1`):

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `AGENT_RUN_TOKEN` | Shared token with which the orchestrator's digital-employee schedule runs an agent headlessly (`x-run-token`) | unset (only a principal holding `agent:manage` can run one) | Only for scheduled agent runs | no |
| `AI_SQL_HELP` | Whether the assistant may explain a Query Studio statement and propose a corrected one; `false` only for the exact string `"false"` | `true` | No | no |
| `AI_CHAT_TIMEOUT_SECS` | Request timeout of `/api/ai/chat` in whole seconds, clamped to 30–1800; raise it for a slow model served on CPU | `120` | No | no |
| `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS` | Lets connection tests dial internal addresses (the guard against using the console as a port scanner, `SEC-15`); `true` only for the exact string `"true"` | unset (refused) | Only to test connectors that point at internal hosts, such as the seeded ones | commented |
| `CONNECTOR_PROBE_ALLOWED_CIDRS` | Private networks a connector may dial although internal addresses are refused: the narrow alternative to the setting above | empty | No | commented |
| `WEBHOOK_ALLOWED_CIDRS` | Internal networks an alert webhook may call (`SEC-10`); never opens loopback, link-local or multicast | empty | No | commented |
| `CONNECTOR_SECRETS_DIR` | Where the API itself reads and writes connector credential files; refs keep saying `file:/run/secrets/...`, the orchestrator's path | `/run/secrets` | Only when the API runs outside Docker | no |
| `CONNECTOR_TLS_CA_FILE` | The CA certificate (PEM) connector dials verify the database server against (`SEC-14` part B); unset still requires TLS but trusts the server's certificate (`require`), set verifies the chain (and, for the PostgreSQL connection test, the host name too: `verify-full`); an unreadable path stops the API at start | unset | No | no |
| `ICEBERG_QUERY_DB` | The `ClickHouse` `DataLakeCatalog` database the API reads Iceberg tables through (compose creates `icecat_api`) | unset in the API (compose sets `icecat_api`); unset means Iceberg tables stay unreadable for policy checks and the catalog | Only when running the API outside compose | no |
| `GIT_SHA` | The commit the image was built from; lets the pipeline source view say whether the code shown is the code that ran | `unknown` | No | no |

`WEBHOOK_TEST_ALLOW_ALL` exists in `config.rs` but is read only in test
builds; no deployment setting can open loopback for webhooks.

Two additional variables live outside `config.rs`, on the frontend side —
listed here because you need them to run the console at all:

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `RUST_API_URL` | Target the Next.js `/api/*` rewrite proxies to, read when `next build` runs (`next.config.ts`) | `http://localhost:8080` | Only when the API is not on `localhost:8080` | no |
| `NEXT_PUBLIC_SSO_ENABLED` | Build-time flag that shows/hides the SSO login UI | unset (SSO UI hidden) | No — see "SSO is gated by a build-time flag" in [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) | no |

See `rust/crates/lakehouse-auth/README.md` for detailed, per-provider OIDC
setup instructions (Okta, Entra, Google, Keycloak).

A further set of variables live only in `docker-compose.yml` — they
configure the P1 object store (RustFS) and Iceberg REST catalog
(Lakekeeper) *services themselves* (ports, Lakekeeper's own Postgres
database, its encryption key) and are not read by `config.rs`, because
they configure the container, not the `lakehouse-iceberg` client that
talks to it:

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `RUSTFS_ACCESS_KEY` | RustFS S3 API access key — RustFS's own ROOT credential. No longer what the seeded `conn-s3-warehouse` connector dials with (see `CONNECTOR_S3_ACCESS_KEY` below); migration 0023 moved that off this var so the connector secret allowlist never has to name RustFS's root key | none; **must be set** (compose refuses to start without it) | Yes | yes |
| `RUSTFS_SECRET_KEY` | RustFS S3 API secret key. Same note as `RUSTFS_ACCESS_KEY` above | none; **must be set** | Yes | yes |
| `CONNECTOR_S3_ACCESS_KEY` | Access key the seeded `conn-s3-warehouse` connector's `secretRef` (`env:CONNECTOR_S3_ACCESS_KEY`) resolves to for a real connectivity test, and what `dagster/dispar_orchestrate/dlt_pipeline.py` and `ops/g3/g3_loadgen.py` authenticate to RustFS with. Deliberately a name distinct from `RUSTFS_ACCESS_KEY` (RustFS's own root key) — this name matches the connector secret allowlist's fixed `env:CONNECTOR_*_ACCESS_KEY` pattern (`CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`), so the allowlist never has to name a process secret to let the seeded connector dial. When empty, falls back to `RUSTFS_ACCESS_KEY` (never to a literal); set to a least-privilege identity in a real deployment | the value of `RUSTFS_ACCESS_KEY` | No | no |
| `CONNECTOR_S3_SECRET_KEY` | Secret key half of the pair above (`env:CONNECTOR_S3_SECRET_KEY`); when empty, falls back to `RUSTFS_SECRET_KEY` | the value of `RUSTFS_SECRET_KEY` | No | no |
| `RUSTFS_HOST_PORT` | Host port mapped to RustFS's S3 API (container port 9000) | `9010` | No | yes |
| `RUSTFS_CONSOLE_HOST_PORT` | Host port mapped to RustFS's web console (container port 9001) | `9011` | No | yes |
| `LAKEKEEPER_PG_DB` | Name of Lakekeeper's own Postgres database on the existing `postgres` service (separate from the `lakehouse` app database's `console` schema) | `lakekeeper` | No | no |
| `LAKEKEEPER_ENCRYPTION_KEY` | Encrypts secrets in Lakekeeper's own schema | none; **must be set** (compose refuses to start without it). Changing it later makes the credentials Lakekeeper already stores unreadable | Yes | yes |
| `LAKEKEEPER_BASE_URI` | Base URL Lakekeeper advertises in its own REST responses — the `lakekeeper` compose service's `LAKEKEEPER__BASE_URI` key reads this same variable, and every REST catalog client honors that advertisement (`iceberg-catalog-rest` and `pyiceberg` both switch to it), so setting it moves the advertisement AND the API's management-API base URL (see the second `LAKEKEEPER_BASE_URI` row below) | `http://lakekeeper:8181` in compose (the in-network name every client of this stack can reach — `REL-19` review R1); `http://localhost:8181` in the code, for a host-side client | No | no |
| `LAKEKEEPER_HOST_PORT` | Host port mapped to Lakekeeper's REST API (container port 8181) | `8181` | No | yes |
| `LAKEKEEPER_OPENFGA_STORE_NAME` | Name of the OpenFGA store Lakekeeper's authorization model lives in | `lakekeeper` | No | no |
| `OPENFGA_PG_DB` | Name of OpenFGA's own Postgres database on the existing `postgres` service | `openfga` | No | no |

P1b (`lakehouse-iceberg`) adds the client-side counterparts below, read by
`config.rs` — these are what a Rust process (not the container) uses to
*connect to* RustFS/Lakekeeper. `lakehouse-api`'s Gold export route
(`routes::gold`, ADR 0010) is the first route to actually build an
`IcebergClient` from these fields; the G1 test and any manual `cargo run`
usage share the same documented source docker-compose already uses:

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `LAKEKEEPER_CATALOG_URI` | Lakekeeper's Iceberg REST catalog base URI, as reached from the Rust process | `http://localhost:8181/catalog` | No | no |
| `LAKEKEEPER_WAREHOUSE` | Lakekeeper warehouse this deployment writes Bronze tables into — see ADR 0003 for the `TENANT_ID` naming convention | `default` | No | no |
| `LAKEKEEPER_CREDENTIAL_SECRET_REF` | `secretRef` (see `lakehouse_core::secret`, ADR 0002) for Lakekeeper's OAuth2 client-credential, when Lakekeeper authorization is enabled | unset (no-auth mode assumed) | No | no |
| `LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE` | File path to the `gold-export` Lakekeeper principal's pre-minted static bearer token (ADR 0011), read at export-request time — not a `secretRef`, see the field's doc comment for why | `/tokens/gold-export.jwt` | No | no |
| `LAKEKEEPER_READ_TOKEN_FILE` | File path to the read-only `lakehouse-api-reader` Lakekeeper principal's pre-minted static bearer token (ADR 0011, WS2 amendment A0), read by `/api/lakehouse/*`'s catalog routes at (re)connect time — same not-a-`secretRef` shape as `LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE`, and the same no-refresh caveat: the file is re-read on each (re)connect, but nothing re-mints it before its 30-day expiry | `/tokens/lakehouse-api-reader.jwt` | No | no |
| `LAKEKEEPER_ADMIN_TOKEN_FILE` | File path to an ADMIN-scoped Lakekeeper principal's pre-minted static bearer token, read by `AppState::lakekeeper_admin` (`POST /api/identity/tenants`'s provisioning path — the only caller able to create warehouses/grants via Lakekeeper's `management/v1/*` API). A missing/unreadable file leaves `lakekeeper_admin` at `None`, degrading provisioning to unavailable rather than failing boot | `/tokens/admin.jwt` | No | no |
| `LAKEKEEPER_OUTSIDE_READ_TOKEN_FILE` | File path to the read-only `outside-readers` Lakekeeper principal's pre-minted static bearer token (DATA-5, ADR 0016), read by the API's `/api/iceberg/*` door on every call. The outside tool's OWN key never reaches Lakekeeper — the door forwards the call under this principal instead. Same not-a-`secretRef` shape and the same no-refresh caveat as `LAKEKEEPER_READ_TOKEN_FILE` | `/tokens/outside-readers.jwt` | No | no |
| `OUTSIDE_S3_ENDPOINT` | The storage address an operator PUBLISHES for outside readers (DATA-5, ADR 0016 point 4). Lakekeeper vends the deployment's internal name (`http://rustfs:9000`), which resolves only inside the network, so the API swaps it for this one in a table-load and a credentials response. Unset means outside reading is OFF: every door route answers 503 naming the missing setting, and the "Readers (7 days)" panel says "Not measured" with that reason — never zero | unset | No | no |
| `TENANT_WAREHOUSE_S3_ENDPOINT` | S3-compatible endpoint `POST /api/identity/tenants` creates a tenant's Lakekeeper warehouse against (`storage-profile.endpoint`). DEDICATED, not `RUSTFS_S3_ENDPOINT` (ADR 0002 Addendum 2's "never the process's own secret" rule applied to this config too). Unset, together with the access/secret key below, makes tenant provisioning refuse honestly (`tenant warehouse storage is not configured`) instead of sending Lakekeeper a request the endpoint of which was never set | unset | No | commented |
| `TENANT_WAREHOUSE_S3_BUCKET` | Bucket every tenant warehouse is created under (`storage-profile.bucket`). MUST NOT equal `LAKEHOUSE_WAREHOUSE_BUCKET`: `lakekeeper-warehouse-init` creates the shared `default` warehouse at that bucket's storage-profile ROOT (no `key-prefix`), and Lakekeeper refuses any other warehouse anywhere inside a bucket a root-level profile already claims (`CreateWarehouseStorageProfileOverlap`) — `routes::identity::tenant_warehouse_storage` refuses honestly (503) rather than send a request that always 400s | `lakehouse-tenant-warehouses` | No | commented |
| `TENANT_WAREHOUSE_S3_REGION` | Region string sent for a tenant warehouse's `storage-profile.region` | `us-east-1` | No | commented |
| `TENANT_WAREHOUSE_S3_PATH_STYLE_ACCESS` | `storage-profile.path-style-access` — `true` for `RustFS`/most on-prem S3 gateways | `true` | No | commented |
| `TENANT_WAREHOUSE_STS_ENABLED` | `storage-profile.sts-enabled` — whether Lakekeeper vends STS-scoped credentials for a tenant warehouse's tables | `true` | No | commented |
| `TENANT_WAREHOUSE_STS_ROLE_ARN` | `storage-profile.sts-role-arn` — the role Lakekeeper assumes to vend those STS credentials; the default matches `docker-compose.yml`'s `lakekeeper-warehouse-init` literal (a dev/test ARN, not a real credential) | `arn:aws:iam::000000000000:role/lakekeeper` | No | commented |
| `TENANT_WAREHOUSE_S3_ACCESS_KEY` | Access key id for the `storage-credential` every tenant warehouse is created with. DEDICATED, never `RUSTFS_ACCESS_KEY` — same ADR 0002 Addendum 2 rule `CONNECTOR_S3_ACCESS_KEY` already follows. Unset makes tenant provisioning refuse honestly rather than attempt a request Lakekeeper would reject | unset | No | yes |
| `TENANT_WAREHOUSE_S3_SECRET_KEY` | Matching secret access key. Never logged, never placed in an error; wrapped in `lakehouse_auth::Secret` the moment it crosses into `lakehouse_auth::openfga` | unset | No | yes |
| `LAKEKEEPER_BASE_URI` | Base URL of Lakekeeper's `management/v1/*` API that `AppState::lakekeeper_admin` dials — a SEPARATE meaning from `LAKEKEEPER_CATALOG_URI` above, and one this var name also serves for the `lakekeeper` compose service's own externally-advertised base URI (see `.env.example`'s comment on this var and `docs/plans/P5-RESULT.md` §4): inside a container `localhost` is that container, so in compose the API, the Lakekeeper init job AND the advertised URI default to the in-network `http://lakekeeper:8181` (tenant provisioning would fail on `localhost`, and so would every REST catalog client that honors the advertisement); the code's default for a host-dev process stays `http://localhost:8181` | `http://localhost:8181` (code); `http://lakekeeper:8181` in compose (`REL-19` review R1) | No | no |
| `GOLD_SOURCE_SCHEMA` | ClickHouse schema `routes::gold` reads Gold marts from | `serving` | No | no |
| `GOLD_EXPORT_RUN_TOKEN` | Shared token gating `POST`/`GET /api/gold/export/{mart}` and `GET /api/gold/exports` (same D4 shape as `ALERTS_RUN_TOKEN`; the `.../consumers` route needs only a session, no token); a session holding the `gold:export` permission always passes regardless (WS6 — this is what lets the console's "Export now" button work even when this is set for the Dagster schedule); with neither, only a service-identity principal may call it | unset | No | no |
| `GOLD_EXPORT_MAX_ROWS` | Hard cap on rows a single Gold export will read/append; a mart over this fails outright (error names the mart and the cap) instead of silently truncating | `5000000` | No | no |
| `GOLD_EXPORT_BATCH_SIZE` | Rows read from ClickHouse and appended to Iceberg per batch, instead of materializing the whole mart in memory at once | `20000` | No | no |
| `RUSTFS_S3_ENDPOINT` | S3-compatible endpoint the `lakehouse-iceberg` `object_store` client targets, and the one the API's store for uploaded files and its RustFS health probe dial (ADR 0014). Under compose the `lakehouse-api` container does not read this `.env` value: it gets the name from `CH_RUSTFS_S3_ENDPOINT` (default `http://rustfs:9000`), because `localhost` inside a container is the container itself | `http://localhost:9010` (`http://rustfs:9000` in compose) | No | no |
| `RUSTFS_S3_REGION` | Region string sent to the S3 client (RustFS does not enforce AWS region semantics, but the S3 API requires a value) | `us-east-1` | No | no |
| `LAKEHOUSE_WAREHOUSE_BUCKET` | Bucket the lakehouse warehouse's Iceberg tables live under — also read by the compose `rustfs-bucket-init` job | `lakehouse-warehouse` | No | no |
| `RUSTFS_ACCESS_KEY_SECRET_REF` | `secretRef` (ADR 0002) for a static RustFS/S3 access key: the credential the API's store for uploaded files and its RustFS health probe use (ADR 0014), and a fallback outside the vended-credentials write path (see `lakehouse-iceberg`'s crate doc). A reference, not a secret; unset, or resolving to an empty value, means uploads are "not configured" | unset (`env:UPLOAD_S3_ACCESS_KEY` in compose) | No | no |
| `RUSTFS_SECRET_KEY_SECRET_REF` | `secretRef` (ADR 0002) for the matching static secret key. Same notes as the access-key reference above | unset (`env:UPLOAD_S3_SECRET_KEY` in compose) | No | no |
| `UPLOAD_S3_ACCESS_KEY` | Access key the default `RUSTFS_ACCESS_KEY_SECRET_REF` resolves to, for the API's store of uploaded files (ADR 0014, decision 1). DEDICATED, never `RUSTFS_ACCESS_KEY` (ADR 0002 Addendum 2). No default credential: empty means uploads answer 503 "Upload storage is not configured." Give it a credential limited to the warehouse bucket (and its `uploads/` prefix where the store supports per-prefix policies) | empty | No, but uploads do not work without it | no |
| `UPLOAD_S3_SECRET_KEY` | Secret key half of the pair above | empty | No, but uploads do not work without it | no |
| `TRINO_URL` | Trino coordinator base URL used only by the WS5 health probe (`GET /api/ops/services`); unset means "never probed," reported as `health: "unknown"`, never a fabricated outage for a deployment that never enables the `trino` compose profile. (Separate from the always-present, always-defaulted `trino_url` the `query.rs` Trino engine route dials — that one keeps its own `http://trino:8080` default so an unreachable Trino there stays a per-request 503, not a boot-time assumption.) | unset | No | no |
| `OPENFGA_URL` | `OpenFGA` base URL used only by the WS5 health probe; unset means "never probed," reported as `health: "unknown"` — `OpenFGA` has no host port in this compose file by default, so a bare-metal or single-service deployment may have no route to it at all | unset | No | no |

A further batch of compose-only variables were added in P2–P5 for
SeaweedFS (the P2 storage-compatibility target, matrix-profile only),
Trino (the P4 small-file-compaction escape hatch, ADR 0009), and Debezium
Server (P5 CDC). None of these are read by `config.rs` — they configure the
containers themselves, and (for Trino/Debezium) are only relevant behind
their opt-in compose profiles:

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `SEAWEEDFS_ACCESS_KEY` | SeaweedFS S3 API access key | none; empty unless you use `--profile seaweedfs`, which refuses to start (`seaweedfs-iam-init` exits 1) while it is empty | Only with `--profile seaweedfs` | yes |
| `SEAWEEDFS_SECRET_KEY` | SeaweedFS S3 API secret key | none; same rule as the access key | Only with `--profile seaweedfs` | yes |
| `TRINO_PUBLISH_HOST_PORT` | SECURITY: `trino` has no authentication of its own, so its host port is NOT published by default — `trino-maintenance-cron` (the only real consumer) reaches it in-network as `http://trino:8080`. Set to the literal string `trino` (matching the Compose profile, not `true`/`1`) to opt in to host access; any other value leaves it disabled. See the `trino-host-port` service comment in `docker-compose.yml` for why the value must be that exact profile name | unset (disabled) | No, and CHANGE ME only on a trusted host if you do | no |
| `TRINO_HOST_PORT` | Host port mapped to Trino's coordinator UI/API (`trino` profile), only published when `TRINO_PUBLISH_HOST_PORT` is also set. | `8091` | No | yes |
| `TRINO_CRON_INTERVAL_SECONDS` | How often `trino-maintenance-cron` runs `ALTER TABLE ... EXECUTE optimize` against every Bronze table (`trino` profile) | `21600` (6h) | No | no |
| `POSTGRES_USER` / `POSTGRES_PASSWORD` / `POSTGRES_DB` | Credentials and database for the compose `postgres` service. `POSTGRES_PASSWORD` is `lakehouse-api`'s OWN database password — no longer what the seeded `conn-pg-lakehouse` connector dials with (see `CONNECTOR_PG_PASSWORD` below) | `lakehouse` / none, **must be set** / `lakehouse` (user and database names are not secrets; the password has no default) | `POSTGRES_PASSWORD`: Yes | no / yes / no |
| `CONNECTOR_PG_PASSWORD` | Password the seeded `conn-pg-lakehouse` connector's `secretRef` (`env:CONNECTOR_PG_PASSWORD`) resolves to for a real connectivity test. Deliberately a name distinct from `POSTGRES_PASSWORD` (the console's own database password) — this name matches the connector secret allowlist's fixed `env:CONNECTOR_*_PASSWORD` pattern (`CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`), so the allowlist never has to name a process secret to let the seeded connector dial. A connector created through the console or the copilot does NOT name its own var (ADR 0002 Addendum 3, `docs/adr/0002-secretref-resolution.md`): it picks a source/kind, and the server derives an `env:CONNECTOR_CONN_<ID>_<PASSWORD\|SECRET_KEY\|ACCESS_KEY\|API_KEY\|TOKEN>` (or `file:/run/secrets/connector_conn_<id>_<suffix>`) name from the id it generates, returned once at creation — the six allowlist patterns are still fixed in code, not configurable, but the exact var name is per-connector and cannot be listed here. When empty, falls back to `POSTGRES_PASSWORD` (never to a literal); set to a least-privilege credential in a real deployment | the value of `POSTGRES_PASSWORD` | No | no |
| `POSTGRES_HOST_PORT` | Host port for Postgres | `5432` | No | yes |
| `CH_HTTP_HOST_PORT` / `CH_NATIVE_HOST_PORT` | Host ports for ClickHouse's HTTP and native interfaces | `8123` / `9000` | No | yes / yes |
| `CH_DB` | ClickHouse default database created at first boot | `default` | No | no |
| `API_HOST_PORT` | Host port for `lakehouse-api` | `8080` | No | yes |
| `DAGSTER_WEBSERVER_HOST_PORT` | Host port for the Dagster webserver (`dagster` profile). It is published on the host's loopback only (`127.0.0.1`): Dagster's UI has no sign-in, so anyone who can reach the port can launch runs, and nothing outside the compose network needs it (the API reaches it in-network). Reach the UI through an SSH tunnel | `3001` | No | yes |
| `SEAWEEDFS_S3_HOST_PORT` | Host port for SeaweedFS's S3 API (`seaweedfs` profile) | `8333` | No | yes |
| `DAGSTER_PG_DB` | Dagster's own Postgres database, created on the existing `postgres` service | `dagster` | No | no |
| `OPENFGA_PG_DB` | OpenFGA's own Postgres database | `openfga` | No | no |
| `LAKEKEEPER_OPENFGA_STORE_NAME` | Name of the OpenFGA store holding the catalog authorization model | `lakekeeper` | No | no |
| `BRONZE_SOURCE_SCHEMA` / `BRONZE_SOURCE_TABLE` | Source schema and table the dlt batch-ingest job reads (`dagster` profile) | `ingest_demo` / `orders` | No | no / no |
| `BRONZE_TABLE_NAME` | Bronze Iceberg table the ingest job writes | `g3a_orders` | No | no |
| `LAKEHOUSE_API_URL` | In-network address Dagster jobs use to call the API back | `http://lakehouse-api:8080` | No | no |
| `CH_RUSTFS_S3_ENDPOINT` | S3 endpoint Iceberg clients use for object I/O. Switching this to the SeaweedFS service is what makes the G2 storage matrix a config-only change | `http://rustfs:9000` | No | no |
| `TENANT_OWNER` / `TENANT_ID` / `TENANT_DOMAIN` / `TENANT_RESIDENCY` / `TENANT_SITE` | Tenant identity surfaced on catalog assets, audit and quota records (`lakehouse-api/src/tenant.rs`). Each falls back to its historical default when blank | unset (historical defaults apply) | No | no / no / no / no / no |
| `BRONZE_CURATED_SLUGS` | Comma-separated dataset slugs presented as curated rather than raw Bronze | unset | No | no |
| `CATALOG_TENANT_ID` | UUID of the tenant that owns this deployment's single shared catalog and `Dagster`-job list. With more than one tenant and this unset, both are refused for every principal without the unrestricted grant (fail closed — neither has a tenant column, so they cannot be filtered). A malformed value fails startup | unset | No — but see the upgrade note in `docs/OPERATIONS.md` | yes |
| `CATALOG_NAMESPACE_META` | JSON object overriding catalog namespace display names/descriptions. Malformed JSON is ignored in favour of the defaults — a bad label is cosmetic, refusing to serve the catalog is an outage | unset | No | no |
| `BUILTIN_DASHBOARD_SPEC` | Path to a JSON file of built-in "Main" dashboard KPI/chart tiles for this tenant (shape: `{kpis:[...], charts:[...]}`, see `rust/crates/lakehouse-bi/specs/builtin-default.json`); unset, unreadable, or invalid means no built-in tiles at all | unset | No | no |
| `GOLD_SOURCE_SCHEMA` | ClickHouse schema Gold export reads marts from | `serving` | No | no |
| `GOLD_EXPORT_MARTS` | Optional operator override, unioned with per-mart publications the console switches on; default empty — a fresh deployment publishes only what is switched on in the console | (empty) | No | no |
| `GOLD_EXPORT_RUN_TOKEN` | Shared token required by `POST /api/gold/export/{mart}`. Seeds the `gold-export-scheduler` service identity at API boot, so the nightly schedule authenticates. Generate your own | unset | No | no |
| `GOLD_MART_NAME` / `GOLD_EXPORT_ROW_COUNT` | Mart name and row count the Gold export acceptance test seeds | `gold_export_smoke` / `7` | No | no / no |

### Upload limits and ingest retry

Counted inside one API process for received files, and in the database for
loads. Over a limit the answer is 429 with `Retry-After: 5` and nothing
waits.

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `UPLOAD_MAX_CONCURRENT_PER_USER` | Uploads one user may have in progress at once: files being received by the API and loads running are each counted against it | `4` | No | no |
| `UPLOAD_MAX_CONCURRENT` | Uploads the whole installation may have in progress at once (received files are counted per API process; loads are counted in the database) | `16` | No | no |
| `UPLOAD_MAX_CONCURRENT_LARGE` | LARGE loads (files totalling more than 512 MiB) that may run at once in the whole installation; the refusal names this setting | `2` | No | no |
| `UPLOAD_SESSION_TTL_HOURS` | Hours after which a chunked upload whose pieces never all arrived is aborted and removed (`DATA-17`) | `24` | No | no |
| `UPLOAD_MAX_CONVERSIONS` | Uploaded workbooks and Parquet files one API process reads at once; each read is its own memory-limited child process (`SEC-22`) | `2` | No | no |
| `UPLOAD_CONVERT_MEMORY_MB` | Memory limit that child sets for itself, in mebibytes; the measured peak of the largest allowed workbook times 1.5 | `1280` | No | no |
| `CONNECTOR_AUTO_DISABLE_AFTER` | Failed runs in a row after which the API pauses a connector and raises the "connector disabled" alert once (`SRC-11`) | `30` | No | no |
| `INGEST_RETRY_DELAYS_SECONDS` | Wait before each retry of a failing table, in seconds (one to three numbers), read by `dagster-code-location` | `60,300,900` | No | no |
| `INGEST_RUN_MAX_RUNTIME_SECONDS` | How long a connector run may run before it is stopped and recorded as failed (`SRC-39`), read by `lakehouse-api` and `dagster-code-location` | `21600` | No | no |
| `INGEST_RUN_START_TIMEOUT_SECONDS` | How long a connector run the orchestrator has taken off its queue may take to start before the API stops holding the connector (`SRC-39`); a run still waiting in the queue is judged by the run limit above; must equal `start_timeout_seconds` in `dagster/dagster.yaml` | `600` | No | no |

### Secret managers and OAuth for connectors

A secret manager is optional, and one per installation (`SRC-12`). Set
`SECRET_MANAGER` and give the product the sign-in below; connectors can then
reference a secret instead of holding a value, and the product reads it at
every test and run, keeping no copy. Give it an identity that can read only
the secrets it should: any `connector:manage` user can reference any secret
that identity can read. Leave it all empty to use typed values only.

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `SECRET_MANAGER` | `vault`, `aws`, `gcp` or `azure`; empty means typed values only | empty | No | commented |
| `SECRET_MANAGER_ALLOWED_PREFIXES` | Reference prefixes connectors may use, comma-separated, in the manager's reference form (e.g. `vault:secret/lakehouse/`); a prefix matches whole path segments, and `{tenant}` stands for the connector's tenant id. Empty with `SECRET_MANAGER` set: no reference is accepted | empty | No | commented |
| `VAULT_ADDR` | Vault address (KV version 2) | empty | No | commented |
| `VAULT_TOKEN` | Vault token, or the AppRole pair below | empty | No | commented |
| `VAULT_ROLE_ID` | Vault AppRole role id | empty | No | commented |
| `VAULT_SECRET_ID` | Vault AppRole secret id | empty | No | commented |
| `AWS_REGION` | AWS Secrets Manager region; on AWS the task, pod or instance role is used when no keys are given | empty | No | commented |
| `AWS_ACCESS_KEY_ID` | AWS access key id, when not using the platform role | empty | No | commented |
| `AWS_SECRET_ACCESS_KEY` | Matching AWS secret access key | empty | No | commented |
| `AWS_SESSION_TOKEN` | AWS session token for temporary credentials | empty | No | commented |
| `AWS_ENDPOINT_URL_SECRETS_MANAGER` | Endpoint override for a stand-in only | empty | No | commented |
| `GOOGLE_APPLICATION_CREDENTIALS` | Service-account key file path, mounted at the same path into `lakehouse-api` and the Dagster code location; empty uses the platform's identity | empty | No | commented |
| `AZURE_TENANT_ID` | Azure app registration tenant | empty | No | commented |
| `AZURE_CLIENT_ID` | Azure app registration client id; alone it picks a user-assigned managed identity | empty | No | commented |
| `AZURE_CLIENT_SECRET` | Azure app registration secret | empty | No | commented |
| `CONNECTOR_TYPED_CREDENTIALS` | `false` stops users typing credential values, so they can only reference a secret; empty or `true` keeps typing possible | empty | No | commented |
| `CONNECTOR_OAUTH_REDIRECT_URI` | Address the OAuth provider sends the browser back to: this console's address followed by `/api/connectors/oauth/callback` (`SRC-12` part C). Empty leaves OAuth sign-in refused, with a message naming it | empty | No | no |
| `CONNECTOR_OAUTH_ALLOW_HTTP` | Leave empty. `true` admits `http://` OAuth addresses and exists only for the `G6` acceptance gate's own provider | empty | No | no |

### Backups

The backup job's OWN bucket, never the warehouse bucket: a dump of the
identity database carries password and service-token hashes, so sharing an
access domain with warehouse credentials would turn one leaked connector
credential into every user's password hash. Only read under the `backups`
profile (`WS5`, `OPS-7`), from
`ops/backups/docker-compose.backups.override.yml`.

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `BACKUP_RETENTION_DAYS` | Days a backup is kept before the backup job removes it | `30` | No | yes |
| `BACKUP_S3_ENDPOINT` | Endpoint of the dedicated backup bucket | unset (the profile refuses to start) | Only with `--profile backups` | commented |
| `BACKUP_S3_BUCKET` | The dedicated backup bucket, never the warehouse bucket | `lakehouse-backups` | Only with `--profile backups` | commented |
| `BACKUP_S3_ACCESS_KEY_SECRET_REF` | `secretRef` the backup job dials with | `env:BACKUP_S3_ACCESS_KEY` | No | commented |
| `BACKUP_S3_SECRET_KEY_SECRET_REF` | `secretRef` of the matching secret key | `env:BACKUP_S3_SECRET_KEY` | No | commented |
| `BACKUP_S3_ACCESS_KEY` | Access key the reference above resolves to | empty | Only with `--profile backups` | commented |
| `BACKUP_S3_SECRET_KEY` | Secret key the reference above resolves to | empty | Only with `--profile backups` | commented |
| `BACKUP_WAREHOUSE_ENDPOINT` | Read-only source side for backup mirroring | `CH_RUSTFS_S3_ENDPOINT` | No | commented |
| `BACKUP_WAREHOUSE_BUCKET` | Warehouse bucket the mirror reads | `LAKEHOUSE_WAREHOUSE_BUCKET` | No | commented |
| `BACKUP_WAREHOUSE_ACCESS_KEY_SECRET_REF` | `secretRef` for the read-only warehouse mirror credential | `env:BACKUP_WAREHOUSE_ACCESS_KEY` | No | commented |
| `BACKUP_WAREHOUSE_SECRET_KEY_SECRET_REF` | `secretRef` for the matching secret key | `env:BACKUP_WAREHOUSE_SECRET_KEY` | No | commented |
| `BACKUP_WAREHOUSE_ACCESS_KEY` | Read-only warehouse access key | empty | No | commented |
| `BACKUP_WAREHOUSE_SECRET_KEY` | Read-only warehouse secret key | empty | No | commented |

### Installer, compose and gate-only names

| Variable | Purpose | Default | Required? | In `.env.example` |
| --- | --- | --- | --- | --- |
| `CONNECTOR_SECRETS_KEY` | AES-256-GCM key managed connector credentials are encrypted with (`SRC-12`), 32 random bytes in base64. Compose refuses to start while it is empty; losing it loses every stored credential | none; **must be set** (generated by `ops/init-env.sh` / `lakehousectl`) | Yes | yes |
| `CONNECTOR_SECRETS_KEY_PREVIOUS` | The OLD key, only while rotating: set it, run `docker compose run --rm lakehouse-api rotate-secrets-key`, then remove the line. It opens files; it never seals | unset | No | no |
| `SMTP_CA_FILE` | PEM bundle for an internal mail relay whose certificate chains to a private CA (`SEC-56`); those CAs are ADDED to the built-in roots. The path is the one inside the API container (`/run/certs/smtp/ca.pem` by default); unreadable, or no PEM, stops the API at start | unset (built-in roots only) | No | commented |
| `SEAWEEDFS_STS_SIGNING_KEY` | Signs the STS tokens SeaweedFS issues for the `G2` matrix warehouse. Unset generates a fresh random key per deployment on first boot into an empty volume | unset (generated per deployment) | Only to keep tokens across a `down`/`up` | no |
| `BRONZE_SOURCE_DB_HOST` | Postgres source `dlt` reads FROM (`dagster` profile) | `postgres` | No | no |
| `BRONZE_SOURCE_DB_PORT` | Port of that Postgres source | `5432` | No | no |
| `CONSOLE_HOST_PORT` | Host port the console publishes | `3000` | No | yes |
| `G8_TRINO_URL` | The `G8` gates' own in-network name for the Trino coordinator they wait for; deliberately NOT `TRINO_URL`, which may hold a host-facing address. Only the two `G8` gate runners read it | `http://trino:8080` | No | no |
| `INGEST_ALLOW_INTERNAL_HOSTS` | The ingest SSRF guard in `dagster-code-location` dials private addresses only when this is `true` (every internal address, loopback and cloud metadata included) | empty (refused) | No | commented |
| `INGEST_ALLOWED_CIDRS` | The narrow alternative for ingest: only these networks pass the guard (`SRC-15`); a bare address is one host | empty | No | commented |
| `CONSOLE_ALLOWED_DEV_ORIGINS` | Hostnames other than `localhost` that may load the console's dev server, comma-separated (`*.` allowed); `next dev` only (`OPS-11`). Read by `next.config.ts` | empty (localhost/127.0.0.1 only) | No | no |

Change capture (`SRC-4`, ADR 0016) runs on Kafka Connect: the `cdc` profile's
`kafka` and `kafka-connect` images are pinned by digest in `docker-compose.yml`,
and the API and the applier find them through `KAFKA_CONNECT_URL` and
`CDC_KAFKA_BOOTSTRAP` (both empty by default, which turns change capture off;
see `docs/OPERATIONS.md`).

## Settings the console owns (`ADM-7`, `ADM-8`, `ADM-9`)

### Email (`ADM-7`)

The mail server, its port, the security mode, the account and its password,
the `From` address and the console's public address can be set from the
console (**Settings → Email**) by a holder of `settings:write`. The next
send — an alert, a scheduled report, a comment mention — uses them without
a restart.

**The environment wins.** For each of those settings the effective value is
the environment's when it is set and non-empty, else the console's saved
value, else the code default. A setting the environment owns is shown
read-only in the console ("Set by the server"), and a save that sends it is
refused with a 409 and writes nothing. To hand a setting back to the
console, remove it from the environment (and restart the API); to hand it
back to the environment, set it there and remove it in the console.

**The connection is one unit.** The mail server, the port, the security,
the account and the password belong together: they are how the sender logs
in to a mail server. When the environment sets any one of `SMTP_HOST`,
`SMTP_USER` or `SMTP_PASS`, all five come from the environment and are
read-only in the console — so a password kept in files is never sent to a
mail server chosen in the console. `SMTP_FROM` and `CONSOLE_PUBLIC_URL` are
settings of their own and follow the rule above on their own.

**The password.** The console's copy is sealed with the installation key
(`CONNECTOR_SECRETS_KEY`) before it is stored, and it never comes back: an
answer says only whether one is set, and the audit row names the field, not
its value. A password saved with a key the installation no longer has is
read as "not set" and logged once.

**Other API processes.** The saved settings reach them within 30 seconds,
not at once: each process re-reads the saved row at most every 30 seconds.
The process that saved sees its own change immediately.

**What is not in the console.** The CA bundle of a private relay
(`SMTP_CA_FILE`) stays a file and is still read at start; the start-up
secrets and the encryption keys stay in the environment; single sign-on
stays in the environment (the console only says whether it is set up).

### Sign-in limits (`ADM-8`)

The failed-login limit (`LOGIN_MAX_FAILURES`), the window it is counted in
(`LOGIN_FAILURE_WINDOW_SECS`), the lockout (`LOGIN_LOCKOUT_SECS`), how long
sign-in records are kept (`AUTH_RETENTION_DAYS`) and the longest life of a
signed embed link (`EMBED_TOKEN_MAX_LIFETIME_SECONDS`) can be set from the
console (**Settings → Sign-in**) by a holder of `settings:write`. The next
sign-in uses them without a restart.

**The environment wins, one limit at a time.** Each limit follows the rule
above on its own: the environment's value when it is set and non-empty, else
the console's saved value, else the code default. A limit the environment
owns is shown read-only in the console ("Set by the server"), and a save
that sends it is refused with a 409 and writes nothing.

**The ranges.** The console accepts 3 to 20 failed tries, a window and a
lockout of 60 to 86400 seconds, 7 to 3650 days of records and an embed link
of 300 to 86400 seconds; a value outside its range is refused with a
sentence that names the field. The environment is not bound by those
ranges: it may set a longer window or a shorter lockout.

### The assistant's provider (`ADM-9`)

The provider address (`LLM_URL`), the model (`LLM_MODEL`) and the API key
(`LLM_KEY` or `MINIMAX_API_KEY`) can be set from the console (**Settings →
Assistant**) by a holder of `settings:write`. The next message uses them
without a restart.

**The provider is one unit.** The address, the model and the key belong
together: they are how the assistant reaches one provider. When the
environment holds a key — `LLM_KEY` or `MINIMAX_API_KEY`, whatever
`LLM_URL` and `LLM_MODEL` say — all three are the environment's, the
console shows them read-only, and a save that sends any of them is refused
with a 409 and writes nothing. So a key kept in files is never sent to an
address chosen in the console.

**Without a key, the address and the model answer for themselves.** The
environment owns `LLM_URL` and `LLM_MODEL` one at a time: a value set in
`.env` is used as it is, shown read-only, and a save that sends that field
is refused with a 409 — the console's saved value of it is ignored while
the environment's is set. Where the environment names neither, the console
owns the field and its saved value is used.

**The address the console sends** must be an `https://` address, and its
host must pass the same internal-address check the connector dials use
(`CONNECTOR_PROBE_ALLOWED_CIDRS`, `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`):
an address that points inside the deployment's network is refused, so an
administrator's account cannot make the API call a service inside it. The
environment's own address is not checked — it is the operator's file, and
an on-premises deployment may point its assistant at a gateway inside its
own network.

**The key.** The console's copy is sealed with the installation key
(`CONNECTOR_SECRETS_KEY`) before it is stored, and it never comes back: an
answer says only whether one is set, and the audit row names the field, not
its value. The address cannot be cleared while a key is set — the key would
go to the deployment's default provider.

**The test button** makes one minimal request with the saved values and
answers with one of four fixed sentences ("The assistant answered.", "Could
not reach the provider.", "The provider refused the key.", "The provider
does not know that model.") — never the provider's own words.

See `docs/OPERATIONS.md` for the operator's view of the same thing, and
`docs/core/features/console-settings.md` for the feature page.
