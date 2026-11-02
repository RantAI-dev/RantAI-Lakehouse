# AI-24: conversation History — reopen, rename, delete, and smooth streaming

Backlog `AI-24` (new Task, AI Copilot, P1, part of `AI-18`'s acceptance). Base: `main` at
`9c066426`. Planner: Claude Opus. Developer: another agent (`AGENTS.md`). The AI team reviews.
Plan file in the repo: `docs/superpowers/plans/2026-11-02-ai-24-history-and-streaming.md`.

Owner decision, 2026-11-02 (`docs/core/features/copilot-chat.md:229`): "a past conversation can
be reopened and continued, renamed and deleted, and replies stream smoothly as they are
written. A missing rename or delete is a gap to build."

(`AI-23` is unused on `main` at `9c066426`; this item takes `AI-24` as assigned.)

## 0. What is wrong, with evidence

### What already exists (not rebuilt here)

- **Rename and delete exist, in the API and in the console.**
  - API: `PATCH /api/ai/sessions` → `sessions_rename` (`routes/ai/mod.rs:2056`). It sets
    `title_locked`, caps the title at 120 characters, and a later save keeps the locked title
    (`:1995-2006`).
  - API: `DELETE /api/ai/sessions?id=` → `sessions_delete` (`:2129`), a soft delete.
  - Both are routed at `routes/mod.rs:1469-1472` and listed in `policy.rs:753-754`
    (`RequiresAuth`). Another user's id is a 404: `session_row` filters on `owner_id`
    (`:1855-1873`).
  - Console: the "…" menu of a History row (`copilot-history-page.tsx:183-192`) opens
    `RenameSessionDialog` and `SessionActionsMenu` (`session-parts.tsx:108`, `:162`); Delete
    asks "Delete conversation?" first (`:209`).
- **Reopen and continue exist.**
  - A History row links to `/copilot?id=…` (`copilot-history-page.tsx:234`).
  - `SessionUrlSync` loads it (`copilot-page.tsx:26-49`) through `loadSession`
    (`use-copilot.ts:567-581`), with its messages, tool steps and mode.
  - The next message is saved into the same id (`persist`, `use-copilot.ts:274-284`,
    `:415`).
- **Streaming exists, and it sends text pieces as they are written.**
  - The model client asks for `stream: true` and reads its SSE
    (`lakehouse-llm/src/stream.rs:273-298`).
  - The API forwards each piece as an NDJSON `{"type":"delta"}` line
    (`routes/ai/mod.rs:740-768`), one write and flush per line (`:1202-1213`), as
    `application/x-ndjson` with `Cache-Control: no-cache` (`:1219-1225`).
  - The console reads the lines (`src/lib/ndjson.ts`), appends each piece
    (`use-copilot.ts:350-356`) and reveals it at a set pace (`chat-messages.tsx:465-493`).

### The gaps, each one pointable

- **G1. The title bar loses the title.** `copilot-page.tsx:110` takes the title from
  `c.sessions`, which holds only the 20 most recent conversations (`RECENT_SESSIONS = 20`,
  `use-copilot.ts:86`; the feature page says eight, `copilot-chat.md:140`, a page mismatch the
  planner corrects). `loadSession` ignores `json.session.title` (`:575-578`), and
  `renameSession` (`:597-611`) only refreshes that list. A conversation reopened from History
  that is not among the 20 newest shows "Conversation", before and after a rename. That fails
  `AI-18-AC20` ("the conversation's title bar shows it too") for any older conversation.
- **G2. Opening a deleted or foreign conversation shows server text.** `loadSession` shows
  `json.error` as is (`use-copilot.ts:574`, `:579`), so the person reads the API's Indonesian
  "sesi tidak ditemukan". `AI-18-AC21` and `AC22` need a plain "not found / could not be
  loaded" message, and the owner's QA rule (`copilot-chat.md:230`) says no technical error
  text.
- **G3. A long conversation reopens without its tool steps.** Over 200,000 bytes,
  `sessions_save` keeps only `role`, `content`, `buildRunId` and `chartCreated` of every
  message (`routes/ai/mod.rs:2008-2020`). The tool steps, times and reasoning of the whole
  conversation are dropped, so `AI-18-AC18` ("with its messages and its tool steps") fails
  exactly for the conversations people return to. Most of that size is the tools' `result`
  payloads (`ToolStep.result`, `tool-step.tsx:34`), not the text.
- **G4. The reveal falls behind and the rest jumps in at the end.**
  - The pace is 25, 60 or 180 characters a second, by how far behind it is
    (`chat-messages.tsx:479`).
  - A hosted model writes about 300 characters a second. Worked from those constants: after
    5 seconds (1,500 characters received) the reveal has shown about 750.
  - At `done` the draft is cleared (`use-copilot.ts:429`) and the checked answer replaces it
    whole (`:392-409`), so about 750 characters appear at once. A long answer always ends in
    a jump.
  - Measured from the code's constants, not on a running model. T5 records it on a real
    endpoint.
- **G5. Every piece re-renders the whole console.**
  - `setDraft` runs once per piece (`use-copilot.ts:354`) inside `CopilotProvider`, whose
    value is a new object on every render (`:613-631`).
  - Every `useCopilot()` consumer therefore re-renders with each piece: the navbar, the dock,
    the sidebar, and the dashboard page with its charts.
  - `BI-32` is this symptom, seen by the owner: a reference-line label keeps moving while the
    assistant answers.
- **G6. Scrolling jumps ahead of the text.**
  - `useStickToBottom` starts a new `behavior: "smooth"` scroll whenever `tail` changes
    (`chat-messages.tsx:542-546`).
  - `tail` includes the received length (`draft.length`, `:262`), not the shown length. The
    chat scrolls once per piece, overlapping smooth scrolls fight each other, and the scroll
    runs ahead of the revealed text.
- **G7. Nothing tells a proxy not to buffer.**
  - The stream carries no `X-Accel-Buffering: no` and no `Cache-Control: no-transform`
    (`routes/ai/mod.rs:1219-1222`; grep finds neither anywhere).
  - A reverse proxy such as nginx buffers a proxied response by default, so a customer who
    puts one in front of the console gets the whole answer at the end.
  - The console's own hop is the Next.js rewrite `/api/:path*` (`next.config.ts:46-58`).
    Whether its default compression holds back an `application/x-ndjson` body is **not
    verified**; T5 measures it.

## 1. Decisions

- **D1. Nothing is rebuilt.** Rename, delete, reopen and the NDJSON stream stay as they are.
  The plan closes G1 to G7.
- **D2. The open conversation's title is the hook's own state** (`sessionTitle`). It is set
  by `loadSession` from `session.title`, by `persist` from the save's `title`, and by
  `renameSession` when it renames the open one. `newChat` clears it. The title bar reads it,
  and falls back to "New conversation" only with no id.
- **D3. Load failures are plain sentences.** 404: "This conversation could not be found. It
  may have been deleted." Anything else: "This conversation could not be loaded. Try again."
  The API's body text is never shown. The API keeps its 404 and its body.
- **D4. Over the size cap, drop the bulk, oldest first, not the steps.**
  - First drop `tools[].result` and `reasoning` from the oldest message onward, until the
    body fits.
  - Every step keeps `tool`, `ok` and `args`, plus `"resultDropped": true`.
  - Only if it still does not fit does today's trim apply.
  - The cap stays 200,000 bytes.
  - The console shows a step with `resultDropped` as its label and outcome, with "details not
    kept for long conversations" in place of the result *(planner default)*.
- **D5. The reveal keeps up, and the end does not jump.**
  - The pace becomes proportional to the backlog: each frame shows
    `max(1, ceil(behind × dt / 0.3 s))` characters, with a floor of 30 characters a second.
    The reveal then trails the received text by about 0.3 s at any model speed
    *(planner default)*.
  - At `done`, when the checked answer starts with the text already shown, the new message
    goes on revealing from that point at the same pace, instead of appearing whole.
  - When the checked answer differs (citations changed it, or the `verifying` round rewrote
    it), it replaces the draft at once, as today. Showing the checked text is the point
    (`citations::annotate_answer`).
  - Reduced motion still shows everything at once.
- **D6. The stream lives in its own context.**
  - `draft`, `liveReasoning` and `progress` move to a second provider value
    (`CopilotStreamContext`, read by `useCopilotStream()`). Only the message list and the
    status pill read it.
  - The main value is memoized, so a piece re-renders only what shows it. `useCopilot()`
    keeps its shape minus those three fields; the callers that read them (find them by
    `grep`) move to `useCopilotStream()`.
  - This should also fix `BI-32`'s symptom. `BI-32` is closed only if the owner sees it gone;
    this plan does not claim it.
- **D7. The scroll follows what is shown.**
  - `useStickToBottom` is keyed on the shown length while streaming. It uses
    `behavior: "auto"` (instant) while text is arriving, and `smooth` only for a new message
    or "jump to latest".
  - Detaching when the reader scrolls up stays as it is.
- **D8. Two headers on the stream.** `X-Accel-Buffering: no` and `Cache-Control: no-cache,
  no-transform`. The reason goes at the site: a buffering or compressing proxy would hold
  the answer to the end; nginx honours the first, and the second asks any proxy not to
  re-encode. The deployment docs gain one line for operators who run their own proxy.
- **D9. The console's own hop is measured, not assumed.** T5 times the pieces through
  `:3000` (the Next.js rewrite) and straight to the API, with the same stubbed model. If the
  console path batches (fewer chunks, or the first piece more than 200 ms later than direct),
  the developer **stops and reports**. Turning off Next.js compression is a decision for the
  whole console, not for this task.

## 2. Anchors (verified at `9c066426`)

- API `rust/crates/lakehouse-api/src/routes/ai/mod.rs`:
  - stream: `model_round_once` `:740`, `stream_chat` `:1182` (headers `:1219-1222`);
  - sessions: `CREATE_CHAT_SESSION_TABLE` `:1625`, `session_filter` `:1731`, `sessions_get`
    `:1752`, `session_row` `:1855`, `sessions_save` `:1964` (locked title `:1995-2006`, the
    200,000-byte trim `:2008-2020`), `sessions_rename` `:2056`, `sessions_delete` `:2129`.
- Routing `routes/mod.rs:1469-1472`; `policy.rs:749-754`.
- Model client `rust/crates/lakehouse-llm/src/stream.rs:273-298`.
- Console `src/features/copilot/`:
  - `use-copilot.ts`: `Msg` `:18`, `RECENT_SESSIONS` `:86`, `persist` `:274-284`, `send`
    `:312-432` (deltas `:350-356`, status and tool clear the draft `:364-375`, `done`
    `:392-409`, `finally` `:425-431`), `loadSession` `:567-581`, `removeSession` `:583`,
    `renameSession` `:597`, the returned value `:613-622`, `CopilotProvider` `:628-631`;
  - `copilot-page.tsx`: `SessionUrlSync` `:26-49`, `activeTitle` `:110`;
  - `chat-messages.tsx`: `tail` `:262`, `StreamingAnswer` use `:404`, `useSmoothText`
    `:465-493`, `StreamingAnswer` `:501-509`, `useStickToBottom` `:526-556`;
  - `copilot-history-page.tsx:183-192`, `:234`; `session-parts.tsx:108`, `:162`, `:209`;
    `tool-step.tsx` `ToolStep` `:34`; `mini-markdown.tsx` `MiniMarkdown` `:171`.
- `src/lib/ndjson.ts` `readNdjson`; `src/lib/stream-markdown.ts` `settleStreamingMarkdown`;
  `next.config.ts:46-58` (rewrites).
- Tests: `rust/crates/lakehouse-api/tests/ai_chat_stream.rs`
  (`a_streamed_chat_sends_deltas_then_the_checked_answer` `:114`);
  `src/features/copilot/use-copilot.test.tsx` (stubs `/api/ai/sessions` at `:115`).
- `BACKLOG.md:66` (`AI-18`), `:294` (`BI-32`).

## 3. Tasks (one commit each, in order)

Rust first (T1, T2), then the console. Cargo runs in the shared `CARGO_TARGET_DIR`, through
the machine's wrapper, with Docker for the `spin_up` tests (`docs/core/HANDOFF.md:144`).

- **T1. The stream's headers (D8) and the sessions' tests.**
  - The two headers, with their reason.
  - A test in `ai_chat_stream.rs` that the streamed response carries both.
  - A new `tests/ai_sessions.rs` (spin-up with ClickHouse; a second user from the helpers the
    other route tests use):
    - rename then save keeps the renamed title;
    - delete, then `GET ?id=` is 404;
    - a save to a deleted id is 404 and does not bring the conversation back;
    - another user's `GET`, `PATCH` and `DELETE` are 404 and change nothing;
    - the list is newest first and holds only the caller's.
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D
    warnings`; `cargo test -p lakehouse-api --test ai_chat_stream`; `cargo test -p
    lakehouse-api --test ai_sessions`.
- **T2. Keep the steps of a long conversation (D4).** The trim in `sessions_save`, as a pure
  function with lib tests:
  - a 300,000-byte conversation saves under 200,000 with every step's `tool` and `ok`;
  - the newest messages keep their `result`;
  - a conversation of plain text over the cap still falls back to today's trim;
  - a body under the cap is byte-for-byte unchanged.
  - *Accept:* `cargo test -p lakehouse-api --lib ai::` (filter on the new tests' names) and
    `cargo test -p lakehouse-api --test ai_sessions`; clippy for the crate.
- **T3. Title and load errors (D2, D3), and a dropped result shown (D4, console half).** The
  sessions calls move into `src/services` (a `ChatSessionService` contract and client), so
  `use-copilot.ts` stops calling `apiFetch` on `/api/ai/sessions` directly (`AGENTS.md`:
  contracts → clients → `services/index.ts` → features).
  - Tests in `use-copilot.test.tsx`:
    - loading a conversation that is not among the recent 20 shows its title;
    - renaming the open one changes the title bar;
    - a 404 shows D3's sentence and not the body's text;
    - deleting the open one starts a new chat.
  - A `tool-step` test that a step with `resultDropped` renders its label and the note.
  - *Accept:* `bun test src/features/copilot src/services`; `bun run typecheck`; `bun run
    lint`.
- **T4. Smooth streaming (D5, D6, D7).**
  - Tests:
    - `useSmoothText` with fake timers: at 300 characters a second for 5 s the shown text
      trails by at most about 0.3 s of text (≤ 120 characters);
    - at `done` with an answer that starts with the shown text, no frame jumps by more than
      one frame's step;
    - with a different answer, the answer replaces the draft at once;
    - with reduced motion, all of it shows at once.
  - A render-count test: a consumer of `useCopilot()` that does not read the stream does not
    re-render while 50 deltas arrive.
  - `useStickToBottom`: no smooth scroll is started per delta.
  - *Accept:* `bun test src/features/copilot`; `bun run typecheck`; `bun run lint`.
- **T5. Measure (D9), and the result doc.**
  - Stub model: the `ai_chat_stream.rs` stub, or a small local SSE server that sends 50
    pieces 50 ms apart.
  - Run it with the console (`bun run build && bun run start`, or the compose `console`
    service) and the API.
  - Record with `curl -N`, one timestamp per line, for `POST /api/ai/chat {stream: true}` on
    `:3000` and on the API port:
    - the time of the first piece;
    - the number of chunks;
    - the largest gap between them.
  - Then, if a model endpoint is configured, one real answer of about 1,500 characters
    through the console in a browser: the pace and whether the end jumps.
  - Write `docs/plans/AI-24-STREAM-RESULT.md`: the method, the numbers, the date, the commit.
    No hostnames or ports of a real deployment (rule 12).
  - If the console path batches, stop and report (D9).
  - *Accept:* the result file exists with both measurements, or says *not verified* with the
    reason for any part not run.

Full verification block (`AGENTS.md`) once before the handoff: the Rust and TypeScript
lines. No Python, no compose change.

## 4. Out of scope

- A rename moving the conversation to the top of History. `sessions_rename` writes a new row
  version, and `updated_at` is the version and the sort key at once
  (`ReplacingMergeTree(updated_at)`, `:1625-1629`). Keeping the order needs a separate
  last-message time. Not asked; record it if the owner minds.
- Rename and delete in the header's history menu. The feature page puts both on the History
  page only (`copilot-chat.md:66-67`).
- Server-side batching of deltas, SSE instead of NDJSON, and per-paragraph memoized markdown.
  None is a gap shown by the code; T5's numbers decide whether one is needed.
- Turning off Next.js compression (D9).
- Tenant scoping of `console.chat_session` (it is per user, and a conversation is private),
  and binding the session SQL's values (built with `esc()`, `:1662`). Both are existing
  shapes this task does not touch. The second is a `SEC-` candidate for the planner to
  backlog.
- Sharing a conversation (open question 4 of the feature page).

## 5. Handoff (developer appends below)

## 6. Review (planner appends below)


**Order (planner, 2026-11-02):** runs AFTER `AI-23` (both touch the assistant chat), on the same machine.
