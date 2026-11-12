# QA round, Administration and security (P2): the audit actor, unknown methods, sessions, who runs a rule, a table's page by a grant

Fixes for five backlog rows opened or decided in the QA round on staging (2026-11-02 to
11-12): `SEC-70` (under `SEC-2`), `SEC-72` (`DEC-19`), `ADM-14` (under `SEC-5`, `SEC-2`),
`GOV-10` (`DEC-17`, under `DATA-14`) and `GOV-11` (`DEC-20`, under `DATA-15`, fits `GOV-7`).

Base: `main` at `75010df81` (every anchor below was read there; the backlog rows were read at
`657c2502`, and nothing under `rust/` or `src/` changed between the two). Branch:
`fix/qa-round-admin-p2`. One pull request for the whole slice, opened as a draft by the
planner as soon as the Rust tasks are pushed, so CI (x86_64) runs every Rust test.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer pushes
after every commit, and does not change a decision below: if the code does not match what a
task says, stop and report the mismatch in the handoff.

The owning features exist (`docs/core/features/login-protection-and-session-cleanup.md`,
`data-quality-in-depth.md`, `asset-page-depth.md`, `table-access-per-role.md`). `DEC-17`,
`DEC-19` and `DEC-20` are the product owner's (2026-11-12). The other decisions below were
made by the planner on the owner's brief; T12 names every feature-page line that changes.

What QA saw:

- `LH-SEC-2-06`: a sign-in lockout is listed in the Audit Log with actor "AI Copilot",
  kind Agent, source `copilot`.
- `LH-BI-13-02`, `LH-BLD-2-03`, `LH-DATA-14-04`, `LH-RPT-1-11`: `500 route_policy_unclassified`
  for a method no route has (`POST /api/dashboard`, `GET` and `DELETE` of a pipeline's target
  lag, `POST` of quality incidents, `PATCH` of a report subscription), and `500 No matched
  path found` for an address that does not exist (`PATCH`/`PUT` of one quality incident).
- `LH-SEC-5-01`: Expires says "just now" for every session; IP and User agent are always "—";
  six sessions for one administrator within twelve seconds.
- `LH-SEC-2-02`: "Too many failed sign-in attempts. Try again later. Try again in about 5
  minutes."
- `LH-DATA-14-04`, `TC-1728`: a viewer ran a data-quality rule (200).
- `LH-DATA-15-03` F2: a person granted `query:read` on a table cannot open its page.
  `LH-DATA-12-07` F2: Data Engineer holds `catalog:write` without `catalog:read`.

## 1. Decisions

- **D1. One task per commit, in the order of section 2** (Rust first, then the console, then
  the documents). Each commit body cites its QA case IDs and its backlog ID. Each fix site
  carries a short comment naming the case (rule 13), for example
  `// QA LH-SEC-2-06 (SEC-70): …`.
- **D2. No new helper where one exists (rule 4).**
  - The assistant's name: `lakehouse_store::agents::COPILOT_EMPLOYEE_NAME`
    (`rust/crates/lakehouse-store/src/agents.rs:1119`).
  - A fixed `{"error": …}` body outside `ApiError`: the struct `timeout_middleware` already
    uses (`rust/crates/lakehouse-api/src/routes/mod.rs:1846-1849`, `:1873-1879`).
  - "In force and made from this asset": the predicate of `list_asset_grants`
    (`rust/crates/lakehouse-store/src/agents.rs:1838-1868`).
  - A person's id: `Principal::user_id` (`rust/crates/lakehouse-auth/src/principal.rs:140`).
  - Console permissions: `useAuth().hasPermission`
    (`src/features/auth/auth-provider.tsx:54`, `:169`).
  - Console times: `formatDateTime`, `isPast`, `formatTimeUntil`
    (`src/lib/format.ts:94`, `:141`, `:172`). No new formatter.
  - Test principals and sessions: `create_principal_with_permissions`,
    `create_zero_permission_principal`, `session_cookie_for_user`,
    `session_cookie_for_seeded_user`, `in_the_catalog_tenant`, `CATALOG_TENANT`
    (`rust/crates/lakehouse-api/tests/common/mod.rs:363-411`, `:463`, `:532`).

### SEC-70, an audit event nobody signed in caused

- **D3. No new principal kind, and no migration.** `audit_event.principal_kind` may only be
  `user`, `service`, `copilot` or `schedule` (`rust/migrations/0024_audit_event.sql:46-47`),
  and none of the four is true for an event that happened before anybody signed in. Such an
  event keeps `principal_id` and `principal_kind` empty (that is the truth) and says what
  caused it in `actor_label`. Three writers set no principal today, found by reading every
  `NewAuditEvent` in the workspace:

  | Action | Writer | `actor_label` it now sets |
  | --- | --- | --- |
  | `auth.login_locked` | `rust/crates/lakehouse-api/src/routes/auth.rs:248-254` | `Sign-in form` |
  | `dashboard.public_link_locked` | `rust/crates/lakehouse-api/src/routes/public_links.rs:447-458` | `Public link` |
  | `auth.oidc.link_refused` | `rust/crates/lakehouse-auth/src/oidc.rs:1332-1350` | `Single sign-on` |

  Every other writer sets a principal kind. `audit_table_change` and `audit_policy_change`
  (`routes/governance.rs:969-997`, `:1351-1382`) take an optional actor, but every caller
  passes the request's principal, which the gate always sets on those routes; they are not
  changed.
- **D4. The list never shows an event with no principal as the assistant's.** In
  `routes/governance.rs`, for an event with no `principal_kind`: `source` is `"system"` (was
  `"copilot"`), `actorKind` is `"service"` (was `"agent"`; the console's kinds are a person,
  a platform service, or an agent, `src/lib/status.ts:279-286`, and the platform recorded the
  event), and the actor is the stored `actor_label`, or `"System"` when there is none (rows
  stored before this change). "AI Copilot" stays the name of an event whose kind is
  `copilot` and that carries no name (`AI-23`). An event that has a principal keeps the
  source #456 gave it: `user` is `console`, `copilot` is `copilot`, `schedule` is `schedule`,
  `service` is `service`. Rows already stored are not rewritten (the table is append-only);
  the list reads them right from now on.
- **D5. Out: the approval route.** A refused self-approval (403) and a second decision (409)
  leave no audit entry (`routes/agents.rs:640-651`). That is backlog `AI-28`, planned by the
  AI team.

### SEC-72, a method no route has (`DEC-19`)

- **D6. Where the two cases are told apart: the order of two calls on the router.** Read in
  `axum` 0.8.9 (`rust/Cargo.lock:535-536`): `Router::layer` wraps every method handler of a
  path and the path's own "no handler for this method" fallback
  (`routing/method_routing.rs`, `MethodRouter::layer`, `fallback: self.fallback.map(…)`), which
  is why `auth_gate` sees such a request today and answers 500. But
  `Router::method_not_allowed_fallback(handler)` replaces that fallback on every path that
  still has the default one (`routing/path_router.rs:116-126`, `default_fallback`), and a
  fallback set **after** `.layer(auth_gate)` is not wrapped by it. So:
  - `.layer(auth_gate)` first: every handler stays behind the gate, and a handler with no
    `POLICY_TABLE` entry still answers `500 route_policy_unclassified` (principle 3,
    unchanged; `policy.rs:1304-1311` is not touched).
  - then `.method_not_allowed_fallback(…)`: a path that exists with no handler for the
    method answers 405 without reaching the gate. Nothing runs, so nothing is opened.
  - then `.fallback(…)`: a path no route matches answers 404. Today the gate's `MatchedPath`
    extractor fails there and axum answers `500 No matched path found`.
  - A path registered with `axum::routing::any` (the Iceberg door,
    `routes/iceberg_door.rs:716-740`) has its own fallback, which is not the default one, so
    it is not replaced and stays behind the gate.
- **D7. The answers.** Both are the usual `{"error": "<sentence>"}` body.
  - 405: `This method is not allowed on this address.`
  - 404: `No such address.`
  - No new `ApiError` variant: the 405 is built where `timeout_middleware` builds its 408
    (same file, same body struct); the 404 is `ApiError::NotFound`.
- **D8. What still answers 500.** A handler that exists and is missing from `POLICY_TABLE`.
  That includes `HEAD` on a path that serves `GET`: axum answers `HEAD` with the `GET`
  handler, so a handler exists and `("HEAD", path)` is not in the table. It is not a QA case
  and is left as it is (section 4).

### ADM-14, the Sessions page and the lock message

- **D9. The browser is recorded at sign-in.** Both places that create a session
  (`routes/auth.rs:212-220`, password; `:1165-1173`, single sign-on) pass the request's
  `User-Agent` header, trimmed, cut to 512 characters, and nothing when the header is absent,
  empty or not plain ASCII. It is what the client says about itself; it is shown, never
  trusted.
- **D10. The network address is not recorded in this slice.** The brief was to reuse the way
  the code already derives a client address. There is none: the sign-in lock is per account
  (`throttle::key_for(&identifier)`, `routes/auth.rs:172`), no code reads a forwarded header or
  the peer address, and the feature page says why ("The API sits behind the console's own
  server, so it cannot see the real client address reliably",
  `docs/core/features/login-protection-and-session-cleanup.md:38-39`). Read in `next` 16.4.3
  (`dist/server/lib/router-utils/proxy-request.js`): the console's `/api` rewrite adds only
  `x-forwarded-host`; an `X-Forwarded-For` the client typed passes through unchanged. So the
  peer address is the console's, and the header is whatever the client sent. Recording
  either would show a wrong address as a fact (rule 14). `created_ip` stays empty, the page
  says "Not recorded" instead of "—", and the feature page says so. Question 1 asks the owner
  for the one thing that would make it recordable.
- **D11. Session times travel as RFC 3339 in UTC.** `GET /api/auth/sessions` sends
  `created_at::text` and `expires_at::text` (`rust/crates/lakehouse-store/src/sessions.rs:96-99`),
  that is `2026-11-12 09:42:13.123456+00`, a shape `Date` parses only by a browser's own
  leniency. They become `2026-11-12T09:42:13.123Z`, which every browser reads and which the
  console's own tests already assume (`src/features/admin/sessions-page.test.tsx:74-75`).
