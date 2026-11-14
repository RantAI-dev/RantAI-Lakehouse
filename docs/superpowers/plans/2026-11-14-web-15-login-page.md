# WEB-15: the sign-in page shows the product, plan

| | |
| --- | --- |
| Backlog | `WEB-15` |
| Spec | `docs/core/specs/web-15.md` |
| Feature page | `docs/core/features/login-page.md` |
| Base | `origin/main` at `40c876d9` |
| Branch | `feat/web-15-login-page` (worktree `/home/hv/mtharreq/lakehouse-web15`) |
| Planner and reviewer | Claude Opus (hub session) |
| Developer | an Opus developer agent (console design work, AGENTS.md exception) |

## 1. Decisions already made

- **The design is mockup A**, chosen by the owner on 2026-11-14. The source is on branch
  `design/chart-builder-mockups` at `5e7b9b8c` (worktree `/home/hv/mtharreq/lakehouse-mockups`):
  - `src/features/mockups/login/mockup-a.tsx` (layout, the sample dashboard);
  - `shared.tsx` (the form: labels, eye, Caps Lock, error shake, loading);
  - `fonts.ts` and `inter-tight/` (Inter Tight, OFL);
  - `login-mockups.css`.
  Read them with `git show 5e7b9b8c:<path>`. Screenshots are in
  `/home/hv/mtharreq/lakehouse-mockups/.mockup-shots/login/a-*.png`.
- **Port it, do not import it.** Move what A needs into `src/features/auth/` (for example
  `login-showcase.tsx` for the sample dashboard, the form parts into `login-page.tsx` or a
  `login-form.tsx`). Leave out the mockup-only parts:
  - the demo bar (Mockup label, Error/Loading/With SSO switch, the link back to the index);
  - the `/mockups/login` entry in `PUBLIC_PREFIXES` (it stays only on the mockup branch).
- **Sign-in behaviour does not change.** Keep every request, redirect, `next` handling,
  `NEXT_PUBLIC_SSO_ENABLED`, `SsoNotice`, the bootstrap and setup paths, the element ids
  (`#login-email`, `#login-password`, `#login-error`) and the accessible names. The existing tests
  for the login page keep passing unchanged, except where they assert old markup.
- **The form comes first.** The sample dashboard and its chart code are a dynamic import with
  `ssr: false`, rendered after the form, so typing never waits for ECharts. Until it loads, its
  area shows the panel background only (no skeleton imitating content, DESIGN.md §3).
- **Motion:** entrance once; the 9 s redraw runs only while the panel is visible
  (IntersectionObserver plus `document.visibilityState`); nothing moves under
  `prefers-reduced-motion`.
- **Sample data:** fixed values in the code, labelled "Sample data", no API call. The palette is
  the console's blue chart palette (as in the mockup's final version).
- **DESIGN.md** (`/home/hv/mtharreq/DESIGN.md`) applies, especially: no sparkles icons, numbers
  with thousands separators, controls at the console's one height (32 px for buttons and inputs
  in the app; the sign-in form's larger fields from the mockup are fine because it is a form page
  of its own), the primary button white on a deep blue.
- **Out of scope:** the setup (first admin) page, the change-password page, the invite page.

## 2. Anchors (at `40c876d9`)

- `src/app/login/page.tsx`, `src/features/auth/login-page.tsx` (377 lines: `SsoButton` :45,
  `nextPathFromQuery` :79, `LoginPage` :88, `onSubmit` :117, the inputs :261 and :286, the error
  :341).
- The tile renderer the mockup used: `TileBody` (grep in `src/features/dashboards`).

## 3. Tasks (one commit each, in order)

1. **T1, the form.** Port A's form into the real page with the real behaviour; logo above the
   title; error shake and password select; loading button; Caps Lock; eye with tooltip. Tests:
   the existing login tests, plus the error state (message shown, password selected) and Caps Lock.
