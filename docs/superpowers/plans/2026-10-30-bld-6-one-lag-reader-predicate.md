# BLD-6: the two "who reads this lag target" queries share one predicate

Backlog: `BLD-6` (Task, Build, P3), from PR #234 review round 1, SHOULD-FIX 1. Base: `main`
at `4ce58470`. Every `file:line` below was read there.

Planner: Claude Opus. Developer: a different agent (`AGENTS.md`). The developer does not change
a decision below. If the code disagrees with the plan, stop and report it.

## 0. What is wrong

`rust/crates/lakehouse-store/src/pipelines.rs` has two queries that ask the same question:
- `count_lag_pipelines_reading` (`:2210`), one candidate at a time;
- `lag_pipelines_without_readers` (`:2255`), a batch.

The question: does a pipeline with a target lag read this candidate's target, either as its
`source` or through `depends_on`?

The predicate is copied by hand: `$1/$2/$3` in one query and `c.id/c.target/c.tenant_id` in
the other. The batched doc (`:2240-2247`) says the batched answer "cannot drift" from the
per-row one, but nothing enforces that. A change to one copy would silently break the other.
`AGENTS.md` rule 1: a comment must be true.

Callers:
- `pipeline_lag.rs:843` uses the per-row count, for the page's "no pipeline with a target lag
  reads this table" note;
- `pipeline_lag.rs:1510` uses the batch (`downstream_without_readers`, doc at `:1479-1497`).

## 1. Decisions (made; do not change)

- **D1. One predicate, written once.**
  - A private `fn lag_reader_predicate(candidate_id: &str, candidate_target: &str,
    candidate_tenant: &str) -> String` in `pipelines.rs` returns the `NOT EXISTS` / `count`
    body's predicate, with the three candidate expressions passed in.
  - Both queries build their SQL from it with `format!`. That is allowed only because every
    argument is a compile-time constant: the per-row query passes `"$1"`, `"$2"`, `"$3"`; the
    batch passes `"c.id"`, `"c.target"`, `"c.tenant_id"`. Values stay bound.
  - Run both through `sqlx::AssertSqlSafe`, as `pipelines.rs:703` already does. Each site gets
    the comment `// safe: only constant expressions are formatted in; values are bound
    (SEC-41).`, the house form (see `lakehouse-store/src/public_links.rs:186`).
- **D2. A test proves they agree.** One store test runs both functions on the same fixture and
  checks that, for every candidate, `count > 0` exactly when the candidate is NOT in the
  batch's "without readers" list. The fixture covers:
  - a reader through `source`;
  - a reader through `depends_on`;
  - a pipeline reading itself (excluded);
  - a reader in another tenant (does not count);
  - a NULL-tenant candidate (no readers);
  - a candidate nobody reads.
- **D3. The comment tells the truth.** The batched doc says the two share
  `lag_reader_predicate` and that the agreement test enforces it. Cite `BLD-6` at both sites and
  at the test.
