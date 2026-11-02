# BI-42: image and web cards print in PDFs and chart images again, under SEC-47's policy

Backlog `BI-42` (Task, Dashboards, P2). Base: `main` at `9c066426`. Planner: Claude Opus.
Developer: another agent (`AGENTS.md`). Source: phase 4–6 owner QA 2026-10-25. Security plan read:
`docs/superpowers/plans/2026-10-23-sec-47-csp.md`.

## 0. What is wrong, with evidence

- SEC-47 gives every console page a `Content-Security-Policy` whose `img-src` and `frame-src` allow
  only `'self' data: blob:` plus content sites (`src/lib/csp.ts:186`). A console page gets the
  deployment's list from `GET /api/settings/reporting` with the visitor's cookie; an embed page gets
  **its own board's** sites from `POST /api/embed/frame` (`src/proxy.ts:114-134`, console branch `:128-134`; SEC-47 decision 3).
- The print view is the exception. `src/proxy.ts:114-119`:
  `if (isRenderPath(pathname)) { policy = contentSecurityPolicy({ nonce, development }) }` — no
  content site, because Gotenberg has no session (comment `:115-118`, module doc `:25-27`). So under
  `/render/dashboard/<token>` (the PDF, `BI-1`) and `/render/chart/<token>` (the chart PNG, `BI-19`
  D13) the policy is `img-src 'self' data: blob:; frame-src 'none'`.
- The cards themselves still decide they may load: `ImageCard` / `WebCard`
  (`src/features/dashboards/content-cards.tsx:22-60`) check the address against
  `reporting.contentSites`, which the print view's payload carries (`payload_for` adds the
  formatting, `routes/dashboard.rs:292`, `routes/settings.rs` `add_formatting`). They mount the
  `<img>` / `<iframe>`, and the browser refuses the request by policy. An image card prints empty
  and a web card prints a refused frame. `src/proxy.test.ts:33-46` pins that the chart view's policy
  equals the PDF view's, i.e. it pins this gap.
- A second, smaller gap behind the first: in the print view an image card does not take part in
  "ready". The view sets `__lakehouseRenderReady` when every **chart** has drawn
  (`render-mode.tsx:5-24`, `render-view.tsx:61-65`); an `<img loading="lazy">` (`content-cards.tsx:35`; the web card's frame `:54` is lazy too)
  may not even be requested before Gotenberg captures. Once the policy allows the site, the card can
  still print blank. *Not verified* in a real Gotenberg run; T5 checks it.

## 1. Decisions

- **D1. SEC-47's protection is kept, exactly as for an embed.** The print view's `img-src` /
  `frame-src` get the content sites of **that board's own image and web cards that the person the
  token names can see now**, and only those still on the deployment's list
  (`settings::content_sites_of_cards`, the one helper the embed's payload and frame answer use,
  `PR #233 review SHOULD-FIX 1`). For a chart token (`RenderClaims.chart`), only that chart's site.
  Never the whole deployment list, and never a site taken from the token without the API verifying
  it: the console cannot check the token's signature (the secret is the API's), so nothing in a
  header may come from the token unverified (principle 3). `frame-ancestors` stays `'none'`,
  `connect-src` stays `'self'`, and the nonce rules are unchanged.
- **D2. The proxy asks the API with the render token, through the existing frame route**
  *(planner default)*. `POST /api/embed/frame` takes a third key, `{ "render": "<token>" }`, next
  to `jwt` and `token`. Reasons: no new public route (`exactly_fourteen_public_entries_exist`
  stays 14, `policy.rs:1336-1361`), and the proxy keeps one route for "what may this page load".
  For a render token `origins` is always `[]` and the proxy ignores it anyway (D4). A body with more
  than one non-empty key gets the empty answer. *Alternative:* a separate public `POST
  /api/render/frame` (count 15). Same checks, one more public route.
