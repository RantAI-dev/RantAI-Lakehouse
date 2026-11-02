# BI-38: a limit on posting comments, and marking one dashboard's mentions read

Backlog `BI-38` (Task, Dashboards, P3). Base: `main` at `9c066426`. Planner: Claude Opus.
Developer: another agent (`AGENTS.md`). Sources: `BI-19` part E handoff (deviations 3 and 5,
planner questions 1 and 2) and the part E review's backlog notes
(`docs/superpowers/plans/2026-10-18-bi-19-sharing.md`, sections 7 and 8).

The row's third point, "the mention email was not seen end to end in QA", is `SEC-56`'s (the mail
sender cannot reach an internal relay). It is not planned here; see section 4.

## 0. What is wrong, with evidence

**Posting has no limit.**
- `POST /api/dashboard/comments` (`rust/crates/lakehouse-api/src/routes/dashboard_comments.rs:404-424`)
  checks access, the text and the mentions, then calls `comments::insert`
  (`lakehouse-store/src/dashboard_comments.rs:179-208`). Nothing counts how many comments the person
  has posted. The part E handoff says so ("5. No rate limit on posting"), and the review kept "a
  general comment post rate limit" as a backlog note.
- The review's E-1 fix bounds the **emails** (one per person per dashboard until read,
  `first_unread` `:140-160`, decided under `lock_board` `:163-170`), but not the posts: anyone with
  view can write rows in a loop, and every post with a mention adds a bell line for up to 20 people.

**"Mark as read" marks every dashboard.**
- `mark_read` (`routes/dashboard_comments.rs:564-580`) takes no input and calls
  `comments::mark_mentions_read(pool, user)` (`store :332-343`), which updates every unread mention
  of the person.
- The console calls it only from the bell's "Mark all as read"
  (`src/components/app-shell/app-navbar.tsx:180`, client `src/services/clients/notifications.ts:19-30`).
  Opening a dashboard's comments (`src/features/dashboards/comments-panel.tsx:223`, opened from the
  bell link with `?comments=1`, QA-3) leaves its mentions unread, so the bell keeps the dot for a
  conversation the person has just read (part E planner question 1).

## 1. Decisions

- **D1. At most 10 comments per person in any 5 minutes, across all dashboards** *(planner default;
  the owner signs the two numbers)*. Two constants in the store with their reason at the site, not
  settings. They allow a real back-and-forth (a reply every 30 seconds) and stop a loop.
- **D2. Counted where it cannot be raced or dodged.** The count is of the author's
  `dashboard_comment` rows with `created_at` inside the window, **deleted rows included** (deleting
  and posting again does not free a place). It is read inside `insert`'s transaction after
  `pg_advisory_xact_lock` on `dashboard_comment_post:<author>`, so two posts at once cannot both take
  the last place (the `DATA-37` D3 pattern). A new migration adds the index
  `dashboard_comment (author_id, created_at)`.
- **D3. A refusal is a 429 with `Retry-After`** (`ApiError::TooManyRequests`, rendered with the
  header by `error.rs:64`): "You have posted many comments in a few minutes. Try again in a moment."
  `Retry-After` is the seconds until the oldest counted post leaves the window, at least 1. Nothing
  is written, no mention is recorded, no email is sent.
