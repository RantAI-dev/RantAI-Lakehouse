# REL-10: `lakehousectl` sets `CATALOG_TENANT_ID`, so a fresh install runs its scheduled work

Backlog `REL-10` (new, P1, Delivery). Planner: Claude Opus, 2026-11-04.
Anchors verified at `main` `6e5be45e`. Base branch: this one,
`fix/rel-10-install-sets-catalog-tenant`.

## 0. What is wrong, with evidence

- Every install seeds four tenants: `rust/migrations/0002_seed_identity.sql:31` and the
  rows after it.
- `.env.example:633` ships `CATALOG_TENANT_ID=` empty, and `lakehousectl`
  (`secrets.rs:150` `write_env`) fills secrets and profiles but not this setting.
- With more than one tenant and the setting empty, the API refuses (`.env.example:617-632`,
  `SEC-50`):
  - every job with no user: scheduled exports, alerts and digests, reports, embeds and
    public links, with a 422 that names the setting;
  - the shared catalog and the Dagster-job list, for every principal without `*:*`.
- So an install made by the one-step installer (`REL-2`) runs no scheduled work until
  someone finds a tenant id and edits the `.env` by hand. That fails `PRODUCT.md`'s definition
  of ready for production ("installed for a second customer and run without the product owner
  fixing things by hand").
- `docs/OPERATIONS.md:1867-1879` tells an operator to set it to "the group tenant in
  `0002_seed_identity.sql`", which is `11111111-1111-4111-8111-000000000001` (`0042_tenant_provisioning.sql:74-85`
  already treats it as the default owner of connectors and pipelines).

## 1. Decisions

- **D1. Install writes the group tenant.** `write_env`, when it creates the `.env`, sets
  `CATALOG_TENANT_ID=11111111-1111-4111-8111-000000000001`. The value is one named constant
  in `secrets.rs` with a doc comment citing `0002_seed_identity.sql` and `0042` (why this
  tenant), not a literal at the call site.
- **D2. Update fills it when it is empty, and never changes a value.** An install made by
  0.2.0 has `CATALOG_TENANT_ID=` (empty). The update adds the value before it starts anything,
  in the same place and the same way `set_profiles` / `ensure_release_profiles` add
  `COMPOSE_PROFILES` (`update.rs:416`, `secrets.rs:288`). A line with any value, valid or
  not, is the operator's and is left alone (the API refuses a malformed one at startup).
  Print one line when it was set: `lakehousectl: the shared catalog now belongs to the group
  tenant (CATALOG_TENANT_ID).`
- **D3. An existing `.env` at install is handled as D2.** `write_env` with an existing file
  already adds the profiles when missing (`secrets.rs:151-172`); add the empty-tenant fill
  beside it, through one helper used by both install and update (rule 4).
- **D4. `.env.example` keeps the line empty** (a hand-run `docker compose` from the repo is a
  developer's stack), but its comment gains one sentence: `lakehousectl install` and
  `update` set it to the group tenant when it is empty (`REL-10`).
- **D5. Docs.** `docs/OPERATIONS.md` (the section at `:1867`): the installer now sets it; the
  manual step is only for a stack not made by `lakehousectl`. The REL-2 feature page or
  `docs/core/reference/` install guide, wherever it lists what the installer writes: add it.

## 2. Anchors (at `6e5be45e`)

| What | Where |
| --- | --- |
| Install's `.env` writer | `rust/crates/lakehousectl/src/secrets.rs:150-240` (`write_env`), `:246-283` (profiles helpers), `:288` (`ensure_release_profiles`), `:311` (`read_value`), `:337` (`replace_assignment`) |
| Update's call | `rust/crates/lakehousectl/src/update.rs:416` (`set_profiles`) |
| Install's call | `rust/crates/lakehousectl/src/install.rs:170` |
| The seed and the group tenant | `rust/migrations/0002_seed_identity.sql:31`, `rust/migrations/0042_tenant_provisioning.sql:74-85` |
| The setting | `.env.example:617-633`, `docker-compose.yml:711`, `:2909`, `rust/crates/lakehouse-api/src/config.rs:59-68` |
| The proofs | `.github/workflows/release.yml` jobs `install-proof` (`:1362`) and `update-proof` (`:1573`) |

## 3. Tasks (one commit each, in order; push after each)

**T1. The fill (D1–D3).** The constant, one helper that sets the key when the `.env` has it
empty or missing, its use in `write_env` (new and existing file) and in `update.rs`.
- *Accept:* `cargo clippy -p lakehousectl --all-targets -- -D warnings`;
  `cargo test -p lakehousectl`, with new unit tests:
  - a new `.env` holds the group tenant;
  - an existing `.env` with `CATALOG_TENANT_ID=` gets it, and one with another value (a
    different UUID, and a non-UUID string) is left byte-for-byte unchanged;
  - a `.env` with no such line at all gets the line;
  - a commented-out `# CATALOG_TENANT_ID=…` does not count as set (as the profiles rule says).

**T2. The proofs see it.** In `release.yml`:
- `install-proof`: after install, assert the install's `.env` has
  `CATALOG_TENANT_ID=11111111-1111-4111-8111-000000000001`, and that the API's log has no
  "CATALOG_TENANT_ID" refusal warning at boot (grep the boot warning `main.rs` prints; quote
  the exact string in the handoff).
- `update-proof`: the old install's `.env` is made with the line empty (as 0.2.0 wrote it);
  after the update it holds the group tenant.
Keep each assertion a shell `test` / `grep -q` with a message that names `REL-10`.
- *Accept:* `actionlint` if installed, else *not verified* with the reason; the PR's own
  release dry run and proofs are the real check (they run on every PR).

**T3. Docs and changelog (D4, D5).** CHANGELOG `## [Unreleased]` → `### Fixed`: one entry
naming `REL-10`: a fresh install refused every scheduled export, alert, report and embed
until `CATALOG_TENANT_ID` was set by hand; install and update now set it to the group tenant
when it is empty, and never change a value an operator set.

## 4. Mutation check (in the handoff)

Remove the helper's call from `write_env` and run T1's "new `.env`" test: it must fail.
Restore, rerun, quote both.

## 5. Out of scope

- Whether a production install should seed the four demo tenants at all (the seed names
  sample companies). Report it as a finding in the handoff; the planner raises it with the
  owner.
- Per-tenant catalogs.

## 6. Verification before handoff

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
git status
```

## 7. Handoff (developer appends below)

## 8. Review (planner appends below)
