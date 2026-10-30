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

## 7. Review (planner appends below)