2. **T2, the sample dashboard.** `login-showcase.tsx`, dynamic, after the form; visibility-gated
   redraw; reduced motion; phone variant (one tile above the form). Test: the form is in the DOM
   and usable without the showcase; reduced motion renders no animation.
3. **T3, screenshots** at 1366×900 and 390×844, light and dark, plus the error state, into
   `docs/plans/web-15-screens/`. Compare with the mockup A shots yourself.
4. **T4, handoff** in section 6.

## 4. Machine rules (owner, 2026-11-14)

- No Rust. Never the full `bun run test` or `bun run lint`.
- First `bun install --frozen-lockfile` in this new worktree (under the cap below).
- Each command with `nice -n 15` under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0`,
  one at a time, after `free -m` shows at least 3 GB available.
- Checks: `bun run typecheck`, `bun x eslint` on the changed files, `bun test` on the auth tests.
- Screenshots: a brief dev server from this worktree on **28622**, only when no other Next dev
  server is running (`ss -ltn | grep -E ':286(20|22)'`), plus one headless Chrome with its own
  profile. Stop both by PID right away. Never touch 28605 or the QA containers.
- Commit as mtharreqq. Do not push.

## 5. Review

(planner)

## 6. Handoff

### Slice 1 (the whole plan), developer, 2026-11-14

Branch `feat/web-15-login-page`, base `40c876d9`, not pushed.

| Task | Commit | What |
| --- | --- | --- |
| T1 | `1009469f` | The form of mockup A in `login-page.tsx`; Inter Tight under `src/app/fonts/inter-tight/` (+ README row), `login-fonts.ts`, `login.css` (the shake) |
| T2 | `2b17c5a2` | `login-showcase.tsx` (sample dashboard, loop, phone tile), wired into the page; `login-hero.tsx` removed |
| T3 | `b56d9858` | `docs/plans/web-15-screens/` (7 PNGs) |
| T4 | this commit | This handoff |

**Commands run** (each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`,
one at a time, with at least 4.5 GB available), last on the tree of `b56d9858`:

- `bun install --frozen-lockfile`: 678 packages installed.
- `bun run typecheck`: no errors.
- `bun x eslint src/features/auth/login-page.tsx src/features/auth/login-page.test.tsx src/features/auth/login-showcase.tsx src/features/auth/login-showcase.test.tsx src/features/auth/login-fonts.ts`:
  0 errors, 1 warning (`@next/next/no-location-assign-relative-destination` on the SSO button's
  `window.location.assign("/api/auth/oidc/start")`, the same statement `main` has; it goes to an API
  route, not a page).
- `bun test src/features/auth src/app/fonts.test.ts`: 32 pass, 0 fail, 102 `expect()` calls, 6 files.
  The five lockout/notice tests in `login-page.test.tsx` pass unchanged. New: 4 in `LoginPage form (WEB-15 T1)`
  (refusal: message, `aria-describedby`, focus and full selection of the password, the shake class;
  the refusal folds away on typing; Caps Lock on, off, and off on blur; the eye), 1 in
  `LoginPage without the sample dashboard (WEB-15 T2)` (form signs in while the preview never loads),
  3 in `login-showcase.test.tsx` (reduced motion desktop and phone: still render mode, final KPI,
  no loop; with motion the 9 s interval exists only while on screen and the tab is visible).
- Screenshots: `bun --bun next dev -p 28622` with `RUST_API_URL` pointed at a closed port (so
  `/api/auth/me` fails and the form shows), one headless Chrome with its own profile, driven over
  CDP; the refusal was a 401 `{"error":"invalid_or_expired"}` answered by the browser (`Fetch`
  domain), so no account and no password was used. Both processes were stopped by PID afterwards,
  and the `AGENTS.md` block `next dev` writes was reverted, not committed.