- **D3. The render token is admitted exactly as the data route admits it.** One function,
  `render::admit(state, token) -> Option<(RenderClaims, Principal)>`, extracted from `render::data`
  (`render.rs:69-114`: verify signature and expiry, the person active, `dashboard:read`, downloads
  allowed for their roles), used by `data` and by the frame route (rule 4). Any refusal, an unset
  `RENDER_TOKEN_SECRET`, or a board or chart the person cannot see gives the same
  `{"origins": [], "contentSites": []}` (no oracle, as `board_for_frame` does today,
  `embed.rs:731-760`). A store failure is the fixed 500 the route already sends; the proxy treats
  anything but 200 as "no site".
- **D4. The proxy, for `/render/dashboard/<token>` and `/render/chart/<token>` only**, parses the
  token from the path (as `embedPageToken` does, `src/lib/embed-frame.ts:60`), asks D2 with the
  3-second timeout (`ASK_TIMEOUT_MS`), builds the policy with `contentSites` from the answer and **no
  `frameAncestors`** (so `'none'`). No cache: the answer is per token and a token lives two minutes.
  Any other `/render/*` path, an unparsable token, a timeout or a bad answer → today's policy (no
  site). The `renderHeaders` (`no-store`, `no-referrer`, `noindex`) are unchanged.
- **D5. The cards take part in "ready" in the print view.** Under a `RenderMode`, `ImageCard` loads
  eagerly and reports `chartMounted` at mount and `chartFinished` on `load` or `error`; `WebCard`
  the same on the frame's `load`; a card that renders the "no longer allowed" sentence reports
  finished at once. Outside the print view nothing changes. A card whose image never answers is
  bounded by Gotenberg's own timeout, as a slow chart is today.
- **D6. Assistant parity.** `export_dashboard` (PDF) and the chart PNG path reach the same print
  view, so the assistant's files get the cards with no tool change. No schema change, no fixture
  regeneration. AI-15: none needed (no tool behaviour or argument changes). The AI team is told in
  the PR description.

## 2. Anchors (verified at `9c066426`)

- `src/proxy.ts`: module doc `:25-27`, `apiBase` `:54`, `askFrame` `:59-68`, `isRenderPath` `:84`,
  the render branch `:114-119`, the embed branch `:120-127`, `renderHeaders` use `:144-146`.
- `src/proxy.test.ts:5-46` (the render tests; `:33-46` pins equal policies).
- `src/lib/embed-frame.ts`: `embedPageToken` `:60-71`, `EmbedFrame` `:73-87`, `embedFrame` `:97-118`.
- `src/lib/csp.ts`: `ASK_TIMEOUT_MS` `:48`, `contentSite` `:69`, `contentSecurityPolicy` `:186`.
- `src/features/dashboards/content-cards.tsx:22-60`; `render-mode.tsx:5-24`; `render-view.tsx:61-65`.
- `rust/crates/lakehouse-api/src/routes/embed.rs`: `FrameBody` `:673-678`, `frame_origins` doc
  `:680-708` and body `:709-723`, `frame_answer` `:726`, `board_for_frame` `:736-760`, `board_cards`
  `:766`, `board_content_sites` `:787-801`.
- `rust/crates/lakehouse-api/src/routes/settings.rs:232` (`content_sites_of_cards`).
- `rust/crates/lakehouse-api/src/routes/render.rs`: `RENDER_TOKEN_REFUSED` `:41`, `data` `:69-145`
  (admission `:77-114`, query `:116`), `principal_of` `:194`.
- `rust/crates/lakehouse-embed/src/lib.rs:291` (`RenderClaims`, `chart` `:297-302`).
- `rust/crates/lakehouse-api/src/policy.rs:169`, `:174`, `:1336-1361`.
- `rust/crates/lakehouse-api/tests/embed_tokens.rs:1006-1230` (frame tests), `tests/render_data.rs`.
- `rust/crates/lakehouse-api/src/pdf.rs:271`, `:339` (`waitForExpression`).

## 3. Tasks (one commit each, in order)

