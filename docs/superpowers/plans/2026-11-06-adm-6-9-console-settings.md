# ADM-6 to ADM-9: deployment settings in the console

Backlog `ADM-6`, `ADM-7`, `ADM-8`, `ADM-9` (P2, Administration & Security). Planner:
Claude Opus, 2026-11-06. Anchors verified at `main` `d2b26f45`. Base branch: `main`.
Feature page `docs/core/features/console-settings.md`; spec
`docs/core/specs/console-settings.md`. Developer: DeepSeek (opencode), at the owner's
request, for the API and the console pages. Two PRs: slice 1 (`ADM-6`, `ADM-7`), then
slice 2 (`ADM-8`, `ADM-9`) on top of merged slice 1.

## 0. Today, with evidence

- Email: `Config::from_map` reads `SMTP_*` once (`rust/crates/lakehouse-api/src/config.rs:1537-1606`).
  Every sender builds from it through `routes::alerts::smtp_config(&Config)`
  (`routes/alerts.rs:844-856`), called at `alerts.rs:1418,1551`, `reports.rs:810`,
  `dashboard_comments.rs:775`, `report_runner.rs:1378`, `pipelines.rs:2677,2875`,
  `load_alerts.rs:308,343`, `authored_pipelines.rs:1185`. "Is email set up" is read as
  `config.smtp_host.is_some()` at `reports.rs:473`, `dashboard_comments.rs:768`,
  `routes/ai/tools/reports.rs:193`.
- Console address for links: `config.console_public_url` (`config.rs:1596`).
- Sign-in limits: `login_throttle_policy(&Config)` (`state.rs:693`) builds
  `AppState::throttle_policy: Arc<ThrottlePolicy>` (`state.rs:232,814,863`), read at
  `routes/auth.rs:181`. Retention: `main.rs:1070`. Embed lifetime:
  `config.embed_token_max_lifetime_seconds` (`config.rs:1358`).
- Assistant: `LlmClient::new(llm_url, llm_model, llm_key)` once (`state.rs:743-746`),
  held as `AppState::llm: Arc<LlmClient>` (`state.rs:84`); `llm_key` empty switches
  features off (`routes/ai/semantic.rs:128`).
- The settings pattern to copy: `routes/settings.rs` (`get_reporting` `:565`,
  `put_reporting` `:596`), store `lakehouse_store::reporting_settings`, migration
  `0070_reporting_settings.sql` (singleton row), policy rows `policy.rs:785-786`
  (`settings:write`), console `src/features/admin/settings-page.tsx` (tabs) and
  `reporting-settings-page.tsx`.
- Sealing: `secret_crypto::Keyring::seal/open(file_name, bytes)` (`secret_crypto.rs:193,204`),
  the keyring shared through `connector_secret_store::SharedKeyring` (`state.rs:30,456`).
- SSO status: `AppState`'s auth `oidc: Option<..>` (`state.rs:64`); PDF:
  `config.gotenberg_url`, `console_internal_url`, `render_token_secret`.
- The internal-address check for outbound dials: `resolve_checked` (used by
  `pg_probe.rs`, `kafka_connect.rs`).

## 1. Decisions

- **D1. Precedence: the environment wins.** For each field, the effective value is the
  environment's when it is set and non-empty, else the stored one, else the code default.
  Every GET answers each field as `{ "value": …, "source": "server" | "console" | "default" }`
  (secrets: `{ "set": bool, "source": … }`). A PUT that sends a field whose source is
  `server` is refused with 409 and the fixed sentence "This setting is set in the
  server's environment and cannot be changed here." Nothing is written.
- **D2. Storage.** One singleton table per section, same shape as `0070`: `email_settings`
  (slice 1), `sign_in_settings` and `assistant_settings` (slice 2). Every column nullable
  (null = not set in the console), plus `updated_at`, `updated_by`. Migrations take the
  next free numbers on `main` at build time (today `0127`), with a why-header; renumber
  before merge if `main` moved. Secrets (`smtp_pass`, `llm_key`) are `BYTEA`, sealed with
  the keyring under the fixed names `settings/email/password` and `settings/assistant/key`.
  A row that does not open (key changed) is treated as not set and logged once, never
  returned.