**Compared with the mockup A shots** (`.mockup-shots/login/a-*.png`): light and dark 1366×900, phone
390×844 and the dark refusal match in layout, sizes and colours; the differences are the demo bar
(left out, plan §1) and the refusal's sentence (below). Measured in the shots: no horizontal scroll
(`scrollWidth` 1366 and 390), the Sign in button's bottom at 648 px (desktop) and 652 px (phone, of
844), so the form is fully on the first phone screen; after the refusal the password is focused
with selection `[0, 16]` of 16 characters.

**Choices the planner should look at** (none changes a request, redirect or id):

1. **The refusal's sentence stays "Invalid username/email or password."** The mockup shows "Wrong
   username or password." and its source says the owner asked for the shorter sentence. Plan §1 says
   the behaviour and the existing tests stay, and a test asserts the old sentence, so I did not
   change it. If the owner's sentence is wanted, it is one string and one test line.
2. From the mockup, taken as markup: the subtitle "Use your account to continue." (was "Use your
   account credentials to continue."); no `••••••••` placeholder in the password field; inputs at
   16 px on a phone (no iOS zoom); `autoCapitalize="none"` and `spellCheck={false}` on the username;
   typing in either field closes the refusal; the logo image is decorative (`alt=""`, the title
   names the product; before, the light logo had `alt="Rantai Lake"`).
3. Added for DESIGN.md §3 ("no button is silent"): the SSO button shows a spinner and
   "Redirecting…" when pressed, and resets when the page comes back from the browser's page cache.
   `onSubmit` returns early while a sign-in is in flight.
4. `autoFocus` on the username stays, as on `main` (the mockup had none).
5. The preview mounts after `requestIdleCallback` (300 ms timeout where it is missing, Safari), and
   only the variant the layout shows is mounted (`matchMedia("(min-width: 1024px)")`), so a phone
   never runs the hidden desktop dashboard.
6. **Left without a user:** `src/components/ui/molten-metal.tsx` and the `ogl` dependency were used
   only by the removed `login-hero.tsx` (its doc comment still names that file). I did not remove
   them: dropping a dependency touches the lockfile and is outside this plan. Suggest a follow-up.
7. Tests: `login-page.test.tsx` imports the page after its mocks (`await import`), because
   `next/font/local` has no runtime default export and a static import links it before
   `mock.module` applies; it also mocks `next/dynamic` to a never-loading component. The only other
   `next/dynamic` user is `landing-page.tsx`, which no test renders.
8. Inter Tight: the file is byte-identical to the landing site's (`site/src/fonts/inter-tight/`,
   SHA-256 in `src/app/fonts/README.md`); that folder is not on `main` yet, so the README names it
   by path and hash.

**Not verified:**

- The full `bun run test` and `bun run lint`, and `next build` (machine rules, §4).
- A real sign-in against an API (success path and `next` redirect): no API ran; the code paths are
  unchanged and covered by the existing tests only for the failure cases.
- iPhone Safari on a real device; Caps Lock in a real browser (unit test only); the charts' hover
  tooltips (not exercised in the shots); `NEXT_PUBLIC_SSO_ENABLED=true` (no screenshot).
- The 9 s redraw in a browser was not timed; the loop's gating is proven by the unit test. The phone
  refusal shot shows the KPI mid-count (1,572,507 of 1,572,524), consistent with a redraw in
  progress when it was taken.

### Slice 2 (owner follow-ups T1 to T5, 2026-11-14), Opus developer

Branch `feat/web-15-login-page`, base `36e679ea`, not pushed.