- **T1. One admission for a render token (D3), no behaviour change.**
  - Extract `render::admit` from `data`; `data` calls it and keeps every answer it gives today.
  - Tests: `tests/render_data.rs` unchanged and green.
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api --test render_data`.
- **T2. The frame route answers a render token (D1, D2, D3).**
  - `FrameBody.render`; `board_for_frame` gains the branch (more than one key → empty).
  - The sites: the person's `viewer_for`, `list_stored_charts(viewer)`, `board_cards` for the claims'
    board, narrowed to `claims.chart` when set, then `content_sites_of_cards`. `origins: []`.
  - Update the route's doc comment (`:680-708`) and the `POLICY_TABLE` comment at `:170-173` to
    name the render token (the row itself is unchanged).
  - Tests in `tests/embed_tokens.rs`:
    - a good PDF token: the sites of the board's image and web cards the person can see, and
      `origins` empty;
    - a chart token: only that chart's site;
    - a card the person cannot see (a hidden chart on the board) adds no site;
    - an expired token, a bad signature, a disabled person, a person whose downloads are off, no
      `RENDER_TOKEN_SECRET`, and a body with both `jwt` and `render`: each the same empty answer;
    - a site taken off the deployment's list is not answered.
  - `cargo test -p lakehouse-api --lib policy` proves the public count is still 14.
  - *Accept:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
    `cargo test -p lakehouse-api --test embed_tokens`; `cargo test -p lakehouse-api --lib policy`.
- **T3. The proxy (D4).**
  - `renderPageToken(pathname)` in `src/lib/embed-frame.ts` (or `src/lib/render-frame.ts`), and a
    `renderFrame(pathname, ask)` that returns `string[]` content sites and `[]` on every failure.
  - `proxy.ts` render branch uses it; the module doc (`:25-27`) and the branch comment say what the
    render page now gets and why it is still safe (D1).
  - Tests in `src/proxy.test.ts` with a fake fetch:
    - `/render/dashboard/<t>` and `/render/chart/<t>` with an answer of two sites: `img-src` and
      `frame-src` carry them, `frame-ancestors 'none'`, `connect-src 'self'`;
    - an answer with `origins: ["https://evil.example"]`: `frame-ancestors` is still `'none'`;
    - a site with `;` or a space is dropped (the `csp.ts` validator);
    - a timeout, a 500, a non-JSON body: today's policy;
    - the request body is `{ "render": "<t>" }` and carries no cookie.
  - Replace the test at `:33-46` by one that says what is still equal (frame, connect, script and
    the headers), with its reason in the test name (rule 2: the old test pinned the gap).
  - *Accept:* `bun run typecheck`; `bun run lint`; `bun test src/proxy.test.ts src/lib`.
- **T4. The cards in the print view (D5).**
  - Tests in `render-view.test.tsx`: a board with a chart and an image card is not ready until the
    image fires `load` (or `error`); a card refused by `siteAllowed` does not hold the page; outside
    render mode the image stays `loading="lazy"`.
  - *Accept:* `bun test src/features/dashboards/render-view.test.tsx src/features/dashboards/content-cards`.
- **T5. Proof (rule 7; compose is not changed).**
  - `bun run build && bun run start` against a running API, then `curl -sS -D - -o /dev/null` a
    `/render/dashboard/<fresh token>` and quote its `Content-Security-Policy` (with the board's site
    in `img-src`), and the same for `/render/chart/<token>`.
  - The ignored real-Gotenberg test (`pdf_real_gotenberg`, `--ignored`) cannot reach an https
    content site from the container without a TLS test server; if one cannot be stood up, record
    *not verified* and leave the end-to-end check to the owner's QA (a PDF and a PNG of a dashboard
    with an image card from an allowed site).
  - *Accept:* the quoted headers in the handoff, or *not verified* with the reason.
- **T6. Docs.** `docs/OPERATIONS.md` (the CSP section: the print view's sites come from its token,
  like an embed's), `CHANGELOG.md`. *Accept:* docs only.

Full verification block of `AGENTS.md` once before handoff (Rust and TypeScript).

## 4. Out of scope

- Loosening anything else in the policy (`style-src`, `connect-src`, reporting endpoints).
- Caching the render answer (D4).
- Gotenberg's network egress to content sites: a deployment whose Gotenberg cannot reach the site
  prints the card's "could not be loaded" sentence; that is configuration, documented in T6.
- A new public route (D2 alternative).

## 5. Handoff (developer appends below)

## 6. Review (planner appends below)