- **D12. "Expires" shows the time and the time left.** First line `formatDateTime(expiresAt)`;
  second line, muted, `formatTimeUntil(expiresAt)` ("in 23h"), or "expired" when `isPast`.
  "Created" stays `formatRelativeTime` (right for a past time). The cause of "just now" is
  `formatRelativeTime` given a future time (`src/lib/format.ts:158`); that helper is not
  changed.
- **D13. The lock message is one sentence, written by the console.** With a wait:
  `Too many failed sign-in attempts: try again in about 5 minutes.` Without one, the
  server's sentence as it is. The server's sentence
  (`routes/auth.rs:129`) and its `Retry-After` header do not change, so no Rust test changes;
  the console already owns its sentence for a 401 in the same handler
  (`src/features/auth/login-page.tsx:155-156`).
- **D14. Six sessions in twelve seconds: the code does not explain it as one sign-in.** One
  successful `POST /api/auth/login` creates exactly one session, and so does one single
  sign-on callback; those two are the only callers of `session::create_session` outside
  tests. The console sends the request once (no retry in `src/services/clients/auth.ts:58-71`
  or `src/services/http.ts`; the button is disabled while it runs, `login-page.tsx:345-348`),
  and nothing refreshes or replaces a session. A new sign-in never ends an earlier one, so
  six rows are six successful sign-ins, each live for 24 hours
  (`rust/crates/lakehouse-auth/src/session.rs:46`). Scripted sign-ins during QA would look
  exactly like this, but that is a guess: nothing recorded tells them apart today. From T4
  on each row carries the browser, so the next occurrence can be read. T4's test pins "one
  sign-in, one session".

### GOV-10, who runs a rule by hand (`DEC-17`)

- **D15. `POST /api/governance/quality/{id}/run` needs `governance:write`** (was `query:read`,
  `rust/crates/lakehouse-api/src/policy.rs:389-391`). Only the policy row changes; the handler
  (`routes/quality.rs:1027-1048`) and the scheduled and on-load checks, which call
  `quality::evaluate` without the route, do not.
- **D16. No migration.** Built-in roles after every migration: Governance Admin holds
  `governance:write` (`rust/migrations/0030_table_maintenance_policy.sql:50-53`) and Platform
  Admin holds `*:*`; both can run a rule, and both can author one. Analyst and Data
  Scientist (`query:read`) lose the action: that is the decision (`LH-DATA-14-04`: a viewer
  ran a rule). Governance Admin holds no `query:read`, so it could **not** run a rule before
  and can now. Data Engineer could not before and cannot now.
- **D17. The console shows Run only with `governance:write`**, at the four places it runs a
  rule (T10). No new text.

### GOV-11, a table's page by a grant, and Data Engineer (`DEC-20`)

- **D18. The rule.** `GET /api/catalog/{id}` admits a caller who holds `catalog:read`, as
  today, **or** a person who holds `query:read` and has a grant of `query:read` that is in
  force (not revoked, not expired) and was made from exactly that asset: a direct grant whose
  `catalog_id` is `{id}`, or the grant of an approved request whose `resource` is `{id}`,
  the same two origins the asset's Grants card lists. Everyone else gets the 403 they get
  today, with the same body (`permission_denied: catalog:read`), before the asset is looked
  up, so the refusal says nothing about whether the id exists. A grant on table A opens A and
  nothing else: table B answers 403.
- **D19. The policy row becomes `RequiresAuth` and the handler decides.** The table cannot
  say "this permission, or a grant on this id". `RequiresAnyPermission(["catalog:read",
  "query:read"])` would be untrue: `query:read` alone is not enough, and
  `tests/route_auth.rs:223-266` asserts that each permission of such a row passes alone.
  `RequiresAuth` with the level resolved in the handler is the pattern the dashboard read
  routes already use (`policy.rs:623-626`). The check is one named function, the first
  statement of `catalog::detail`, asserted both ways in `tests/route_auth.rs`.
- **D20. What the admitted person sees.** The same body a `catalog:read` reader gets for
  that table, built by the same code as that person: the tenant gate, the role's table access
  (`GOV-7`: a table the person's roles hide still answers 404, because a grant gives a
  permission, not a table), masks and row filters on the sample. Unchanged and still on their
  own permission: `GET /api/catalog`, `/api/catalog/query` and search (`catalog:read`), the
  annotation, the restore status, the access request, the storage and meaning reads
  (`catalog:read`), lineage (`lineage:read`), grants (`access:approve`). `query:read` itself
  stays global, as `routes/catalog_grants.rs:10-18` says.
- **D21. The console lets such a person reach the page.** `/data/assets/[assetId]` requires
  `catalog:read` **or** `query:read` (`src/components/app-shell/nav-config.ts:299`); the API
  decides, and a refusal shows as "no access", not "not found"
  (`src/services/clients/assets.ts:114-123`). `/catalog` and the sidebar entry stay
  `catalog:read`.
- **D22. Data Engineer gains `catalog:read` by a migration.** The role that holds
  `catalog:write` (`0002_seed_identity.sql:43`) cannot read what it edits; `0031` fixed the
  same shape for Governance Admin. Only the role named `Data Engineer` that does not hold the
  token is touched; a renamed or custom role is not.

### Text, and the assistant

- **D23. Fixed text (English, as the console).**
  - Audit actors: `Sign-in form`, `Public link`, `Single sign-on`, `System`.
  - 405: `This method is not allowed on this address.` 404: `No such address.`
  - Sessions page: `Not recorded`; `expired`.
  - Lock: `Too many failed sign-in attempts: try again in about N minutes.` (`1 minute`
    for one).
- **D24. The assistant: no tool, schema or page-context change.** No dashboard behaviour
  changes, and no assistant tool runs a quality rule (`run_rule` is used only by its route).
  `tests/fixtures/tool_schemas.json` and `tests/fixtures/permissions.json` are not
  regenerated: no permission is added or renamed. A Data Engineer's assistant can use the
  read tools that take `catalog:read` after T6, by the same permission as the pages.

## 2. Tasks

Rust first (T1 to T7), then the console (T8 to T11), then the documents (T12). All Rust
paths below are under `rust/crates/`.

### T1. An event nobody signed in caused says what caused it (LH-SEC-2-06; SEC-70)

- Anchors: `lakehouse-store/src/audit.rs:62-64` and `:154-155` (the `actor_label` docs);
  the three writers of D3; the tests
  `lakehouse-api/tests/login_throttle.rs:189-219`
  (`the_lock_sets_exactly_one_audit_event_naming_neither_the_email_nor_the_password`),
  `lakehouse-api/tests/public_links.rs:520-563`
  (`five_wrong_passwords_lock_the_link_and_a_locked_link_says_so`),
  `lakehouse-auth/tests/oidc.rs:624-676`
  (`an_unconfirmed_email_is_refused_for_an_existing_account`).
- Change:
  1. In `lakehouse-store/src/audit.rs`, after `AUDIT_COLUMNS`, add three documented `pub const`
     strings: `ACTOR_SIGN_IN_FORM = "Sign-in form"`, `ACTOR_PUBLIC_LINK = "Public link"`,
     `ACTOR_SINGLE_SIGN_ON = "Single sign-on"`. One doc comment above them says the rule of
     D3: an event nobody signed in caused has no `principal_id` and no `principal_kind`, and
     names what caused it in `actor_label`.
  2. Change both `actor_label` doc comments to: "Display name of the principal, or, for an
     event nobody signed in caused, what caused it (see `ACTOR_SIGN_IN_FORM`)."
  3. At each of the three writers add one field,
     `actor_label: Some(<the constant>.to_owned()),`. Do not set `principal_id` or
     `principal_kind`. Nothing else in the events changes (`resource_id` stays the 12 hex
     characters, `args` stay as they are).
- Check: extend the three tests above, each in its own file. Read `actor_label`,
  `principal_kind` and `principal_id` of the row the test already counts and assert the
  label is the constant's text and the other two are `NULL`. Each fails today (the label is
  `NULL`).

### T2. The audit list never gives such an event to the assistant (LH-SEC-2-06; SEC-70)

- Anchors: `lakehouse-api/src/routes/governance.rs:304-318` (`actor_kind_of`), `:320-330`
  (`source_of`), `:376-388` (the doc of `audit`, which lists the sources), `:436-449` (the
  row), `:2927-2946` (the two unit tests).
- Change:
  1. `actor_kind_of`: `Some("user")` is `"user"`; `Some("copilot" | "schedule")` is
     `"agent"`; everything else, including `None`, is `"service"`. Reword its doc comment to
     say why `None` is a service (D4).
  2. `source_of`: add the arm `Some("copilot") => "copilot"` and make the last arm
     `_ => "system"`. Reword its doc comment: no recorded kind is `"system"`, never the
     assistant.
  3. Add a pure function beside them,
     `fn actor_of(label: Option<&str>, principal_id: Option<&str>, principal_kind: Option<&str>) -> String`:
     the label; else the principal id; else `COPILOT_EMPLOYEE_NAME` when the kind is
     `Some("copilot")` (`AI-23`); else `"System"` (a `const UNKNOWN_ACTOR: &str` beside it).
     Use it for `"actor"` at `:438` and delete the inline chain and its literal.
  4. In the doc of `audit` add `"system"` to the list of sources.