- **D4. No behaviour change.** The SQL meaning, the tenant rules (a NULL tenant matches nothing,
  and the per-row function's early `return Ok(0)` stays) and both signatures are unchanged.
  `pipeline_lag.rs` does not change.

## 2. Anchors (at `4ce58470`)

| What | Where |
| --- | --- |
| Per-row count | `lakehouse-store/src/pipelines.rs:2210` (`count_lag_pipelines_reading`), SQL `:2219-2226` |
| Batch | `:2255` (`lag_pipelines_without_readers`), SQL `:2264-2275`, doc `:2236-2251` |
| The `AssertSqlSafe` pattern | `pipelines.rs:703`, `:783`, `:1025`; the comment form at `public_links.rs:186` |
| Existing tests | `lakehouse-store/tests/pipelines.rs:2468-2535` (`count_lag_pipelines_reading_counts_other_lag_pipelines`), `create_tenant` / `tenant_input` helpers in that file |
| Callers (unchanged) | `lakehouse-api/src/pipeline_lag.rs:843`, `:1510` |

## 3. Tasks (one commit each, in order)

**T1. The agreement test first** (`lakehouse-store/tests/pipelines.rs`).
- Write D2's test against today's two copies. It must pass now, because they agree today.
- Name it as a sentence, for example `the_batched_lag_reader_check_agrees_with_the_per_row_count`.
- *Accept:* `cargo test -p lakehouse-store --test pipelines lag` passes.

**T2. The shared predicate** (`lakehouse-store/src/pipelines.rs`).
- D1 and D3, with D4's guarantees.
- *Accept:*
  - `cargo fmt --check`, `cargo clippy -p lakehouse-store --all-targets -- -D warnings`, and
    `cargo test -p lakehouse-store --test pipelines lag` pass;
  - then `cargo test -p lakehouse-api pipeline_lag` passes (the callers);
  - mutation check: change one word of the shared predicate in a scratch edit (for example drop
    `p.id != …`), run the test, quote that it FAILS, then revert.

## 4. Verification before handoff

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
git status
```

## 5. Out of scope

Any change to what counts as a reader, the lag rules themselves, or `pipeline_lag.rs`.

## 6. Handoff (developer appends below)

### Handoff (written by the planner from the developer's logs)

Both developer sessions ended early, and neither wrote this entry. The first was stopped when the
planner's turn was interrupted. The second failed with a provider error (MiniMax `2013 invalid
params`) after its full test run had finished. The planner copied the lines below from the
developer's own logs on the planner machine, at `cda966e1`. It did not re-run them here.

**Commits**
- `c1b386f1` test(store): the batched lag reader check agrees with the per-row count (BLD-6 T1)
- `cda966e1` refactor(store): one lag reader predicate for both queries (BLD-6 T2)

**Commands and counts**, all on `cda966e1`, through `bin/cargo-locked`, with `-j 2`:

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean (empty output) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `Finished dev profile ... in 7m 31s` |
| `cargo test -p lakehouse-store --test pipelines lag` | `ok. 15 passed; 0 failed` |
| `cargo test -p lakehouse-api pipeline_lag` | `ok. 42 passed; 0 failed` |
| `cargo test --workspace` | 159 binaries, 8,550 passed, 0 failed, 51 ignored |

**Mutation check (plan T2).** With the shared predicate broken in a scratch edit,
`the_batched_lag_reader_check_agrees_with_the_per_row_count ... FAILED` (`test result: FAILED. 0
passed; 1 failed`, in `/tmp/bld6-mutation.log` and `/tmp/bld6-mutation3.log`). The tree was clean
afterwards (`git status` showed no tracked change at `cda966e1`).

**Not verified:** `git status` after the full run was not recorded by the developer. The planner
checked the worktree afterwards and found no tracked change.

## 7. Review (planner appends below)

### Review round 1 (planner, 2026-10-30, at `cda966e1`)

- **The SQL means the same.** I checked each placeholder against the old text.
  - Per-row: `p.tenant_id = $3`, `p.id != $1`, `p.source = $2`, and `u.tenant_id = $3 AND
    u.target = $2` in the `depends_on` branch.
  - Batched: the same, with `c.tenant_id`, `c.id`, `c.target`.
  - The NULL-tenant early `return Ok(0)` is kept (D4). Both signatures are unchanged, and
    `pipeline_lag.rs` is untouched.
- **`format!`** takes only the six constant expressions, values stay bound, and both sites carry
  the `SEC-41` comment (D1).
- **The test covers every case D2 lists**, plus one: a reader without a target lag does not
  count. It writes the self-reading row with a bound `UPDATE`, because the store refuses to
  create one. That is right, and the test says why.
- **Nit, not a finding:** the batched doc says "D3: the store test ... keeps the two answers in
  step". In this plan the test is D2 and the comment is D3. Fix it when the file is next touched.

**Open: no BLOCKER, no SHOULD-FIX.**