- **D4. Only posts count.** An edit is not counted: it writes no new row, and its emails are already
  limited by E-1. A deleted or hidden dashboard still answers 404 before the limit is read (access
  first, as today, so the limit is no oracle for a board's existence).
- **D5. `POST /api/dashboard/comments/mentions/read` takes an optional board.** Body `{}` (today's,
  and the bell's "Mark all as read") marks every mention. Body `{ "board": "<id>" }` first checks the
  board as the other comment routes do (`target_for`: a board the caller cannot see is
  `404 dashboard not found.`), then marks only the caller's unread mentions on comments of that
  board. A body of another shape is 400. The answer stays `{ "marked": n }`.
- **D6. Opening a dashboard's comments marks its mentions read** *(planner default)*: once the
  panel's list has loaded without error, the console posts D5 with the board, then tells the bell to
  read again. The bell is loaded once by the navbar (`app-navbar.tsx:51`), so a small event in
  `src/lib` (the pattern of `features/dashboards/dashboard-events.ts:13`; `src/lib` because the
  navbar in `components/app-shell` must not import a feature) makes it reload. A failure to mark is
  silent: the mentions stay unread, which is the truth.
- **D7. Assistant parity: no change.** No assistant tool posts a comment or reads the bell today;
  the read-only `list_dashboard_comments` is `BI-37`'s. A later posting tool would go through
  `comments::insert`, so it would get D1 with no extra work. No fixture change, no AI-15 case.

## 2. Anchors (verified at `9c066426`)

- `rust/crates/lakehouse-store/src/dashboard_comments.rs`: `MAX_CHARS` `:25`, `first_unread`
  `:140-160`, `lock_board` `:163-170`, `insert` `:179-208`, `mark_mentions_read` `:332-343`.
- `rust/crates/lakehouse-store/tests/dashboard_comments.rs` (store tests, `sqlx::test`).
- `rust/migrations/0114_dashboard_comments.sql` (table `:26`, indexes `:38-39`, `:49`); the highest
  migration on `main` is `0117_outside_table_read_day.sql` (`0115` is not on `main`), so the next
  free number at `9c066426` is `0118`. Re-check before writing it: take the next number above
  the highest on `main` at build time.
- `rust/crates/lakehouse-api/src/routes/dashboard_comments.rs`: route table `:7-14`, `target_for`
  `:166`, `post` `:404-424`, `mark_read` `:564-580`.
- `rust/crates/lakehouse-api/tests/dashboard_comments_routes.rs`; `tests/route_auth.rs:3115`
  (`the_comment_routes_are_gated_in_both_directions`).
- `rust/crates/lakehouse-api/src/policy.rs:727` (the mark-read row, `RequiresAuth`, unchanged).
- `rust/crates/lakehouse-api/src/error.rs:64` (`TooManyRequests` → `Retry-After`).
- Console: `src/services/contracts/notifications.ts:60` (`markMentionsRead`),
  `src/services/clients/notifications.ts:19-30`, `src/components/app-shell/app-navbar.tsx:51`, `:180`,
  `src/features/dashboards/comments-panel.tsx:61-62` (`CommentsPanel`, its list), `:223`
  (`CommentsDialog`), `src/features/dashboards/dashboard-events.ts:13`.

## 3. Tasks (one commit each, in order)

- **T1. Migration and store (D1, D2, D5).**
  - Migration `NNNN_dashboard_comment_author_index.sql` with a why-header (D2): one
    `CREATE INDEX IF NOT EXISTS … ON dashboard_comment (author_id, created_at)`.
  - `insert` locks the author, counts, and returns an outcome the route can tell apart, e.g.
    `Posted { id, email }` or `TooMany { retry_after_secs }` (the name is the developer's; the
    variant set is not).
  - `mark_mentions_read(pool, user, board: Option<&str>)`; with a board the `UPDATE` is limited to
    mentions whose comment is on that board.
  - Store tests:
    - ten posts in a row are written, the eleventh is `TooMany` and writes no comment and no mention;
    - a deleted post still counts;
    - a post older than the window does not count (insert a row with an old `created_at`);
    - two posts started together at the ninth place: exactly one is written;
    - mark-read for board A leaves board B's mentions unread; with no board both are read.
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-store --all-targets -- -D warnings`;
    `cargo test -p lakehouse-store --test dashboard_comments`.
- **T2. Routes (D3, D4, D5).**
  - `post` maps `TooMany` to D3's 429. `mark_read` reads the optional body and runs `target_for`
    when a board is named. Update the route table in the module doc.
  - Route tests in `tests/dashboard_comments_routes.rs`:
    - the eleventh post is 429 with a `Retry-After` header and the fixed sentence, and the list
      still has ten;
    - a post to a hidden dashboard by a person at the limit is still 404 (D4);
    - mark-read with a board the caller cannot see is 404 with the same body as an unknown board;
    - mark-read with board A marks only A's (the bell lines of B remain);
    - a body `{"board": 1}` is 400.
  - `route_auth.rs` is unchanged (no new route): run it to prove it.
  - *Accept:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api --test dashboard_comments_routes`;
    `cargo test -p lakehouse-api --test route_auth`; `cargo test -p lakehouse-api --lib dashboard_comments`.
- **T3. Console (D3, D6).**
  - `markMentionsRead(board?: string, signal?)`: body `{}` or `{ board }`. The contract comment says
    what each marks.
  - `src/lib/mention-events.ts`: `notifyMentionsRead()` and a subscribe function; the navbar reloads
    its notifications on it.
  - `CommentsPanel`: after the list loads, mark the board read once per open, then notify. A 429
    on post shows the server's sentence under the box (it already shows `message`).
  - Tests: opening the dialog posts `{ board }` once (not on every re-render); a failed mark shows
    nothing; the navbar reloads after the event; a 429 shows the sentence and keeps the typed text.
  - *Accept:* `bun run typecheck`; `bun run lint`;
    `bun test src/features/dashboards/comments-panel.test.tsx src/components/app-shell src/services/clients`.
- **T4. Docs.** Feature page `sharing-and-delivery.md`: one sentence under E (the limit and the
  mark-read on open; the planner sets the decision rows). `CHANGELOG.md` line. *Accept:* docs only.

Full verification block of `AGENTS.md` once before handoff. The migration is proven by the
`sqlx::test` runs and by `docker compose up` of the API profile on a clean project (rule 8 applies to
compose; a migration is applied by the API at start, so record that `up` or write *not verified*).

## 4. Out of scope

- The mention email end to end (`SEC-56`: trusting an internal mail relay).
- A limit on edits (D4) or on the @ picker (already debounced, `BI-19` E-3).
- Comments on a chart (needs a D15 change).
- Settings for the two numbers (D1).

## 5. Handoff (developer appends below)

### Handoff (developer, 2026-11-02)

Branch `fix/bi-38-comment-limits`, based on the merge of `origin/main` (`dc9c9ac1`) into the
plan's branch (`b8b38e64`). Every commit cites `BI-38` and the decision it implements; pushed
after each.

- `e5104b5e` **T1** — migration `0121_dashboard_comment_author_index.sql`; `insert` locks the
  author (`dashboard_comment_post:<author>`) after the board's mention lock, counts the window
  (deleted rows included) and returns `PostOutcome::{Posted, TooMany { retry_after_secs }}`;
  `mark_mentions_read(pool, user, board: Option<&str>)`; five new store tests; the existing store
  tests use a `posted` helper for the new return type (D1, D2, D5).
- `a6043ce7` **T2** — `post` maps `TooMany` to `ApiError::TooManyRequests` (`TOO_MANY_POSTS`,
  `pub` for the test, as `SERVICE_REFUSED` is) so `error.rs` renders the 429 and `Retry-After`;
  `mark_read` takes an optional body and resolves a named board through `target_for`; five route
  tests (D3, D4, D5).
- `298861a5` **T3** — `markMentionsRead(board?, signal?)`, `src/lib/mention-events.ts`, the
  panel's once-per-open mark after the list loads, the navbar's reload on the event, and tests
  for the 429 sentence and kept text (D3, D6).
- `9af0119f` **T4** — feature page Part E sentence and the CHANGELOG entry.

**Commands run, with counts** (final commit, scoped to the crates and folders touched; every
cargo line through `~/rantai/bin/cargo-locked` with `-j 2`, no `cargo clean`):

```
cd rust && ~/rantai/bin/cargo-locked fmt --check                                  # 0 diffs
~/rantai/bin/cargo-locked clippy -p lakehouse-store -p lakehouse-api --all-targets -j 2 -- -D warnings
                                                                                  # 0 warnings/errors
~/rantai/bin/cargo-locked test -p lakehouse-store --test dashboard_comments -j 2  # 13 passed, 0 failed
~/rantai/bin/cargo-locked test -p lakehouse-api --test dashboard_comments_routes -j 2
                                                                                  # 17 passed, 0 failed
~/rantai/bin/cargo-locked test -p lakehouse-api --test route_auth -j 2            # 73 passed, 0 failed
~/rantai/bin/cargo-locked test -p lakehouse-api --lib dashboard_comments -j 2     # 6 passed, 2940 filtered
bun run typecheck                                                                 # 0 errors
bunx eslint <the eight touched src files>                                         # 0 problems
bun test src/features/dashboards/comments-panel.test.tsx src/components/app-shell src/services/clients
                                                                                  # 164 passed, 0 failed, 510 expect(), 16 files
```

`bun run lint` (the whole tree) was **not green on this machine**, and the failure is not in this
branch: it reports 12,356 problems / 649 errors, every error under
`dagster/.venv/lib/python3.12/site-packages/...` (a local Python virtualenv, untracked and not
ignored by `eslint.config.mjs`). `bunx eslint . --ignore-pattern 'dagster/.venv/**'` gives
0 errors and 10 pre-existing warnings. CI has no `.venv`; not verified there.

**Not verified.**

- The full-workspace block (`cargo clippy --workspace --all-targets --all-features`,
  `cargo test --workspace`) was **not** run: the developer brief says "No full-workspace run" and
  scopes every cargo command, which overrides the plan's "full verification block once before
  handoff". The crates touched are `lakehouse-store` and `lakehouse-api`; both are green above.
- `docker compose --profile '*' config --quiet` and a real `docker compose up` were not run (no
  compose change). The migration is proven applied by `sqlx::test`: the 13 store tests and the
  17 route tests each build a fresh database from `rust/migrations`, `0121` included.
- The Python lint scripts and the Dagster tests were not run: no Python changed.
- No browser QA (the console tests run in happy-dom). The bell reload is a `window` event and
  was exercised by the unit test only.

**Notes for the review.**

- `mark_read` keeps today's no-body behaviour: an empty body is `{}` (mark all). D5 names `{}`
  and another shape as 400; the route took no body before `BI-38`, so dropping the empty body
  would have been a regression. The body uses `deny_unknown_fields`, so extra keys are 400;
  `tests/route_auth.rs` sends a synthetic body with extra keys to every comment route and still
  passes (it only requires "not 401/403" for a reader).
- The existing panel test "posts the text with the person picked" now picks the comment POST by
  endpoint: the panel also sends mark-read, which is a POST to another path.
- `PostOutcome` is the plan's "outcome the route can tell apart"; the variant set is D1's
  (`Posted { id, email }`, `TooMany { retry_after_secs }`).
- The count query binds the window as `make_interval(secs => $2)`; `POST_WINDOW_SECS` is the
  single place the five minutes live, beside `POST_LIMIT` (D1).

## 6. Review (planner appends below)


**Migration number (planner, 2026-11-02):** the index migration is `0121`.

### Review round 1 (planner, 2026-11-02, at `543f0dfd`)

- **T1 (D1, D2, D5).**
  - Migration `0121` has a why-header. Its index is deliberately not partial, because deleted rows count against the limit.
  - `insert` counts and writes under one advisory key per author, `dashboard_comment_post:<author>`, so two posts sent at once cannot both get in.
  - `mark_mentions_read` takes an optional board.
- **T2 (D3, D4).** At the limit the API answers 429 with `Retry-After` and a fixed sentence. Mark-read for one board goes through `target_for`.
- **T3 (D6).** The panel marks its own board read once per open, and the navbar count follows through the event. The text being typed is kept when a post is refused.
- **Numbers.** The default of 10 posts in 5 minutes holds until the owner answers.
- **Counts (the developer's):**
  - store `dashboard_comments`: 13; routes: 17; `route_auth`: 73; `--lib`: 6;
  - console: 164;
  - clippy and fmt: clean.

**Open: no BLOCKER, no SHOULD-FIX.** The merge waits for the PR's CI.