- Check:
  - Unit tests in the same `tests` module. Change
    `actor_kind_of_narrows_copilot_principal_kinds_to_agent`: its last line asserts
    `actor_kind_of(None)` is `"service"`. Change `source_of_follows_the_principal_kind`: its
    last line asserts `source_of(None)` is `"system"`. Both lines were written for the
    behaviour this task corrects; say so in the commit (rule 2). Add
    `actor_of_names_the_label_then_the_id_then_the_assistant_only_for_a_copilot_event`
    (four cases: a label wins; an id when no label; `"AI Copilot"` for kind `copilot` with
    neither; `"System"` for no kind with neither).
  - Integration, in `lakehouse-api/tests/login_throttle.rs`:
    `a_lock_is_listed_in_the_audit_log_as_the_sign_in_forms_and_not_the_assistants`. Lock
    the account as the test at `:189` does, then `GET /api/governance/audit` with
    `session_cookie_for_seeded_user(&pool, "fajar@meridian.example")`. Expect 200, and for
    the one entry of `body["audit"]` whose `action` is `auth.login_locked`: `actor` is
    `"Sign-in form"`, `actorKind` is `"service"`, `source` is `"system"`. Fails today on all
    three.

### T3. A method no route has answers 405, an unknown address 404 (LH-BI-13-02, LH-BLD-2-03, LH-DATA-14-04, LH-RPT-1-11; SEC-72)

- Anchors: `lakehouse-api/src/routes/mod.rs:1273` (`pub fn router`), `:1758-1774` (the comment
  and the three `.layer` calls and `.with_state`), `:1841-1849` (`TimeoutBody`), `:1873-1879`
  (its one use), `:2140-2172` (the doc of `route_policy_tests`), `:2296-2319`
  (`an_unregistered_route_denies_by_default_instead_of_serving_public`);
  `lakehouse-api/src/lib.rs:1675-1695` (`removed_agent_endpoints_answer_like_an_unknown_path`);
  `lakehouse-api/src/policy.rs:1297-1311` (read only).
- Change, all in `routes/mod.rs`:
  1. Rename `TimeoutBody` to `FixedErrorBody` (the definition and its one use) and reword
     its doc comment to name both users: the request timeout and the 405 below.
  2. Add two constants and two handlers beside `timeout_middleware`:

     ```rust
     const METHOD_NOT_ALLOWED: &str = "This method is not allowed on this address.";
     const NO_SUCH_ADDRESS: &str = "No such address.";

     async fn method_not_allowed() -> Response {
         (
             StatusCode::METHOD_NOT_ALLOWED,
             ApiJson(FixedErrorBody { error: METHOD_NOT_ALLOWED.to_owned() }),
         )
             .into_response()
     }

     async fn no_such_address() -> crate::error::ApiRejection {
         ApiError::NotFound(NO_SUCH_ADDRESS.to_owned()).into()
     }
     ```

     Each gets a doc comment: what it answers, `QA … (SEC-72, DEC-19)`, and that it runs
     outside `auth_gate` on purpose because it serves nothing.
  3. Move the tail of `router` into a new private function and call it. `router` keeps
     every `.route` and `.merge` exactly as it is, bound to a local, and ends with
     `gated(routes, state)`:

     ```rust
     fn gated(routes: Router<AppState>, state: AppState) -> Router {
         routes
             .layer(from_fn_with_state(state.clone(), crate::query_limits::scope_middleware))
             .layer(from_fn_with_state(state.clone(), auth_gate))
             .method_not_allowed_fallback(method_not_allowed)
             .fallback(no_such_address)
             .layer(from_fn_with_state(state.clone(), timeout_middleware))
             .with_state(state)
     }
     ```

     The order of these six calls is the fix: do not reorder them. Move the existing
     comments about `auth_gate`, `scope_middleware` and the timeout with their lines, and add
     a comment above the two fallbacks that says what D6 says in four lines: they come
     after `.layer(auth_gate)` so that a handler stays behind the gate (and still answers
     500 when it is not in `POLICY_TABLE`), while a method no handler serves and a path no
     route matches never reach it; an `any()` route keeps its own gated fallback.
  4. `auth_gate` and `POLICY_TABLE` are not changed.
  5. Comments that would now be untrue (rule 1): the doc of `route_policy_tests`
     (`:2140-2172`) and the comment in `lib.rs:1675-1683`, which both say an unknown method or
     path answers 500. Reword them to the new rule. The assertion in `lib.rs` stays as it is
     (both sides are now 404).
- Check:
  - In `route_policy_tests` (`routes/mod.rs`), replace
    `an_unregistered_route_denies_by_default_instead_of_serving_public` by
    `a_handler_that_is_not_in_the_policy_table_answers_500_and_is_never_served`: build
    `super::gated(Router::new().route("/health", get(super::health)).route("/__sec72_unclassified", get(super::health)), state)`
    with the state `test_router` builds, request `GET /__sec72_unclassified`, expect 500 and
    a body containing `route_policy_unclassified`. The old test proved deny-by-default with
    `DELETE /health`, a method no handler serves, which `DEC-19` turns into 405; the new one
    proves it with a real handler. Say so in the commit (rule 2). The throwaway path must
    not start with `/api/` and must be the last `.route` of that chain:
    `every_registered_route_has_a_policy_entry` (`tests/route_auth.rs:2175`) reads this
    file's `.route(` calls.
  - Add `a_method_no_handler_serves_answers_405_with_the_fixed_sentence`: on
    `test_router()`, with no credentials, each of `DELETE /health`, `POST /api/dashboard`,
    `GET /api/pipelines/x/target-lag`, `DELETE /api/pipelines/x/target-lag`,
    `POST /api/governance/quality/incidents`, `PATCH /api/reports/subscriptions/x` answers
    405 with the body `{"error":"This method is not allowed on this address."}`, parsed as
    JSON and compared whole.
  - Add `an_address_no_route_matches_answers_404_with_the_fixed_sentence`:
    `GET /api/never-existed`, `PATCH /api/governance/quality/incidents/x` and
    `PUT /api/governance/quality/incidents/x` answer 404 with `{"error":"No such address."}`.
  - Add `a_route_registered_for_any_method_stays_behind_the_gate`:
    `POST /api/iceberg/v1/config` with no credentials answers 401, not 405. It passes today
    too; it guards D6's last point.
  - In `lakehouse-api/tests/route_auth.rs`, add
    `a_signed_in_person_gets_405_for_a_method_no_route_has_and_404_for_an_unknown_address`:
    the same six and three requests with a Platform Admin session (a fresh
    `session_cookie_for_user` per request), same statuses and bodies. Fails today (500).
  - Report in the handoff whether the 405 carries an `Allow` header. Do not add one by hand
    and do not assert on it.

### T4. A sign-in records the browser; session times are RFC 3339 (LH-SEC-5-01; ADM-14)

- Anchors: `lakehouse-api/src/routes/auth.rs:158` (`login`), `:212-220`, `:441-448` (the
  "documented gap" paragraph), `:1007-1011` (`oidc_callback` already takes `headers`),
  `:1165-1173`, `:2857` (the test
  `oidc_callback_with_a_matching_state_and_nonce_creates_a_session_and_redirects`, request
  built at `:2919-2931`); `lakehouse-store/src/sessions.rs:24-28`, `:50-71`, `:96-99`;
  `lakehouse-store/tests/sessions.rs:230-234`; `lakehouse-api/tests/login_throttle.rs:48-62`
  (the `login` helper), `:79-94` (`seed_password_identity`).
- Change:
  1. `routes/auth.rs`: add `const SESSION_USER_AGENT_MAX_CHARS: usize = 512;` and

     ```rust
     fn session_user_agent(headers: &HeaderMap) -> Option<String> {
         let raw = headers.get(header::USER_AGENT)?.to_str().ok()?.trim();
         if raw.is_empty() {
             return None;
         }
         Some(raw.chars().take(SESSION_USER_AGENT_MAX_CHARS).collect())
     }
     ```

     with a doc comment (D9: what the client says about itself, shown and never trusted;
     `QA LH-SEC-5-01 (ADM-14)`).
  2. `login` takes the headers: `State(state): State<AppState>, headers: HeaderMap, body: Bytes`
     (the body stays the last argument). At both `create_session` calls pass
     `session_user_agent(&headers).as_deref()` as the user agent. The address argument stays
     `None`, with a one-line comment at each site: no address, because the API has no
     trusted source for it (D10).
  3. Reword the comments that say both columns are always `null`: `auth.rs:441-448`,
     `sessions.rs:24-28` and the two field docs at `:63-71`,
     `lakehouse-store/tests/sessions.rs:230-234`. New truth: the user agent is recorded at
     sign-in and is `null` for older sessions; the address is never recorded, and why.
  4. `list_sessions_for_caller` (`sessions.rs:96-99`): select
     `to_char(s.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at`
     and the same for `expires_at`, in place of the two `::text` casts (escape the inner
     quotes in the Rust literal). `ORDER BY s.created_at DESC, s.id` stays on the column.
     Reword the two field docs at `:50-62`: RFC 3339, UTC, milliseconds.
- Check:
  - Unit tests in `routes::auth::tests`:
    `the_session_user_agent_is_the_header_trimmed_and_cut_to_512_characters` (a normal
    value comes back as it is; surrounding spaces are trimmed; a 700-character value comes
    back 512 long) and
    `the_session_user_agent_is_nothing_for_an_absent_empty_or_non_ascii_header`.
  - `lakehouse-api/tests/login_throttle.rs`:
    `a_sign_in_records_the_browser_and_creates_exactly_one_session`. Seed the identity,
    sign in once with the right password and the header `user-agent: QA-Browser/1.0` (give
    the `login` helper an optional user agent, or add a second helper beside it), expect
    200, then `SELECT count(*), max(user_agent), max(created_ip) FROM session WHERE
    app_user_id = …` is `(1, Some("QA-Browser/1.0"), None)`. Fails today on the user agent.
  - Extend the single sign-on test at `auth.rs:2857`: add `.header(header::USER_AGENT,
    "QA-Browser/1.0")` to its request and, at its end, assert the one `session` row has
    that `user_agent` and a `NULL` `created_ip`.
  - `lakehouse-store/tests/sessions.rs`: `session_times_travel_as_rfc_3339_in_utc`. Insert
    one session with `created_at = '2026-11-12 09:42:13.123+00'` and
    `expires_at = '2999-01-02 03:04:05.678+00'`; the listed row has
    `created_at == "2026-11-12T09:42:13.123Z"` and `expires_at == "2999-01-02T03:04:05.678Z"`.
    Fails today.

