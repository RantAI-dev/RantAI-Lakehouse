# DOC-2: the step-by-step user guide, linked from the landing page

Backlog: `DOC-2` (Task, Docs, P3). The product owner decided on 2026-10-30 that the planner
writes the guide and that the landing page links to it. Base: `main` at `fdf357ce`.

Planner: Claude Opus. Developer: a different agent (`AGENTS.md`). The developer does not change
a decision below. If the code disagrees with the plan, stop and report it.

## 1. Decisions (made; do not change)

- **D1. The guide** is `docs/core/reference/user-guide.md`. The planner rewrote it as
  click-by-click steps, every label read from the source at `fdf357ce`. The running-console
  walk-through is still open, and the guide says so at the top.
- **D2. The link target** is the guide on GitHub, because the landing page is public and the
  repository is public: `https://github.com/RantAI-dev/RantAI-Lakehouse/blob/main/docs/core/reference/user-guide.md`.
- **D3. Where the link goes.**
  - Add `guide` to `LINKS` in `src/features/landing/landing-copy.ts`.
  - Add `{ label: "User guide", href: LINKS.guide }` to `FOOTER.connect`, before `GitHub`.
  - The footer already opens `http` links in a new tab (`landing-footer.tsx`). No other
    change is needed there.
  - The top navigation (`NAV_LINKS`) is unchanged. It only holds in-page anchors.

## 2. Tasks

**T1. The landing link** (`src/features/landing/landing-copy.ts`). D2 and D3. Cite `DOC-2`.
- If a test asserts the footer links, update it. Otherwise add none.
- *Accept:* `bun run typecheck`, `bun run lint`, and `bun test src/features/landing`.

## 3. Out of scope

- Serving the guide inside the console.
- Changing the landing copy.
- Any other section of the landing page.

## 4. Handoff (developer appends below)

### Handoff (developer, 2026-10-31)

**Commits**

- `6ef93d3b` — `feat(landing): link the user guide from the footer (DOC-2 T1)`. Touches
  `src/features/landing/landing-copy.ts` only: `LINKS.guide` added (DOC-2 comment at the
  site) and `{ label: "User guide", href: LINKS.guide }` added to `FOOTER.connect` before
  the GitHub entry. No test file changed: nothing under `src/` asserts the footer links
  (grepped `FOOTER`, `LINKS`, `landing-copy`, `LandingFooter`, `footer` across `*.test.ts*`),
  and `src/features/landing` holds only `scramble.test.ts` and `scroll-timeline.test.ts`.

**Checks run (foreground, on `6ef93d3b`)**

- `bun run typecheck` — exit 0, no output.
- `bun run lint` — exit 0: 0 errors, 11 warnings, all pre-existing and in unrelated files
  (`src/components/ui/sidebar.tsx`, `src/features/alerts/alerts-page.tsx`,
  `src/features/auth/login-page.tsx`, `src/features/catalog/open-format-card.test.tsx`,
  `src/features/copilot/use-copilot.ts`, `src/hooks/use-data-table.ts`,
  `src/lib/dashboard-specs.ts`, `src/lib/uploads.test.ts`, `src/services/http.ts`). Nothing
  in `src/features/landing/`.
- `bun test src/features/landing` — 19 pass, 0 fail, 2 files.

**Not verified**

- The rest of the verification block in `AGENTS.md` was not run: this change touches no
  Rust, Python, Dagster or compose file, and `AGENTS.md` ("Keep build time down") scopes
  each commit to the languages it touched.
- The worktree had no `node_modules`, so `bun install --frozen-lockfile` (739 packages) ran
  once before the checks. No lockfile change: `git status` was clean apart from
  `landing-copy.ts`.
- The link was not opened in a browser; the footer renders `http` links with
  `target="_blank"` (`landing-footer.tsx`), unchanged by this task.

## 5. Review (planner appends below)

### Review round 1 (planner, 2026-10-31, at `ef7b5fb8`)

- T1 matches D2 and D3:
  - `LINKS.guide` points at the guide on `main`, with a `DOC-2` comment;
  - "User guide" sits first in `FOOTER.connect`;
  - `NAV_LINKS` and `landing-footer.tsx` are unchanged.
- No test asserts the footer links, so adding none is right.
- I re-ran the checks on `ef7b5fb8`, in the foreground:
  - `bun run typecheck`: clean;
  - `bun run lint`: 0 errors, 11 warnings, all pre-existing;
  - `bun test src/features/landing`: 19 pass, 0 fail.

**Open: no BLOCKER, no SHOULD-FIX.** `DOC-2` stays In Progress until the guide is walked through on
a running console.