- **D3. One resolver, read through a short cache.** New module
  `rust/crates/lakehouse-api/src/runtime_settings.rs`:
  `async fn email(state) -> EffectiveEmail` (an `SmtpConfig` plus the console public
  URL), and in slice 2 `sign_in(state)` and `assistant(state)`. Each reads its row at most
  once per 30 s per process (a `tokio::sync::RwLock` with the read time), and a PUT on
  this process clears its cache at once. `smtp_config(&Config)` is replaced by the
  resolver at every call site in section 0, and every `smtp_host.is_some()` check reads
  the resolver's answer. No second merge rule anywhere (rule 4).
- **D4. Permission.** All new routes are `Policy::RequiresPermission("settings:write")`,
  GET included, in `POLICY_TABLE` and asserted both ways in `tests/route_auth.rs`.
  No seed change: `settings:write` already exists.
- **D5. Secrets on the wire.** A GET never carries a secret, only `set`. A PUT with the
  secret field absent keeps it; `null` clears it; a string replaces it. The secret is in
  `SecretValue` from parse to seal. It never appears in a log, an error or the audit row.
- **D6. Audit.** Every PUT writes one audit row (the existing helper used by
  `routes/maps.rs:120` or the shared one, grep first) with the action
  `settings.<section>.update` and the list of field NAMES changed.
- **D7. Errors.** Upstream text never reaches a response (principle 4). The test email
  answers one of these fixed sentences: "Sent to <your address>.", "Could not reach the
  mail server.", "The mail server refused the user name or password.", "The secure
  connection to the mail server failed.", "Email is not set up: the mail server is
  missing." Map from `lakehouse_notify`'s error kinds; add a kind there if one is
  missing, never by matching message text.

### Slice 1: `ADM-6` and `ADM-7`

- **D8. `GET /api/settings/deployment`** (`ADM-6`) answers a list, one entry per feature:
  `{ "feature": "pdfExport" | "email" | "consoleAddress" | "singleSignOn" | "assistant",
  "on": bool, "missing": [setting names], "where": "server" | "console" }`. Rules: PDF on
  when `gotenberg_url`, `console_internal_url` and `render_token_secret` are all set;
  email on when the resolved mail server is set; console address on when the resolved
  public URL is set; single sign-on on when `oidc` is `Some`; assistant on when the
  resolved key is non-empty. It reports configuration only, and the page says so
  ("set up", not "working"). Backups are not listed: the API cannot see the backup
  container's settings (feature page, Limits).