### T5. Running a rule by hand needs `governance:write` (LH-DATA-14-04, TC-1728; GOV-10)

- Anchors: `lakehouse-api/src/policy.rs:389-391`; `lakehouse-api/src/routes/quality.rs:1015-1031`
  (the doc of `run_rule`); `lakehouse-api/tests/route_auth.rs:2523-2590`
  (`quality_incident_routes_are_gated_in_both_directions`, the pattern to follow).
- Change:
  1. The row becomes
     `("POST", "/api/governance/quality/{id}/run", Policy::RequiresPermission("governance:write")),`
     and the two comment lines above it become: running a rule by hand is a governance
     action like editing one (`GOV-10`, `DEC-17`, QA `LH-DATA-14-04`); it took `query:read`
     before; the scheduled and on-load checks do not pass through this route.
  2. Add to the `# Errors` of `run_rule`: `403` without `governance:write` (the policy gate).
  3. Nothing else: not the handler, not `quality::evaluate`, not a migration (D16).
- Check:
  - Unit test in `policy::tests`: `running_a_quality_rule_by_hand_needs_governance_write`
    asserts `policy_for("POST", "/api/governance/quality/{id}/run")` is
    `Some(Policy::RequiresPermission("governance:write"))`.
  - `lakehouse-api/tests/route_auth.rs`:
    `running_a_quality_rule_by_hand_is_gated_in_both_directions`, with
    `/api/governance/quality/00000000-0000-4000-8000-000000000000/run`: a principal holding
    `query:read,governance:read` gets 403; one holding `governance:write` alone gets 404
    ("Quality rule not found": past the gate, the rule does not exist); no cookie gets 401.
    Fails today (the first answers 404, the second 403).
  - Run `lakehouse-api/tests/quality_incidents.rs` once: every test in it runs a rule through
    this row, as a principal that holds `governance:write` (`:114`), so it must stay green
    unchanged.

### T6. Data Engineer gains `catalog:read` (LH-DATA-12-07 F2; GOV-11)

- Anchors: `rust/migrations/0031_governance_admin_catalog_read.sql` and
  `0033_connector_ingest_spec.sql:38-41` (the idiom), `0133_pages_per_role.sql:27-31` (the
  empty-string case); `lakehouse-store/tests/connector_ingest_spec.rs:30-65` (the tests of
  `0033`); `lakehouse-store/tests/identity.rs`; `lakehouse-api/tests/route_auth.rs:1234-1262`
  (`seeded_data_engineer_may_get_and_put_ingest_spec`).
- Change: one new migration, the next free number. The highest on `main` at `75010df81` is
  `0136_sso_trust_email.sql`, so `0137_data_engineer_catalog_read.sql`; but the sibling
  branch `fix/qa-round-connectors-p1` plans a `0137` too. Run `ls rust/migrations | tail -3`
  after fetching `origin/main`, take the next free number, and say in the handoff which
  number you took. If `main` gains that number before this merges, the planner will ask
  for a rename commit (the file is not applied anywhere yet). Content, after a why-header:

  ```sql
  UPDATE role
  SET permissions = CASE
      WHEN trim(permissions) = '' THEN 'catalog:read'
      ELSE permissions || ', catalog:read'
  END
  WHERE name = 'Data Engineer'
    AND permissions !~ '(^|,)\s*catalog:read\s*(,|$)'
    AND permissions !~ '(^|,)\s*(\*:\*|catalog:\*)\s*(,|$)';
  ```

  The why-header says: `GOV-11`, `DEC-20` (product owner 2026-11-12), QA `LH-DATA-12-07` F2;
  Data Engineer holds `catalog:write` since `0002` without `catalog:read`, so the role that
  edits catalog entries cannot open the catalog or a table's page; the same shape `0031`
  fixed for Governance Admin; only the role with this name that does not already hold the
  token (or a wildcard that covers it) is touched, so a renamed or custom role is left
  alone and a second run adds nothing.
- Check:
  - `lakehouse-store/tests/identity.rs`:
    `the_data_engineer_role_holds_catalog_read_exactly_once_even_when_the_migration_runs_again`
    (`#[sqlx::test(migrations = "../../migrations")]`). Read the role's `permissions`:
    `catalog:read` occurs once and `catalog:write` is still there. Then run the migration
    file again with `sqlx::raw_sql(include_str!("../../../migrations/<the file>"))` and
    read again: still once.
  - `lakehouse-api/tests/route_auth.rs`: `a_seeded_data_engineer_may_read_the_catalog`.
    With `session_cookie_for_seeded_user(&pool, "bayu@meridian.example")`,
    `GET /api/catalog` and `GET /api/lakehouse/tables` answer neither 401 nor 403 (403 today),
    and `POST /api/identity/roles` still answers 403.

### T7. A `query:read` grant on a table opens that table's page (LH-DATA-15-03 F2; GOV-11)

