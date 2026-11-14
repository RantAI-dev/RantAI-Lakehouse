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

(developer)