| Task | Commit | What |
| --- | --- | --- |
| T1 | `def7768d` | Lake depth contours behind the preview: `scripts/login-contours.mjs` draws `public/login-contours-{0,1,2}.svg` (seeded field, marching squares; ~37 KiB each, ~8.5 KiB gzipped); `login-contours.tsx` shows them as CSS masks over a theme colour (`login.css`), 36 s opacity drift gated by `use-in-view.ts` (now shared with the showcase loop), static under reduced motion and without JS |
| T2 | `bc54a08d` | KPI tile: no repeated "2024", change on one line ("+19.3% vs 2023", amount in the tooltip), trend from the monthly bars; x-axis labels level (`axes.x.rotate: 0`) |
| T3 | `40de7ee6` | Favicons redrawn from the official SVGs at 196:167: `icon-light-32`, `icon-dark-32`, `apple-touch-icon` (white), plus `android-chrome-192/512` and `src/app/favicon.ico`, which had the same stretch |
| T4 | `11099478` | Sign in button `#050A2F`/white (light), `#F1F4F9`/navy (dark), offset focus ring, keeps colours while busy; login page only |
| T5 | `1f4eee44` | emil-design-eng motion pass (press scale, field focus ease, eye tooltip delay and icon swap, asymmetric error box, Caps Lock entrance, label crossfade, showcase entrance) |

**Commands run** (each under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15`,
with at least 4.4 GB available), last on the tree of `1f4eee44`:

- `bun run typecheck`: no errors.
- `bun test src/features/auth/login-page.test.tsx src/features/auth/login-showcase.test.tsx src/features/auth/login-contours.test.tsx`: 16 pass, 0 fail, 48 expect() calls.
- `bun x eslint` on every changed TS/TSX file and `scripts/login-contours.mjs`: 0 errors, 1 warning
  (the existing `no-location-assign-relative-destination` on the SSO button).
- Dev server (port 28699) + headless Chrome: screenshots at 1366x900 and 1920x1080, light and dark,
  reduced motion, a refused sign-in; a CDP probe confirmed the eye tooltip is absent at 150 ms and
  shown at 650 ms, the pressed eye button computes `scale: 0.97`, and the three contour layers run
  with opacities summing to 1.

**Not verified:** the full `bun run test`/`bun run lint` and `next build` (machine rules);
iPhone Safari; the contour drift and the motion pass judged by eye over a full cycle (only sampled by
the probe); Safari's support of the mask stack (`mask-composite: intersect` is supported since
Safari 15.4; not checked on a device).

### Slice 3 (owner follow-ups T6 to T9, 2026-11-14), Opus developer

Branch `feat/web-15-login-page`, base `f4669f5c`, not pushed.

| Task | Commit | What |
| --- | --- | --- |
| T6 | `1b667ddb` | Contours ripple outward: each drawing lives 9 s (fade in, spread 3.5 % from the deep point with a few px of drift, fade out), one born every 3 s; transform/opacity only; edge fade moved into each layer's mask (container units); drawings 2000 x 1704; dark lines lake blue |
| T7 | `2dcadaf3` | Panel solid navy `#0B1440` (light) / `#0A1030` (dark), lighter faint contours, white caption, soft navy-black lift on the desktop card |
| T8 | `bd130f43` | Title centred over the form column |
| T9 | `3f8a384f` | First-look sequence under 900 ms (contour "drop", form stagger 50 ms, caption, card rise), gated by `.login-first` for the first 900 ms |

**Commands run** (each under the 3 GB scope, at least 3 GB available), last on the tree of `3f8a384f`:

- `bun run typecheck`: no errors.
- `bun test src/features/auth/login-page.test.tsx src/features/auth/login-showcase.test.tsx src/features/auth/login-contours.test.tsx`: 16 pass, 0 fail, 48 expect() calls.
- `bun x eslint` on the changed TS/TSX files: 0 errors, 1 warning (the existing SSO `location.assign`).
- CPU of this session's headless Chrome only (software compositing, no GPU on the VM), 1366x900,
  4 samples x 3 s: contours moving, showcase hidden 78 % of a core with the fade on the container,
  45 % after moving it into each layer's mask; reduced motion 0.2 %.

**Not verified:** CPU in a GPU-composited browser (expected far lower; none on the VM); the full
`bun run test`/`bun run lint` and `next build`; iPhone Safari; the reveal timed in real time (frames
were taken with the CSS animations slowed 10x, while the dashboard's JS loading ran at full speed).