- Anchors: `lakehouse-store/src/agents.rs:1838-1868` (`list_asset_grants`), `:1885-1908`
  (`create_direct_grant`); `lakehouse-api/src/routes/catalog.rs:1750-1803` (`detail` and its
  doc), `:1809` (`SAMPLE_PERMISSION`), `:3252-3276` (`ensure_asset_readable`), `:3326`
  (`annotation_pool`); `lakehouse-api/src/policy.rs:241`, `:1319-1323` (the gate's 403);
  `lakehouse-api/src/routes/catalog.rs:6463` (`mod sample_masking`), `:6500` (`analyst`),
  `:6605` (`detail_as`), `:6871` (`detail_masks_sample_rows_and_flags_masked_columns`);
  `lakehouse-api/tests/catalog_grants.rs`; `lakehouse-api/tests/common/mod.rs:396-411`.
- Change:
  1. Store, right after `list_asset_grants`:
     `pub async fn holds_asset_grant(pool: &PgPool, user_id: Uuid, catalog_id: &str, permission: &str) -> Result<bool, StoreError>`,
     one bound query, documented with `# Errors`:

     ```sql
     SELECT EXISTS (
         SELECT 1 FROM access_grant g
         LEFT JOIN approval_item a ON a.id = g.approval_id
         WHERE g.user_id = $1 AND g.permission = $3
           AND g.revoked_at IS NULL AND g.expires_at > now()
           AND (g.catalog_id = $2 OR a.resource = $2)
     )
     ```

     The doc comment says it is the predicate of `list_asset_grants` for one person and one
     permission.
  2. `routes/catalog.rs`, beside `ensure_asset_readable`:
     `async fn ensure_may_open_asset(state: &AppState, principal: &Principal, id: &str) -> Result<(), ApiError>`:
     - `principal.has("catalog:read")`: `Ok(())`.
     - else, when `principal.has(SAMPLE_PERMISSION)` and `principal.user_id()` is `Some`:
       `holds_asset_grant(annotation_pool(state)?, user_id, id, SAMPLE_PERMISSION).await?`;
       `true` is `Ok(())`.
     - everything else: `Err(ApiError::PermissionDenied("catalog:read".to_owned()))`, the
       same variant and text the gate answers today.
     A store failure propagates through `?` (classified, fail closed). The doc comment
     states D18 and D20 and cites `GOV-11`, `DEC-20`, QA `LH-DATA-15-03` F2.
  3. `detail`: the first statement becomes
     `ensure_may_open_asset(&state, &principal, &id).await?;`, before the tenant gate.
     Nothing after it changes. Add the 403 to the doc's `# Errors`.
  4. `policy.rs:241`: `("GET", "/api/catalog/{id}",  Policy::RequiresAuth),` with a comment
     above it: the handler admits `catalog:read`, or a person's `query:read` grant made from
     exactly this asset, and refuses everyone else with the same 403
     (`catalog::ensure_may_open_asset`; `GOV-11`, `DEC-20`); the table cannot express "or a
     grant on this id". The rows for `/api/catalog` and `/api/catalog/query` are not touched.
- Check:
  - In `mod sample_masking` (`routes/catalog.rs`), next to the test at `:6871`:
    `detail_admits_a_person_granted_query_read_on_that_table_and_masks_what_they_read`. The
    person is `analyst("query:read")` with `id` set to the seeded user
    `33333333-3333-4333-8333-000000000005`; give them a grant with
    `lakehouse_store::agents::create_direct_grant(&pool, that id, "query:read",
    "serving.mart_x", the seeded user 33333333-3333-4333-8333-000000000006, 30)`.
    `detail_as` answers 200, `schema[1].masked` is `true` and the sample's `email` is `***`
    (the assertions of the test at `:6871`). Then call `detail` for `serving.mart_y` as the
    same person (build the state as `:6656-6661` does): 403. Then the same person with no
    grant row (use the seeded user `…000000000001` as `id`) on `serving.mart_x`: 403.
  - `lakehouse-api/tests/catalog_grants.rs`, with
    `spin_up_with_env` setting `CATALOG_TENANT_ID` to `common::CATALOG_TENANT` and the
    grantee put `in_the_catalog_tenant`:
    - `a_query_read_grant_on_a_table_opens_that_tables_page_and_no_other`: the admin grants
      `query:read` from `silver.orders` through `POST /api/catalog/silver.orders/grants`.
      As the grantee: `GET /api/catalog/silver.orders` is neither 401 nor 403 (no engine is
      reachable in this harness, so the expected answer is 503; put the status in the
      assertion message); `GET /api/catalog/silver.customers` is 403; `GET /api/catalog` is
      403 (the list is unchanged).
    - `a_revoked_or_expired_grant_no_longer_opens_the_page`: after `DELETE` of the grant,
      403; and for a second grant, after
      `UPDATE access_grant SET expires_at = now() - interval '1 second'`, 403.
    - `a_grant_of_another_permission_does_not_open_the_page`: a `lineage:read` grant from
      `silver.orders` leaves `GET /api/catalog/silver.orders` at 403.
  - `lakehouse-api/tests/route_auth.rs`: `the_asset_page_route_is_gated_in_both_directions`
    on `/api/catalog/silver.orders`: `catalog:read` alone is neither 401 nor 403; a
    zero-permission principal is 403; `query:read` alone (a role, no grant) is 403; no
    cookie is 401.
  - Run `lakehouse-api/tests/permission_catalog.rs` once (it reads `POLICY_TABLE` and every
    `.has("…")` literal); it must stay green unchanged.

### T8. The Sessions page shows a real expiry and the browser (LH-SEC-5-01; ADM-14)

- Anchors: `src/features/admin/sessions-page.tsx:8`, `:44-50`;
  `src/features/admin/sessions-page.test.tsx:67-89`; `src/services/contracts/auth.ts:12-14`.
- Change:
  1. Import `formatDateTime`, `formatTimeUntil` and `isPast` beside `formatRelativeTime`.
  2. The Expires cell is two lines (D12): `formatDateTime(r.expiresAt)`, and under it, in
     `text-xs text-muted-foreground`, `isPast(r.expiresAt) ? "expired" : formatTimeUntil(r.expiresAt)`.
  3. The IP cell is `r.createdIp ?? "Not recorded"`. The User agent cell is
     `r.userAgent ?? "Not recorded"`; a recorded value is one truncated line
     (`block max-w-md truncate`) with the full text in `title`.
  4. Reword the comment in `contracts/auth.ts:12-14`: `userAgent` is the browser at sign-in,
     `null` for a session older than that; `createdIp` is always `null` (not recorded).
  5. Do not change `src/lib/format.ts`.
- Check, in `sessions-page.test.tsx`:
  - `shows when a session expires as a time and as the time left`: `createdAt` two hours
    ago, `expiresAt` twelve and a half hours ahead (both built from `Date.now()` with
    `toISOString()`). The row shows `formatDateTime(expiresAt)` and "in 12h", and the text
    "just now" is nowhere on the page. Fails today.
  - `shows the browser a session was created from and says when nothing was recorded`: two
    rows, one with `userAgent: "QA-Browser/1.0"`, one with `null`; "QA-Browser/1.0" is
    shown and "Not recorded" appears three times (two addresses, one browser).

### T9. The lock message is one sentence (LH-SEC-2-02; ADM-14)

- Anchors: `src/features/auth/login-page.tsx:145-152`;
  `src/features/auth/login-page.test.tsx:85-104`.
- Change: in the `too_many_requests` branch, when `retryAfterSecs` is above zero the
  message is the console's whole sentence,
  `` `Too many failed sign-in attempts: try again in about ${minutesText}.` `` (`minutesText`
  as today); otherwise it is `serviceError.message` unchanged. The server's sentence is no
  longer joined to the console's.
- Check, in `login-page.test.tsx`:
  - Change the test at `:86`: the alert's text is exactly
    `Too many failed sign-in attempts: try again in about 5 minutes.` and does not contain
    "Try again later". It asserted both sentences, which is the defect; rename it to
    `shows one sentence with the wait in whole minutes on a 429` and say so in the commit
    (rule 2).
  - Add `shows the server's sentence when a 429 carries no wait`: the same stub without
    `Retry-After`; the alert is `Too many failed sign-in attempts. Try again later.`
  - Add a case for 60 seconds: "about 1 minute."

### T10. Run is shown only with `governance:write` (LH-DATA-14-04, TC-1728; GOV-10)

- Anchors: `src/features/governance/data-quality-page.tsx:56`, `:229-237`;
  `src/features/catalog/asset-quality.tsx:98`, `:287`, `:341`;
  `src/features/governance/quality-rule-create-page.tsx:72`; tests
  `src/features/governance/data-quality-page.test.tsx:234-270` (how a rule's drawer is
  opened), `src/features/governance/quality-rule-create-page.test.tsx:156-184`,
  `src/features/catalog/asset-detail-tabs.test.tsx:1294-1314`.
- Change:
  1. `data-quality-page.tsx:229`: the "Run check" button renders when
     `selected.evaluable && hasPermission("governance:write")`.
  2. `asset-quality.tsx:341`: "Run checks" renders when `canWrite && runnable.length > 0`.
  3. `asset-quality.tsx:98` and `quality-rule-create-page.tsx:72`: the first run after
     adding a rule asks `hasPermission("governance:write")` in place of
     `hasPermission("query:read")`.
  4. Each site gets a `// QA LH-DATA-14-04 (GOV-10): …` comment. No text on screen changes.
- Check:
  - `data-quality-page.test.tsx`, a new `describe`: with `["governance:read", "query:read"]`
    a rule's drawer shows no "Run check"; with `["governance:write"]` it does. Open the
    drawer as the run-history tests do.
  - `quality-rule-create-page.test.tsx`: the test at `:156` ends by asserting that no run
    was tried "without `query:read`". With `governance:write` the run is now tried: change
    its last assertion to wait for a `POST …/r1/run`, reword its comment, and say why in
    the commit (rule 2). Add `does not try the first run for a person without
    governance:write`: `stubApi(["query:read"])`, add a rule, and no write ends in `/run`.
    Fails today.
  - `asset-detail-tabs.test.tsx:1300-1314`: the test waits for "Run checks" to prove the
    session loaded for a person without `governance:write`; that button is now hidden for
    them. Wait instead until the stub has answered `/api/auth/me` (the spy's calls, then
    one 50 ms tick as `sessionSettled` does in `data-quality-page.test.tsx:125-128`), then
    assert that neither "Add rule" nor "Run checks" is there; in the second half assert
    both are there with `governance:write`. Say why in the commit (rule 2).

### T11. The console opens a table's page for `query:read` too (LH-DATA-15-03 F2; GOV-11)

- Anchors: `src/components/app-shell/nav-config.ts:298-299`;
  `src/components/app-shell/nav-config.test.ts:108-128`.
- Change: `{ pattern: "/data/assets/[assetId]", requires: ["catalog:read", "query:read"] }`
  with a comment: a `query:read` grant made from a table opens that table's page; the API
  decides which table (`GOV-11`, `DEC-20`, QA `LH-DATA-15-03` F2). `/catalog`, the sidebar
  entry (`:125`) and the command palette stay `catalog:read`.
- Check, in `nav-config.test.ts`: with only `query:read`, `canOpen("/data/assets/customers")`
  is true and `canOpen("/catalog")` is false; with only `dashboard:read` both are false.
  The first fails today.

### T12. The documents say what is now true (SEC-70, SEC-72, ADM-14, GOV-10, GOV-11)

Edit only these lines.

In every item below the new text is the block under it. A table cell stays on one line; a
bullet is wrapped like the bullets around it.

- `docs/core/features/login-protection-and-session-cleanup.md`:
  - `:86`, row 7, the "Expect" cell becomes:

    ```text
    An entry for the lock in step 3, with the actor "Sign-in form" and the kind Service, not the assistant. It contains no email and no password
    ```

  - `:90`, row 11, the "Expect" cell becomes:

    ```text
    Live sessions are listed, each with the time it expires and the browser it was created from ("Not recorded" for a session older than this change)
    ```

  - Under "Limits to tell a customer", after `:70`, add the bullet:

    ```text
    - A session's network address is not recorded: the API sits behind the console's own server and has no trusted source for it, so the Sessions page says "Not recorded". The browser is recorded as the browser named itself.
    ```

- `docs/core/features/data-quality-in-depth.md:49`, decision 6. The Signed cell becomes
  `2026-10-15 (updated 2026-11-12)` and the Default cell becomes:

  ```text
  Viewing needs what the Data Quality page needs; defining rules, resolving incidents and running a rule by hand need `governance:write`. Running by hand took `query:read` before (product owner 2026-11-12, `DEC-17`). Scheduled and on-load checks run as the system and are not affected.
  ```

- `docs/core/features/asset-page-depth.md`, after `:72` (the bullet "A grant expires after
  30 days, like a decided request."), add the bullet:

  ```text
  - A `query:read` grant made from a table's page also opens that table's page for the grantee, without `catalog:read` (product owner 2026-11-12, `DEC-20`). It opens that table only: not the catalog list, not search, not another table's page. Masks, row filters and the role's table access apply as everywhere.
  ```

- `docs/core/features/table-access-per-role.md:57` (the bullet "A temporary access grant
  gives a permission, not a table.") becomes:

  ```text
  - A temporary access grant gives a permission, not a table. A `query:read` grant made from a table's page opens that table's page (`GOV-11`), but only where the person's roles may read the table.
  ```

- `docs/OPERATIONS.md`, after `:1371`, add one paragraph: the migration of T6 (by its file
  name) adds `catalog:read` to the role named `Data Engineer` when it does not hold it, so
  the role that edits catalog entries can open the Catalog and a table's page; a renamed
  or custom role is not changed.
- `CHANGELOG.md`, under `## [Unreleased]`, in the style of the entries around them (what a
  customer sees, the backlog ID, the limit where there is one):
  - `### Security`: running a data-quality rule by hand needs `governance:write` (`GOV-10`);
    a request with a method no route has answers 405 and an unknown address 404, where both
    answered 500 (`SEC-72`).
  - `### Changed`: a `query:read` grant made from a table opens that table's page, and Data
    Engineer holds `catalog:read` (`GOV-11`, with the migration's number).
  - `### Fixed`: a sign-in lockout, a locked public link and a refused single sign-on link
    are no longer listed as the assistant's (`SEC-70`); the Sessions page shows when a
    session expires and the browser it came from, and the lock message is one sentence
    (`ADM-14`).
- Do not edit `docs/core/BACKLOG.md`, `PRODUCT.md`, `HANDOFF.md`, `AGENTS.md` or
  `docs/CODE-STANDARD.md` (the reviewer does what is needed, after merge).

## 3. Verification

The build machine is a strong ARM64 host (20 CPUs, 121 GB of memory, Docker running); see
section 3a for the checkout and the cargo settings. CI on the pull request runs on x86_64
and is the final word: a check that is green here and red there is red.

Per commit, only for what the commit touched. Never `cargo clean`. Never a bare
`cargo test -p lakehouse-api <filter>`: a unit test runs with `--lib` and its name, an
integration test file runs with `--test <file>`.

- Rust, from `rust/`: `cargo fmt --check`, then for each crate the commit touched
  `cargo clippy -p <crate> --all-targets -- -D warnings`, then the tests of the task:

  | Task | Crates for clippy | Tests |
  | --- | --- | --- |
  | T1 | `lakehouse-store`, `lakehouse-auth`, `lakehouse-api` | `cargo test -p lakehouse-api --test login_throttle`; `cargo test -p lakehouse-api --test public_links`; `cargo test -p lakehouse-auth --test oidc` |
  | T2 | `lakehouse-api` | `cargo test -p lakehouse-api --lib actor_kind_of_narrows_copilot_principal_kinds_to_agent`; `… --lib source_of_follows_the_principal_kind`; `… --lib actor_of_names_the_label_then_the_id_then_the_assistant_only_for_a_copilot_event`; `cargo test -p lakehouse-api --test login_throttle` |
  | T3 | `lakehouse-api` | `cargo test -p lakehouse-api --lib route_policy_tests`; `… --lib removed_agent_endpoints_answer_like_an_unknown_path`; `cargo test -p lakehouse-api --test route_auth` |
  | T4 | `lakehouse-store`, `lakehouse-api` | `cargo test -p lakehouse-api --lib the_session_user_agent`; `… --lib oidc_callback_with_a_matching_state_and_nonce_creates_a_session_and_redirects`; `cargo test -p lakehouse-api --test login_throttle`; `cargo test -p lakehouse-store --test sessions` |
  | T5 | `lakehouse-api` | `cargo test -p lakehouse-api --lib running_a_quality_rule_by_hand_needs_governance_write`; `cargo test -p lakehouse-api --test route_auth`; `cargo test -p lakehouse-api --test quality_incidents` |
  | T6 | `lakehouse-store`, `lakehouse-api` | `cargo test -p lakehouse-store --test identity`; `cargo test -p lakehouse-api --test route_auth` |
  | T7 | `lakehouse-store`, `lakehouse-api` | `cargo test -p lakehouse-api --lib sample_masking`; `cargo test -p lakehouse-api --test catalog_grants`; `cargo test -p lakehouse-api --test route_auth`; `cargo test -p lakehouse-api --test permission_catalog` |

- TypeScript: `bun install --frozen-lockfile` once, then per commit `bun run typecheck`,
  `bun run lint`, and `bun test` on the touched test files:
  `src/features/admin/sessions-page.test.tsx` (T8), `src/features/auth/login-page.test.tsx`
  (T9), `src/features/governance/data-quality-page.test.tsx`,
  `src/features/governance/quality-rule-create-page.test.tsx` and
  `src/features/catalog/asset-detail-tabs.test.tsx` (T10),
  `src/components/app-shell/nav-config.test.ts` (T11). Do not run prettier.
- Always: `python3 ops/lint/check_bare_iceberg_count.py && python3
  ops/lint/check_intra_package_imports.py`.
- No Python and no compose file changes, so no pytest and no `docker compose` check.

Before handoff, once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-store -p lakehouse-auth -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-api --lib route_policy_tests
cargo test -p lakehouse-api --lib sample_masking
cargo test -p lakehouse-api --test route_auth
cargo test -p lakehouse-api --test login_throttle
cargo test -p lakehouse-api --test catalog_grants
cargo test -p lakehouse-api --test public_links
cargo test -p lakehouse-api --test quality_incidents
cargo test -p lakehouse-api --test permission_catalog
cargo test -p lakehouse-store --test sessions
cargo test -p lakehouse-store --test identity
cargo test -p lakehouse-auth --test oidc
cd .. && bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status   # nothing generated staged
```

The full `cargo test --workspace` is CI's on the pull request; list it in the handoff as
*not verified locally, CI*. The handoff quotes each command with its counts, names every
Rust test added or changed so it can be found in the CI log, and says *not verified* for
anything not run (rule 7).

What the planner read and did not run, and the developer's runs must confirm. If one of
these answers differently, report the answer and stop; the plan is corrected, not the
assertion.

- That a fallback set after `.layer(auth_gate)` is outside the gate, and that an `any()`
  route keeps its gated fallback (read in the `axum` 0.8.9 sources; T3's four tests are the
  proof).
- That an admitted asset read answers 503 in the integration harness (T7; the test asserts
  only "neither 401 nor 403").
- That `to_char(… 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"')` gives exactly the text T4's store test
  expects.
- That `Not recorded` and the two-line Expires cell render as T8's tests look for them.

## 3a. Machine

- Checkout: `/home/rismana/rantai/lakehouse-agents-2`.
- Before any cargo command:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-2 CARGO_BUILD_JOBS=6`.
- The machine is shared. Work only under `/home/rismana/rantai/`. Start no container you do
  not remove.
- The Rust integration tests start one Postgres container through `lakehouse-test-support`
  and reuse it when it already runs. Before the first `cargo test`, run
  `docker ps -a --filter label=org.rantai.lakehouse-test-support` and write down whether it
  is there. If your run started it, remove it before the handoff (`docker rm -f <id>`); if
  it was there before, leave it: another checkout is using it. Say which in the handoff.

## 4. Out of scope

- **The approval route's missing audit entries** (a refused self-approval, a second
  decision): backlog `AI-28`, the AI team's (D5).
- **Recording a session's network address** (D10, question 1).
- **`HEAD` on a path that serves `GET` still answers `500 route_policy_unclassified`** (D8).
  It is not one of the QA cases; changing it means treating `HEAD` as `GET` in the policy
  lookup (question 2).
- **Ending or merging a person's earlier sessions at a new sign-in** (D14). Each sign-in
  keeps adding a session, as designed.
- **Other readers of the audit trail.** The home page's activity feed already says
  "Unknown" for an event with no name and its `actor_kind` for no principal is still
  `"agent"` (`lakehouse-store/src/overview.rs:596-603`); the console does not read that field.
  A table's change history says "Unknown" (`routes/catalog_governance.rs:866-870`). Both will
  show the new labels for new events without a change.
- **`audit_table_change` and `audit_policy_change` write the kind `user` for any principal**
  (`routes/governance.rs:986`, `:1370`), also for a service identity. Noticed while reading;
  not a QA finding.
- **The QA cases `LH-AI-18-10` and `LH-SRC-12-08`**, which the `SEC-70` row also cites, were
  not read as cases. T2 covers them as far as their rows have no principal.
- **The other reads of a table's page for a grantee without `catalog:read`**: restore
  status, storage, meaning, tag keys and the access request keep `catalog:read`; their
  cards show their own refusal (D20, question 3). The "Your access" card still lists
  `catalog:read` as not held, which is true.
- **A grant that means one table.** `query:read` stays global; a grant made from a table
  still reads every table in Query Studio (`routes/catalog_grants.rs:10-18`).
- **One mapping for "no access" messages across the console**: `WEB-11`.
- **The role table in `routes/ai/gate.rs:20-28`**, a record of an earlier change, already
  older than `0031`.
- Everything else in the QA round that is not on rows `SEC-70`, `SEC-72`, `ADM-14`,
  `GOV-10`, `GOV-11`.

### Open questions for the owner

None of these blocks the plan.

1. May a session's network address be recorded from a header that the operator's own
   reverse proxy sets? The planner's proposal: one setting that names the header (empty by
   default, which keeps "Not recorded"); when set, the API takes the address from that
   header only, and the operator answers for the proxy overwriting it. Without such a
   setting there is no address the API can trust (D10). If yes, it is one Rust task, one
   line each in `.env.example`, `docs/CONFIGURATION.md` and the compose file, and its own
   backlog item.
2. Should `HEAD` on a path that serves `GET` follow the `GET` row of the policy table
   (same handler, same permission) instead of answering 500? The planner's proposal is yes,
   as its own small item.
3. Should a `query:read` grant made from a table also open that page's other reads that
   take `catalog:read` (restore status, storage, meaning)? The planner's proposal is no:
   the grant opens the table's description, schema and rows, and the rest stays with
   `catalog:read`.

## 5. Handoff

Branch `fix/qa-round-admin-p2`, pushed after every commit. Twelve commits, one
per task, in the plan's order:

| Task | Commit | Title |
| --- | --- | --- |
| T1 | `e73f546a` | fix(audit): an event nobody signed in caused names what caused it (T1) |
| T2 | `fb31cb86` | fix(audit): the list never gives an event nobody signed in to the assistant (T2) |
| T3 | `6ad87107` | fix(routes): a method no route has answers 405, an unknown address 404 (T3) |
| T4 | `efd5bc7b` | fix(sessions): a sign-in records the browser; session times are RFC 3339 (T4) |
| T5 | `b0a3a1c8` | fix(quality): running a rule by hand needs governance:write (T5) |
| T6 | `8f888a54` | feat(identity): Data Engineer holds catalog:read (T6) |
| T7 | `e6f0ec55` | feat(catalog): a query:read grant on a table opens that table's page (T7) |
| T8 | `a413b9b6` | fix(sessions): the page shows the expiry and the browser (T8) |
| T9 | `25603c52` | fix(login): the lock message is one sentence (T9) |
| T10 | `f9853b7a` | fix(quality): Run is shown only with governance:write (T10) |
| T11 | `3360599a` | feat(nav): the console opens a table's page for query:read too (T11) |
| T12 | `ec3fc3e5` | docs: what is now true for the audit actor, 405/404, sessions, Run, a grant (T12) |

### Migration number

`rust/migrations/0138_data_engineer_catalog_read.sql`. On `main`
(`git ls-tree --name-only origin/main rust/migrations/ | tail -3`) the highest is
`0136_sso_trust_email.sql`; `0137` is planned by the unmerged sibling branch
`fix/qa-round-connectors-p1`, so this slice took `0138` and left `0137` free,
as the plan's T6 and the task brief both say.

### Commands run, with counts

Final commit (`ec3fc3e5`), in the foreground, from `rust/` with
`CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-2 CARGO_BUILD_JOBS=6`:

| Command | Result |
| --- | --- |
| `cargo fmt --check` | no diff |
| `cargo clippy -p lakehouse-store -p lakehouse-auth -p lakehouse-api --all-targets -- -D warnings` | `Finished` on all three crates |
| `cargo test -p lakehouse-api --lib route_policy_tests` | `5 passed; 0 failed; 3492 filtered out` |
| `cargo test -p lakehouse-api --lib sample_masking` | `11 passed; 0 failed; 3486 filtered out` |
| `cargo test -p lakehouse-api --test route_auth` | `93 passed; 0 failed` |
| `cargo test -p lakehouse-api --test login_throttle` | `10 passed; 0 failed` |
| `cargo test -p lakehouse-api --test catalog_grants` | `13 passed; 0 failed` |
| `cargo test -p lakehouse-api --test public_links` | `16 passed; 0 failed` |
| `cargo test -p lakehouse-api --test quality_incidents` | `8 passed; 0 failed` |
| `cargo test -p lakehouse-api --test permission_catalog` | `2 passed; 0 failed; 1 ignored` |
| `cargo test -p lakehouse-store --test sessions` | `10 passed; 0 failed` |
| `cargo test -p lakehouse-store --test identity` | `32 passed; 0 failed` |
| `cargo test -p lakehouse-auth --test oidc` | `35 passed; 0 failed` |
| `bun run typecheck` | no errors |
| `bun run lint` | `0 errors`, `11 warnings` (all pre-existing) |
| `bun run test` | `4001 pass; 1 skip; 0 fail; 11988 expect() calls; 4002 tests across 380 files` |
| `python3 ops/lint/check_bare_iceberg_count.py` | `R11 guard: no bare count() against a Bronze Iceberg table found.` |
| `python3 ops/lint/check_intra_package_imports.py` | `OK: all intra-package dispar_orchestrate imports resolve` |
| `git status` | nothing untracked or generated |

Per commit, only what the commit touched:

- T1: `cargo test -p lakehouse-api --test login_throttle` (8 passed),
  `--test public_links` (16), `cargo test -p lakehouse-auth --test oidc` (35);
  `cargo clippy` on `lakehouse-store`, `lakehouse-auth`, `lakehouse-api`.
- T2: `--lib actor_kind_of_narrows_copilot_principal_kinds_to_agent`,
  `--lib source_of_follows_the_principal_kind`,
  `--lib actor_of_names_the_label_then_the_id_then_the_assistant_only_for_a_copilot_event`
  (1 passed each); `--test login_throttle` (9 passed); clippy `lakehouse-api`.
- T3: `--lib route_policy_tests` (5 passed, 339 s),
  `--lib removed_agent_endpoints_answer_like_an_unknown_path` (1);
  `--test route_auth` (90 at this commit), later 91/92/93 as T5–T7 added theirs.
- T4: `--lib the_session_user_agent` (2 passed),
  `--lib oidc_callback_with_a_matching_state_and_nonce_creates_a_session_and_redirects`
  (1); `--test login_throttle` (10); `lakehouse-store --test sessions` (10);
  clippy `lakehouse-store`, `lakehouse-api`.
- T5: `--lib running_a_quality_rule_by_hand_needs_governance_write` (1);
  `--test route_auth` (91); `--test quality_incidents` (8).
- T6: `lakehouse-store --test identity` (32); `--test route_auth` (92).
- T7: `--lib sample_masking` (11); `--test catalog_grants` (13);
  `--test route_auth` (93); `--test permission_catalog` (2 passed, 1 ignored).
- T8: `bun test src/features/admin/sessions-page.test.tsx` (5 pass).
- T9: `bun test src/features/auth/login-page.test.tsx` (5 pass).
- T10: `bun test` on `data-quality-page.test.tsx`, `asset-detail-tabs.test.tsx`,
  `quality-rule-create-page.test.tsx` (191 pass).
- T11: `bun test src/components/app-shell/nav-config.test.ts` (11 pass).
- T12: documents only; the two lint scripts above.

The `405` carries axum's `Allow` header. Observed with a temporary print in
`a_method_no_handler_serves_answers_405_with_the_fixed_sentence`:
`GET,HEAD` for `DELETE /health` and `POST /api/dashboard`, `PUT` for both
target-lag requests, `GET,HEAD` for `POST /api/governance/quality/incidents`
and `GET,HEAD,PUT,DELETE` for `PATCH /api/reports/subscriptions/x`. Nothing
asserts it and nothing sets it by hand (the fallback's own body is the fixed
sentence), as the plan says.

### Tests added, and how each failed before its fix

Each failure was produced by stashing the source files of that task and
running the test against the previous code (or, for T6, by removing the
migration's `UPDATE` and leaving its header), then restoring.

- T1, `the_lock_sets_exactly_one_audit_event_naming_neither_the_email_nor_the_password`
  (extended): `assertion left == right failed: the lock names the sign-in form
  as its cause; left: None; right: Some("Sign-in form")`.
- T1, `five_wrong_passwords_lock_the_link_and_a_locked_link_says_so`
  (extended): `left: None; right: Some("Public link")`.
- T1, `an_unconfirmed_email_is_refused_for_an_existing_account` (extended):
  `left: None; right: Some("Single sign-on")`.
- T2, `a_lock_is_listed_in_the_audit_log_as_the_sign_in_forms_and_not_the_assistants`:
  `left: String("agent"); right: "service"`.
- T2, `actor_of_names_the_label_then_the_id_then_the_assistant_only_for_a_copilot_event`
  (new); `actor_kind_of_narrows_copilot_principal_kinds_to_agent` and
  `source_of_follows_the_principal_kind` changed their last line from the old
  fallback (`"agent"`, `"copilot"`) to the new exact value — rule 2, said in
  the test comments and the commit body.
- T3, `a_signed_in_person_gets_405_for_a_method_no_route_has_and_404_for_an_unknown_address`:
  `assertion left == right failed: DELETE /health has no handler; left: 500;
  right: 405`.
- T3, `a_handler_that_is_not_in_the_policy_table_answers_500_and_is_never_served`
  replaces `an_unregistered_route_denies_by_default_instead_of_serving_public`
  (rule 2: the old one proved it with `DELETE /health`, which is now a 405).
  New: `a_method_no_handler_serves_answers_405_with_the_fixed_sentence`,
  `an_address_no_route_matches_answers_404_with_the_fixed_sentence`,
  `a_route_registered_for_any_method_stays_behind_the_gate`.
- T4, `a_sign_in_records_the_browser_and_creates_exactly_one_session`:
  `left: None; right: Some("QA-Browser/1.0")`.
- T4, `session_times_travel_as_rfc_3339_in_utc`: `left:
  "2026-11-12 09:42:13.123+00"; right: "2026-11-12T09:42:13.123Z"`.
- T4, `the_session_user_agent_is_the_header_trimmed_and_cut_to_512_characters`
  and `the_session_user_agent_is_nothing_for_an_absent_empty_or_non_ascii_header`
  (new); the single sign-on round-trip test now sends `user-agent:
  QA-Browser/1.0` and asserts the session row.
- T5, `running_a_quality_rule_by_hand_is_gated_in_both_directions`:
  `assertion left == right failed: a viewer may not run a rule by hand;
  left: 404; right: 403`. Also `running_a_quality_rule_by_hand_needs_governance_write`.
- T6, `the_data_engineer_role_holds_catalog_read_exactly_once_even_when_the_migration_runs_again`:
  `left: 0; right: 1`, with Data Engineer's permissions printed as
  `pipeline:*, catalog:write, connector:manage, workload:cancel, alert:write,
  ingest:read,assistant:use,ops:read,alert:read,governance:read`. Also
  `a_seeded_data_engineer_may_read_the_catalog`.
- T7, `a_query_read_grant_on_a_table_opens_that_tables_page_and_no_other`:
  `a query:read grant from silver.orders must open it, got 403 Forbidden:
  {"error":"permission_denied: catalog:read"}`. Also
  `detail_admits_a_person_granted_query_read_on_that_table_and_masks_what_they_read`,
  `a_revoked_or_expired_grant_no_longer_opens_the_page`,
  `a_grant_of_another_permission_does_not_open_the_page` and
  `the_asset_page_route_is_gated_in_both_directions`.
- T8, `shows when a session expires as a time and as the time left` and
  `shows the browser a session was created from and says when nothing was
  recorded`: both fail on the reverted page (the first on `in 12h`, the second
  on `QA-Browser/1.0` and the three "Not recorded" cells).
- T9, `shows one sentence with the wait in whole minutes on a 429` (renamed and
  rewritten, rule 2) and `says about one minute for a wait of 60 seconds` fail
  on the reverted page; `shows the server's sentence when a 429 carries no wait`
  is new and passes both ways.
- T10, `hides Run check for a person who may only read governance`,
  `does not try the first run for a person without governance:write` (new) and
  the two rewritten tests (`asset-detail-tabs`' Add rule test and
  `quality-rule-create-page`'s threshold test) fail with the four source files
  reverted.
- T11, `opens a table's page for query:read, and not the catalog`: fails on the
  reverted `nav-config.ts`.

### Notes for the reviewer

- `cargo fmt` also rewrapped one existing `if let (Some(detail), Some(o)) = …`
  line in `routes/governance.rs` (T2's commit). It is a formatting change in a
  file T2 touched, not a behaviour change.
- `sqlx::migrate!` embeds the migration set at compile time of
  `lakehouse-store`, so after adding `0138` the api integration harness kept
  its old set until that crate was recompiled (`touch
  crates/lakehouse-store/src/lib.rs`, no content change). Worth knowing for the
  reviewer's own run.
- The Postgres testcontainer (`org.rantai.lakehouse-test-support`, id
  `ca1ffac89342`) was already running when this slice started (`docker ps -a`
  said "Up 2 hours") and is left in place, as section 3a says.
- No `MISMATCH`: every anchor in the plan was found, by its text where a line
  number had moved.

### Not verified

- `cargo test --workspace` — *not verified locally, CI* (the plan's section 3
  puts the full run on the pull request). The per-crate and per-test runs above
  are what ran here.
- `cargo test --doc` — *not verified*: not in section 3's block.
- x86_64 CI — *not verified*: this machine is ARM64.
- `docker compose` — *not run, not needed*: no compose file changed.
- `pytest` / `dispar_orchestrate` — *not run, not needed*: no Python changed.

## 6. Review

### Round 1 (planner, 2026-11-12), at `80ac6b05`

Read against the plan and `AGENTS.md`; nothing was run for this review, CI on the draft pull
request (#487) runs the suites. No BLOCKER, 4 SHOULD-FIX. No defect found in the 405/404
router order, the 500 for an unclassified handler, the grant check's code paths, migration
`0138`, the audit actor mapping or the sessions change; no assertion in
`tests/route_auth.rs` was removed or loosened.

**Planner's ruling on R2 (changes D20): option (b), and wider.** A person admitted by a table
grant, without `catalog:read`, is shown that table and nothing about other tables or other
people. `ensure_may_open_asset` returns which of the two admitted the caller; for a caller
admitted by the grant, the body leaves out `upstream`, `downstream`, `usage.joinedTables`,
`usage.frequentUsers`, `usage.frequentQueries`, `dependents`, `replacementAssetId` and
`replacementName` (empty lists or absent keys, as each field's type allows, and one typed
flag so the console can say "Shown with a table grant: related tables and usage are hidden"
instead of drawing empty panels as if nothing existed). The table's own description, columns,
sample (under the caller's masks and row filters), freshness, marks, policy summary and
history stay. A caller with `catalog:read` gets the body unchanged. The two sentences T12
added then stay as written; add the one sentence about what is hidden to
`asset-page-depth.md` (Signed: *(proposed, not yet signed)*).

R1, R3 and R4: as the findings say.

### R1. SHOULD-FIX: the revoked, expired and other-permission tests of T7 pass without ever reaching the SQL they are named for, and the approved-request origin has no test at all

- Where: `rust/crates/lakehouse-api/tests/catalog_grants.rs:412-423` (`catalog_tenant_harness`,
  the grantee is `create_zero_permission_principal`), `:493`
  (`a_revoked_or_expired_grant_no_longer_opens_the_page`), `:540`
  (`a_grant_of_another_permission_does_not_open_the_page`);
  `rust/crates/lakehouse-api/src/routes/catalog.rs:3311-3320`;
  `rust/crates/lakehouse-store/src/agents.rs:1879-1900`.
- What happens: the grantee holds no role permission, so the grant under test is also the
  only thing that gives them `query:read`. Once it is revoked or expired,
  `load_user_permissions` (`lakehouse-auth/src/repository.rs:199-204`) drops it, so
  `principal.has(SAMPLE_PERMISSION)` is false and `ensure_may_open_asset` answers 403 before
  `holds_asset_grant` is called. The same holds for the `lineage:read` grant: the person never
  has `query:read`. Delete `g.revoked_at IS NULL AND g.expires_at > now()` or
  `g.permission = $3` from `holds_asset_grant` and all three assertions still pass. Those
  predicates are the only guard in the real cases: (a) a grant from table A is revoked or
  expires and the person then gets a `query:read` grant from table B (the route allows it,
  `rust/crates/lakehouse-api/src/routes/catalog_grants.rs:168-181`, because they no longer
  hold the permission): A's page must stay
  closed; (b) a person whose role holds `query:read` without `catalog:read` (the seeded Data
  Scientist) is given a `lineage:read` or `catalog:write` grant from a table: its page must
  stay closed. The unit test in `sample_masking` covers only the `user_id` and `catalog_id`
  predicates. The second origin (`a.resource = $2`, the grant of an approved request) is
  exercised by no test in either crate; `holds_asset_grant` has no store test
  (`grep holds_asset_grant lakehouse-store/tests` is empty).
- Fix: no production change. In `catalog_grants.rs` make the grantee of these two tests a
  principal that holds `query:read` by role (`create_principal_with_permissions(&pool,
  "query:read")`, put `in_the_catalog_tenant`), and insert the `query:read` grant rows with
  `lakehouse_store::agents::create_direct_grant` (the route answers 409 for a person who
  already holds the permission); revoke through the route's `DELETE`, expire with the existing
  `UPDATE`. Assert 200-or-503 before and 403 `permission_denied: catalog:read` after. For the
  other-permission test, grant `lineage:read` through the route to that same role-holder. Add
  a store test file case for `holds_asset_grant` (in `lakehouse-store/tests/agents.rs`):
  direct grant true; other asset, other user, other permission, revoked, expired false; and a
  grant row with `approval_id` whose `approval_item.resource` is the asset true, another
  resource false.
- Check: each new assertion must fail when the matching line of the `holds_asset_grant` SQL
  is removed (`revoked_at`, `expires_at`, `permission`, `a.resource`); say so per assertion in
  the handoff.

### R2. SHOULD-FIX: the body a grantee without `catalog:read` receives names other tables, while the backlog row and the new documents say the grant shows "only that table"

- Where: `rust/crates/lakehouse-api/src/routes/catalog.rs:1774-1805` (nothing after the
  admission distinguishes the two kinds of reader), `:2634-2639` (`upstream`), `:2830`
  (`downstream`), `:2124-2134` (`replacementAssetId`, `replacementName`), `:2305-2327`
  (`mark_governance`); `rust/crates/lakehouse-api/src/routes/catalog_governance.rs:409-420`
  and `:658-679` (`usage.joinedTables` with each table's `assetId`), `:548-549`
  (`frequentUsers`, `frequentQueries`), `:197-246` (`dependents`);
  `docs/core/features/asset-page-depth.md:73`; `CHANGELOG.md:44`; `docs/core/BACKLOG.md:355`.
- What happens: a person with a `query:read` grant from `silver.orders` and no `catalog:read`
  or `lineage:read` calls `GET /api/catalog/silver.orders` and gets, besides that table: the
  Bronze dataset it is built from (`upstream`: slug and `bronze.<table>`), up to ten other
  tables people joined with it in the last 30 days, each with its catalog id
  (`usage.joinedTables`), the names of the people who query it most and the shapes of their
  statements (`frequentUsers`, `frequentQueries`), the saved queries and dashboards that read
  it (`dependents`), the replacement asset's id and name when it is deprecated, and its policy
  summary and change history. For a Bronze id it is `downstream` (`silver.<table>`). This is
  exactly what plan D20 decides ("the same body a `catalog:read` reader gets"), so the code
  matches the plan. But `GOV-11` in the backlog says "The page shows such a person only that
  table", and the two sentences T12 added ("It opens that table only: not the catalog list,
  not search, not another table's page") read as if nothing of another table is shown. The
  lineage neighbours reach a person who was never given `lineage:read`.
- Fix: a decision for the planner and the owner, not an improvisation for the developer.
  Either (a) keep D20 and add one sentence to `asset-page-depth.md` under the new bullet:
  the page still names the tables it is built from and joined with, the people who query it
  and what reads it; or (b) for a caller admitted by the grant (not by `catalog:read`), leave
  out `upstream`, `downstream`, `usage.joinedTables`, `replacementAssetId` and
  `replacementName`: have `ensure_may_open_asset` return which of the two admitted the
  caller, and strip those keys from the body at the end of `detail`.
- Check: for (b), extend
  `detail_admits_a_person_granted_query_read_on_that_table_and_masks_what_they_read`: the
  grantee's body has no `upstream`, and `usage.joinedTables` is empty, while the same call as
  `analyst("catalog:read, query:read")` still has them. For (a), no test; the sentence.

### R3. SHOULD-FIX: no test proves the `GOV-7` half of D20 for a grant-holder, though three documents now state it

- Where: `rust/crates/lakehouse-api/src/routes/catalog.rs:1784-1797`;
  `docs/core/features/table-access-per-role.md:57`, `docs/core/features/asset-page-depth.md:73`,
  `CHANGELOG.md:44`; no case in `tests/catalog_grants.rs` or in `mod sample_masking`.
- What happens: the code is right today: `table_access_for` runs after the admission for
  every reader, so a grantee whose roles hide the table gets the 404 an unknown id gets. But
  the only thing keeping it right is statement order in `detail`. A later change that returns
  early for an admitted grantee, or moves the grant check below the access check, would open
  a hidden table to a grant-holder and no test would fail. The plan's T7 checks did not ask
  for one; the claim was then written into two feature pages and the changelog.
- Fix: one test, no production change.
- Check: in `tests/catalog_grants.rs`,
  `a_grant_does_not_open_a_table_the_persons_roles_hide`: a grantee whose role is narrowed by
  `GOV-7` to a schema that does not contain `silver.orders` (set up the role's table access the
  way the `GOV-7` tests of `/api/catalog/{id}` do), with a `query:read` grant from
  `silver.orders`; `GET /api/catalog/silver.orders` answers 404 `Asset not found`, not 403 and
  not 200/503.

### R4. SHOULD-FIX: T5 deleted the first line of an unrelated test's doc comment in `tests/route_auth.rs`

- Where: `rust/crates/lakehouse-api/tests/route_auth.rs:2655-2658` (base `75010df81`:
  `:2592-2595`).
- What happens: the two new tests were inserted between the first and second lines of the doc
  comment of `quality_schedule_routes_are_gated_in_both_directions`. The line
  ``/// `DATA-14` (F3): any login may read the schedules, and setting one needs`` and the
  blank line before it are gone, so the comment now starts mid-sentence
  (``/// `governance:write`. A principal without it is refused (403) and nothing is stored;``)
  directly under the closing brace of the test above. This is the one removed line in the
  file (`git diff 75010df81 HEAD -- tests/route_auth.rs` has exactly one `-` line); no
  assertion was removed or loosened.
- Fix: restore the deleted line and the blank line above it, unchanged.
- Check: `git diff 75010df81 HEAD -- rust/crates/lakehouse-api/tests/route_auth.rs | grep -c '^-[^-]'`
  prints `0`.

