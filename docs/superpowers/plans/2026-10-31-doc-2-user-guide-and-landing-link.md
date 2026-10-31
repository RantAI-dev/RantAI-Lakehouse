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

## 5. Review (planner appends below)