- **D9. `GET`/`PUT /api/settings/email`** (`ADM-7`): fields `host`, `port` (1–65535),
  `security` (`"tls"` | `"starttls"`, mapping to today's `smtp_secure`), `user`, `password`,
  `from` (an address), `consolePublicUrl` (`http(s)://`, trailing `/` trimmed, as
  `config.rs:1596`). Validation sentences are fixed and name the field.
- **D10. `POST /api/settings/email/test`** sends one short email to the caller's own
  account address, with the effective settings (saved ones; not the unsaved form). At
  most one per caller per 10 s (429 with `Retry-After`). Answer per D7.
- **D11. Console.** `settings-page.tsx` gains two tabs, **Deployment** (first) and
  **Email**, shown only to a holder of `settings:write` (the page already checks this
  for other sections; reuse that). Deployment: one row per feature, on/off pill, the
  missing settings, and a link to the tab that fixes it (Email) or the doc
  (`docs/CONFIGURATION.md`) for server-only ones. Email: a form in the existing settings
  section style (`reporting-settings-page.tsx`), fields with `source: "server"` disabled
  with the hint "Set by the server", password as a write-only field ("Set" / "Not set",
  Replace, Clear), Save, and Send test email showing D7's sentence. Contracts → client →
  `services/index.ts` → feature, `apiFetch`, `useService`/`useServiceAction`.

### Slice 2: `ADM-8` and `ADM-9`

- **D12. `GET`/`PUT /api/settings/sign-in`** (`ADM-8`): `maxFailures` (3–20),
  `failureWindowSecs` (60–86400), `lockoutSecs` (60–86400), `retentionDays` (7–3650),
  `embedMaxLifetimeSecs` (300 to the ceiling `parse_embed_token_max_lifetime` already
  enforces). `AppState::throttle_policy`, the retention sweep (`main.rs:1070`) and the
  embed lifetime check read the resolver. The Sign-in tab keeps the SSO status on top
  and gains this form below it.
- **D13. `GET`/`PUT /api/settings/assistant`** (`ADM-9`): `url`, `model`, `key`. `url`
  must be `https://` and its host must pass `resolve_checked` (no internal address), at
  save and at each client rebuild. `AppState::llm` becomes swappable
  (`Arc<RwLock<Arc<LlmClient>>>` or `arc_swap` if already a dependency; no new dependency
  otherwise); the resolver rebuilds the client when the effective values change.
  `POST /api/settings/assistant/test` makes one minimal request with the saved values
  and answers fixed sentences ("The assistant answered.", "Could not reach the
  provider.", "The provider refused the key.", "The provider does not know that model.").
  New **Assistant** tab.

## 2. Tasks

Slice 1 (Rust first, then the console):

| # | Task | Acceptance |
| --- | --- | --- |
| S1-T1 | Migration `email_settings` + `lakehouse_store::email_settings` (get, upsert, clear secret) | `sqlx::test`: upsert then get round-trips; null columns stay null |
| S1-T2 | `runtime_settings.rs` email resolver (D1, D2, D3) | Unit tests: env wins; console used when env empty; default when neither; a sealed password opens; a row sealed with another key reads as not set; cache cleared by a save |
| S1-T3 | Replace every `smtp_config(&state.config)` and `smtp_host.is_some()` from section 0 | `grep -rn 'smtp_config(&\|smtp_host.is_some' rust/crates/lakehouse-api/src` shows only the resolver; existing alert/report tests pass |
| S1-T4 | `GET`/`PUT /api/settings/email`, `POST …/test` (D4–D7, D9, D10), policy rows, `route_auth.rs` | Tests: 403 without `settings:write`; GET never contains the password; PUT on a `server` field → 409, nothing written; audit row lists field names, no secret; test email to the caller only (a mock SMTP, or the `smtp_private_ca.rs` relay pattern); second test within 10 s → 429. Mutation check: remove the 409 guard, its test fails; restore |
| S1-T5 | `GET /api/settings/deployment` (D8) | One test per feature for on and off with the right `missing` names |
| S1-T6 | Console: contracts, client, Deployment tab, Email tab (D11) | `bun test` for the new files: server-set field disabled; password never pre-filled; test result sentence shown |
| S1-T7 | Docs: `docs/CONFIGURATION.md` (email settings can be set in the console; env wins), `docs/OPERATIONS.md`, CHANGELOG `[Unreleased]` | — |

Slice 2:

| # | Task | Acceptance |
| --- | --- | --- |
| S2-T1 | Migrations `sign_in_settings`, `assistant_settings` + store modules | `sqlx::test` round-trips |
| S2-T2 | Resolvers `sign_in`, `assistant`; swappable `llm`; throttle, retention and embed read the resolver | Tests: a saved `maxFailures: 3` locks on the 4th failure without restart; env value wins; assistant client rebuilt on change |
| S2-T3 | Routes (D12, D13), policy rows, `route_auth.rs` | 403/409/audit as S1-T4; `url` with `http://` or an internal host → 400 with a fixed sentence; key never in GET. Mutation check on the internal-host check |
| S2-T4 | Console: Sign-in form, Assistant tab | `bun test` as S1-T6 |
| S2-T5 | Docs and CHANGELOG | — |

Checks per commit: Rust `cargo fmt --check`, `cargo clippy -p lakehouse-api --all-targets
-- -D warnings`, `cargo test -p lakehouse-api <filter>` (plus `-p lakehouse-store` /
`-p lakehouse-notify` when touched); TypeScript `bun run typecheck && bun run lint && bun
test <paths>`. Full block from AGENTS.md once per slice before handoff.

## 3. Out of scope

Backups in the console (`ADM-10`). SSO settings in the console. Network allowlists,
memory and upload limits, start-up secrets (feature page, Decisions). Pushing a change
to other API processes faster than the 30 s cache.

## 4. Handoff

### Handoff (slice 1) — developer, 2026-11-06

Branch `feat/adm-6-7-console-settings`, base `446eb551` (the previous
developer's `S1-T1`: migration `0127` `email_settings` + the store module).
Slice 1 only: `ADM-6` and `ADM-7`. Nothing of slice 2 (`ADM-8`, `ADM-9`)
was started.

**Commits** (one per task, in plan order; every one pushed):

| Commit | Task | What |
| --- | --- | --- |
| `91267502` | S1-T2 + S1-T3 | `runtime_settings.rs`: the one resolver (env → console → default) with its 30-second cache, and every sender and every "is email set up" check reading it. `routes::alerts::smtp_config` is gone |
| `b1e16437` | S1-T4 | `GET`/`PUT /api/settings/email`, `POST /api/settings/email/test`, the policy rows, `route_auth.rs`, and `lakehouse_notify`'s `send_classified` |
| `50459855` | S1-T5 | `GET /api/settings/deployment` |
| `e97b0601` | S1-T6 | the console: contracts, client, registry, the Deployment and Email sections |
| `5025dd25` | S1-T7 | `docs/CONFIGURATION.md`, `docs/OPERATIONS.md`, `CHANGELOG.md` |

**Migration**: `0127_email_settings.sql` (from `S1-T1`, already on the
branch). No migration was added in this slice.

**Commands run, with their counts** (foreground, from the branch's head):

- `cargo fmt --check` — clean.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — clean (4 m 31 s).
- `cargo test -p lakehouse-api -p lakehouse-store -p lakehouse-notify` — **8609 passed, 0 failed, 43 ignored** across 151 test binaries (42 m 58 s, exit 0).
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings` — clean.
- `cargo clippy -p lakehouse-notify --all-targets -- -D warnings` — clean.
- `cargo test -p lakehouse-api --test email_settings_route` — **11 passed** (S1-T4).
- `cargo test -p lakehouse-api --test deployment_settings_route` — **5 passed** (S1-T5).
- `cargo test -p lakehouse-api --test route_auth` — **80 passed** (the two new named tests among them).
- `cargo test -p lakehouse-api --test sec11_guard` — **4 passed**.
- `cargo test -p lakehouse-api --lib -- routes:: settings:: runtime_settings:: policy` — **2048 passed** (includes `every_policy_table_entry_is_mounted_and_gated_as_declared`).
- `cargo test -p lakehouse-api --lib route_policy` — **2 passed**.
- `cargo test -p lakehouse-api --lib runtime_settings` — **6 passed** (the resolver's own tests).
- `cargo test -p lakehouse-notify` — **32 passed** (lib) + **6 passed** (`smtp_private_ca`).
- `bun run typecheck` — clean.
- `bun --bun eslint src` — **0 errors**, 10 pre-existing warnings in files this slice does not touch.
- `bun run test` — **3476 passed, 1 skipped, 0 failed** (347 files).
- `python3 ops/lint/check_intra_package_imports.py` — OK; `python3 ops/lint/check_bare_iceberg_count.py` — OK.
- `docker compose --profile '*' config --quiet` — OK (no compose file was changed).
- `git status` — clean, nothing generated staged.

**Mutation check (S1-T4).** Done and restored: with the 409 guard in
`routes::settings::put_email` removed, `a_field_the_environment_owns_is_refused_and_nothing_is_written`
fails (the answer is 200 instead of 409, and the console's own port was
written). The guard was put back and the whole file passed again (11
passed).

**What the reviewer checks, and where it is proved:**

- *The password never appears in a GET, a log, an error or an audit row* —
  the answer carries `{"set": bool, "source": …}` only, the password is
  sealed before it is stored, the audit row names the field and not its
  value, and the row holds the sealed bytes: `the_password_never_comes_back_in_an_answer`,
  `a_save_writes_one_audit_row_naming_the_fields_and_never_the_secret`.
- *A PUT on a `server`-sourced field is 409 and writes nothing* — the guard
  runs before the row is read or written, and the whole save is refused:
  `a_field_the_environment_owns_is_refused_and_nothing_is_written` (the
  mutation check above).
- *Every new route needs `settings:write`, GET included* — the three
  policy rows and the two named tests in `tests/route_auth.rs`
  (`the_email_settings_routes_are_refused_without_settings_write`,
  `a_platform_admin_is_not_refused_the_email_settings_routes`).
- *The test email goes only to the caller's own address* — the request
  carries no address; the recipient is the caller's own account address,
  and the test proves it against a real relay:
  `the_test_email_goes_to_the_callers_own_address_and_to_nobody_else`
  (a `Mailpit` container behind a private CA, the `smtp_private_ca.rs`
  pattern: the relay holds exactly one message and its one recipient is
  the caller's address).
- *Only fixed sentences in responses (D7)* — the answer is one of five
  fixed sentences, mapped from `lakehouse_notify`'s failure **kinds**
  (never from message text: `lakehouse_notify` grew `send_classified` for
  this). The refusal sentences name the field they are about.

**Decisions the code makes where the plan left room** (all in the plan's
spirit, none contradicting it):

1. **S1-T2 and S1-T3 are one commit.** The resolver is dead code until
   every call site reads it, and `clippy -D warnings` — which this
   repository runs on every commit — refuses a commit that leaves a module
   unused. Splitting them would have needed an `allow(dead_code)`, which
   rule 2 forbids.
2. **A missing setting is named the way the operator sets it**: the
   environment variable for a setting only the server owns
   (`GOTENBERG_URL`, `OIDC_ISSUER`, `LLM_KEY`), the console's own field
   name for a setting the console can set (`host`, `consolePublicUrl`).
   `where` says which of the two it is.
3. **The deployment answer is `{"features": [ … ]}`** — the list under a
   key, like every other answer in this API.
4. **The test email answers 200** with `{"sent": bool, "message": "…"}`:
   the request was accepted and performed, and the outcome is in the body,
   in one of D7's sentences. A failed send says `sent: false`, never a
   success.
5. **The audit row lists the fields whose value the save changes.** A
   password that is sent always counts as changed (the sealed bytes are
   never compared, the seal is randomised).
6. **The console never sends a field the server owns** — it would be
   refused with a 409 — so the form disables it, says "Set by the server",
   and leaves it out of the save body.
7. **An empty password is no password**: the resolver reads it as "not
   set", so a save that sent `""` cannot look like a set one.

**Not verified:**

- The two new console sections were not looked at in a browser; their
  behaviour is proved by `bun test` (9 new tests) and by the API's own
  tests.
- `bun run lint` over the whole checkout fails on the files under
  `dagster/.venv/` (a local Python virtualenv, not in git, not touched by
  this slice). `bun --bun eslint src` is clean.
- The Dagster test suite (`pytest dispar_orchestrate`) and a real
  `docker compose up` were not run: this slice changed no Python and no
  compose file. `docker compose --profile '*' config --quiet` passes.
- Slice 2 (`ADM-8`, `ADM-9`) was not started, as instructed.

**Note for the reviewer about the build directory.** The shared
`CARGO_TARGET_DIR` (`/home/shiro/rantai/cargo-target`) is used by another
worktree at the same time (`/var/home/shiro/wt/rel-20-b`), and cargo linked
that worktree's older `lakehouse-store`/`lakehouse-notify` artifacts —
builds of this branch failed with "could not find `email_settings` in
`lakehouse_store`". No `cargo clean` was run; this slice's builds ran in
`CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target-adm67` (a cold, empty
directory for this branch) and every command above ran there.

### Handoff (review fixes R1, R2) — developer, 2026-11-06

Branch `feat/adm-6-7-console-settings`, at `a971571e` (the planner's review of
slice 1). Two commits, one per finding: the merge (R2), then the fix (R1).
Nothing else of slice 2 was started.

| Commit | Finding | What |
| --- | --- | --- |
| `d822b6fd` | R2 | `git merge --no-ff origin/main` (no rebase): `REL-20` (#397) moved the process into the library target |
| (this one) | R1 | the connection fields are one unit: the environment owns all five of them as soon as it sets one |

**R2 — what conflicted in the merge, and how it was resolved.**

Three files were changed on both sides; one conflicted.

- `rust/crates/lakehouse-api/src/main.rs` — **one conflict hunk**, the
  file header and the whole module block. `REL-20` (#397) emptied
  `main.rs` of every `mod` line and moved the process into the library
  target (`lakehouse_api::run`), while this branch had added
  `mod runtime_settings;` to that block. Resolved by hand, hunk by hunk:
  the header is `main`'s (the binary is the process entry point), and
  every `mod` line is gone — including this branch's — because the module
  tree lives only in `src/lib.rs` now. A short note at the top of the file
  says so and cites the finding. `--ours`/`--theirs` was not used on any
  file both sides changed.
- `rust/crates/lakehouse-api/src/lib.rs` — merged by git; the result is
  `main`'s file plus this branch's `pub mod runtime_settings;`, so the new
  module is declared exactly once (`lib.rs:51`).
- `CHANGELOG.md` — merged by git; both sides' entries under `[Unreleased]`
  are kept.

One more thing `main` moved onto us: `main` took migration number `0127`
(`0127_query_history_stopped.sql`), and this branch's migration was
`0127_email_settings.sql`. Two migrations cannot share a version, so ours
is renumbered to `0128_email_settings.sql` — the plan says to take the
next free number on `main` at build time and to renumber before the merge
when `main` moved. The migration is not applied anywhere yet, so nothing
was edited in place; the store module's doc comment now says `0128`, and
the CHANGELOG entry names `0128`.

**R1 — the fix.**

The connection fields — the mail server, the port, the security, the
account and the password — are now ONE unit
(`runtime_settings.rs`, `resolve_email`): when the environment sets any
one of `SMTP_HOST`, `SMTP_USER` or `SMTP_PASS`, all five are the server's.
The resolver then uses only the environment's values for them (the code
default where the environment names none), the GET reports them `server`,
and a PUT that sends any of them is the existing 409 — the GET and the PUT
read the sources from the resolver, so both follow from the one rule.
`from` and `consolePublicUrl` stay per field: they are not part of the
login and cannot receive a secret.

With the rule in place, the hole the finding names is closed: with
`SMTP_PASS` set and `SMTP_HOST` unset, the effective host is the
environment's — none — so the console's saved host is **not** used for a
send, and the test button answers "Email is not set up: the mail server
is missing." instead of logging in to the console's host with the
operator's password.

Two consequences of the rule that the code answers honestly, and that the
feature page already describes (the planner added the limit to
`docs/core/features/console-settings.md`):

- The Deployment tab names a missing mail server `SMTP_HOST` (not
  `host`) when the environment owns the connection — the answer's rule is
  "the environment variable for a setting only the server owns, the
  console's own field name for a setting the console can set"
  (`routes/settings.rs`, `deployment_features`).
- `docs/CONFIGURATION.md` and `docs/OPERATIONS.md` said the environment
  wins field by field, and that the console's host is settable while
  `SMTP_HOST` is unset. Both now say the connection is one unit.

**Tests added** (the finding names the first one):

- `a_save_of_a_connection_field_is_refused_when_the_environment_owns_the_connection`
  (`tests/email_settings_route.rs`) — env `SMTP_PASS` only: a PUT of
  `host` is 409, nothing is written, and the GET reports all five fields
  as `server`.
- `a_saved_console_host_is_not_used_when_the_environment_owns_the_connection`
  (`tests/email_settings_route.rs`) — a row saved straight into the store
  (as a row saved before the operator added `SMTP_PASS` looks) is not used
  for a send: the GET reports the environment's values, and the test
  email answers "Email is not set up: the mail server is missing."
- `the_environments_connection_is_one_unit`,
  `the_from_address_and_the_console_address_stay_per_field`,
  `the_console_owns_the_connection_when_the_environment_says_nothing`
  (`runtime_settings.rs`'s own tests) — the unit rule, the two fields
  that stay per field, and the case the finding names as unchanged (env
  none → the console's five fields are used, as before).
- `a_mail_server_the_environment_owns_is_named_the_way_the_operator_sets_it`
  (`tests/deployment_settings_route.rs`).

**Mutation check (R1).** Done and restored: with the unit rule dropped
(`server_owns_connection` forced to false) three tests fail —

- `a_save_of_a_connection_field_is_refused_when_the_environment_owns_the_connection`
  (the answer is 200 instead of 409, and the console's host was written),
- `a_saved_console_host_is_not_used_when_the_environment_owns_the_connection`
  (the host is the console's, not the environment's),
- `runtime_settings::tests::the_environments_connection_is_one_unit` (the
  console's host is used).

The rule was put back and all three pass again.

**Commands run, with their counts** (foreground, on the branch's head,
`CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target-adm67`,
`CARGO_BUILD_JOBS=2`):

- `cargo fmt --check` — clean.
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings` — clean.
- `cargo test -p lakehouse-api --lib runtime_settings` — **9 passed**
  (6 before, 3 new).
- `cargo test -p lakehouse-api --test email_settings_route` — **13 passed**
  (11 before, 2 new).
- `cargo test -p lakehouse-api --test deployment_settings_route` — **6
  passed** (5 before, 1 new).
- `cargo test -p lakehouse-api --test route_auth` — **80 passed**.
- `cargo check -p lakehouse-api` (on the merged tree, before R1) — clean.
- `bun run typecheck` — clean.
- `bun test src/features/admin` — **77 passed, 0 failed** (12 files).

**Not verified:**

- The workspace-wide block (`cargo clippy --workspace --all-targets
  --all-features -- -D warnings` and `cargo test --workspace`) was NOT
  run on this branch: the task scoped the checks to `lakehouse-api`, and
  CI runs the rest on the pull request. The merge brought in `main`'s code
  unchanged, and `main` is green in CI.
- The Dagster suite (`pytest dispar_orchestrate`), a real `docker compose
  up` and `docker compose --profile '*' config --quiet` were not run:
  these two commits changed no Python and no compose file.
- The console's own screens were not looked at in a browser. The console's
  contract did not change — the sources are still `server`, `console` and
  `default`, and the console already disables a field the server owns and
  leaves it out of the save body — so no console code was touched. What is
  new is that the API may report five fields as `server` at once.
- R3 was accepted by the planner and needed no code; the limit is in the
  feature page's Limits.

## 5. Review

### Review of slice 1 (planner, 2026-11-06, at `b1d022fc`)

Checked: the migration; `runtime_settings.rs` (`resolve_email`, the cache, the open-once
logging); `routes/settings.rs` (`put_email`, `password_field`, `email_row_to_save`,
`audit_email_update`, `post_email_test`, `test_email_sentence`); the policy rows; the
config flags. The 409 loop, the absent/null/string password rule, the audit row (field
names only), the recipient (the caller's own account address, body never read) and the
fixed sentences match D1–D10.

- **R1 (BLOCKER) A server-owned password can be sent to a console-chosen server.**
  `resolve_email` merges per field. With `SMTP_PASS` (or `SMTP_USER`) set in the
  environment and `SMTP_HOST` unset, the host source is `default`, so an administrator may
  save a host of their choosing, and every send (and the test button) then logs in to that
  host with the environment's user and password. That hands a secret the operator kept in
  files to whoever holds `settings:write`, which is what D1 ("the server file wins") exists
  to prevent. Fix: the connection fields are one unit. When the environment sets any of
  `SMTP_HOST`, `SMTP_USER` or `SMTP_PASS`, then host, port, security, user and password
  are all `server`: the resolver uses only the environment's values for them, the GET
  reports them `server`, and a PUT that sends any of them is the existing 409. `from` and
  `consolePublicUrl` stay per field. Tests: env `SMTP_PASS` only → a PUT of `host` is 409
  and nothing is written; a stored console host is NOT used for a send when env
  `SMTP_PASS` is set (the effective host is the environment's, here none, so email is not
  set up); env none → the console's five fields are used as today. Mutation check: drop
  the unit rule, the first test fails. Cite `ADM-7 review R1`.
- **R2 (BLOCKER, mechanical) Merge `origin/main`.** `REL-20` (#397) removed every `mod`
  line from `src/main.rs`; the module tree lives only in `src/lib.rs`. This branch adds
  `mod runtime_settings;` to both. Merge `origin/main` (no rebase), keep the declaration in
  `lib.rs` only, resolve hunk by hunk, and say what conflicted. CI now runs nextest in
  three partitions; the sum floor is 6,826 and this branch only adds tests.
- **R3 (accepted, documented) The mail server may be an internal address.** The host is
  checked as a hostname only, so the test button can dial internal hosts and its sentence
  says whether something answered. That is needed (on-premises mail relays are usually
  internal), the route needs `settings:write`, and it is limited to one try per 10 s per
  person. Written into the feature page's Limits; no code change.

### Review of slice 2 (planner, 2026-11-06, at `ccafe9e9`)

Checked: `resolve_sign_in` and its readers (`AppState::throttle_policy` → `routes/auth.rs`,
the cleanup sweep in `lib.rs`, both embed checks in `routes/embed.rs`); `resolve_assistant`,
`assistant()` (rebuild only on change, the address check before a console address is used)
and every caller (`routes/ai/mod.rs`, `agents.rs`, `pipelines.rs`, `semantic.rs`,
`settings.rs`); `check_assistant_address` (https, host, `resolve_checked`); `put_assistant`
(409 loop, the key sealed, the URL-cleared-while-a-key-is-set refusal). The slice-1 R1
lesson is applied: an environment key makes url, model and key all `server`.

- **R4 (BLOCKER) The environment's `LLM_URL` and `LLM_MODEL` do not win when it sets no
  key.** `resolve_assistant` only checks `llm_key`; with `LLM_URL` or `LLM_MODEL` set and no
  key, the console's value replaces the operator's and the GET calls the environment's value
  `default`. That breaks D1 ("the server file wins", shown read-only) and the feature page.
  Fix like slice 1's `smtp_port_from_env`: `Config` records whether `LLM_URL` / `LLM_MODEL`
  were set and non-empty; each is then `server` on its own (409 on PUT, read-only), and the
  existing key rule stays (an environment key makes all three `server`). Tests: env
  `LLM_URL` only → PUT `url` 409, nothing written, GET source `server`; same for
  `LLM_MODEL`; env none → console values used. Mutation check on the first. Cite
  `ADM-9 review R4`.
- **R5 (accepted, documented) The address check is not pinned to the dial.** The console's
  address is resolved and checked when it is saved and when the client is rebuilt, but the
  client then resolves the name again when it dials, so a name whose DNS answer changes
  afterwards (rebinding) is not caught. The route needs `settings:write`, and the assistant
  client has no pinned-address mode today. Written into the feature page's Limits; a pinned
  dialer would be its own task.
- **Missing: the slice 2 handoff.** The developer's session ended before it was written.
  The fix round must add "Handoff (slice 2)" with the commands and counts, the migration
  numbers (`0129`, `0130`) and the mutation checks done.
