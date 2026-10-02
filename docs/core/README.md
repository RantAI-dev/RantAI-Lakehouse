# Core product documents

The product owner's view of RantAI Lakehouse. Kept small on purpose, so it
stays current.

| File | What it is | When you touch it |
| --- | --- | --- |
| [PRODUCT.md](PRODUCT.md) | The one product document: what it is, what is in it, what it lacks, what blocks production, what is next, open decisions | When a feature is accepted; before a release |
| [BACKLOG.md](BACKLOG.md) | The one list of work | Weekly |
| [features/](features/) | One page per feature: what the user can do, decisions, limits, acceptance checklist. Written only when a feature is built | Starting and finishing a feature |
| [reference/](reference/) | Parked material: security overview, support model, release policy, competitive comparison, user guide, glossary | When a customer or contract needs it |

## How a feature moves

1. It is in `BACKLOG.md` under Now.
2. It gets a page in `features/` (copy `_TEMPLATE.md`). The planner agent
   may draft it; the product owner answers its decisions.
3. The agents plan, build, review and merge it (`AGENTS.md`).
4. The product owner runs the acceptance checklist on that page.
5. `PRODUCT.md` and `BACKLOG.md` are updated.

Merged is not accepted. A feature counts as done at step 4.

## Rules

- Nothing invented. A status is Have, Partial, Missing or *Not verified*. No
  made-up numbers, dates or targets.
- Say where a claim comes from.
- No customer names, hostnames or secrets.
- Engineering detail stays in `docs/ARCHITECTURE.md`, `docs/OPERATIONS.md`,
  `docs/adr/` and `docs/plans/`. Link to it; do not copy it.

## About `reference/`

Written 2026-10-02 before the positioning was settled, so parts of it lean
on "runs on the customer's premises" as the main message. That is now one
option, not the headline (see `PRODUCT.md` section 1). Read those files for
their facts; rework the framing before giving one to a customer.
