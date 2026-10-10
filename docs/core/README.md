# Core product documents

The product owner's view of RantAI Lakehouse. Kept small on purpose, so it
stays current.

| File | What it is | When you touch it |
| --- | --- | --- |
| [PRODUCT.md](PRODUCT.md) | The one product document: what it is, what is in it, what it lacks, what blocks production, what is next, open decisions | When a feature is accepted; before a release |
| [BACKLOG.md](BACKLOG.md) | The master list of work (ID, kind, item, module, priority, status, PR); the Lark base mirrors it | Weekly, and on every merge |
| [specs/](specs/) | One spec per backlog task: target numbers against the best competitor, acceptance checklist, what is left out | When a task is added or its targets change |
| [features/](features/) | One page per feature: what the user can do, decisions, limits, acceptance checklist. Written only when a feature is built | Starting and finishing a feature |
| [HANDOFF.md](HANDOFF.md) | Current state for a planner or reviewer taking over: what is merged, what is in flight, how to review, what to watch for | When the planner role changes hands |
| [reference/](reference/) | Parked material: security overview, support model, release policy, competitive comparison, user guide, glossary | When a customer or contract needs it |

## How a feature moves

1. It is in `BACKLOG.md` with Kind **Feature**, a Module, a Priority and
   Status **Planned**.
2. It gets a page in `features/` (copy `_TEMPLATE.md`). The planner agent
   may draft it; the product owner answers its decisions and names the
   Acceptor in the base.
3. The agents plan, build, review and merge it (`AGENTS.md`). On merge its
   Status becomes **In Acceptance**, with the PR recorded.
4. The product owner runs the acceptance checklist on that page.
5. `PRODUCT.md` and `BACKLOG.md` are updated (Status **Released**), and the
   base row is updated to match.

Merged is not accepted. A feature counts as done at step 4.

**The repo and the base.** `BACKLOG.md` is the master list the agents read.
The Lark base mirrors it: status, PR, dates and QA-case results are kept in
the repo and copied to the base. The base adds only what a public repo must
not hold: the people (Owner, Acceptor). When the repo changes, the base row
is updated to match; nothing the repo holds is changed only in the base.

## Rules

- Nothing invented. A status is Have, Partial, Missing or *Not verified*. No
  made-up numbers or dates. A target in `specs/` either cites a competitor's
  documented number or is marked *(proposed)* until the product owner signs
  it on the feature page.
- Say where a claim comes from.
- No customer names, hostnames or secrets.
- Engineering detail stays in `docs/ARCHITECTURE.md`, `docs/OPERATIONS.md`,
  `docs/adr/` and `docs/plans/`. Link to it; do not copy it.

## About `reference/`

Written 2026-10-02 before the positioning was settled, so parts of it lean
on "runs on the customer's premises" as the main message. That is now one
option, not the headline (see `PRODUCT.md` section 1). Read those files for
their facts; rework the framing before giving one to a customer.
