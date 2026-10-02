"""Shared `@op` source-provenance metadata (WS4, grand plan §6): every op
in this code location declares where its own source lives and which
commit it was built from, so `GET /api/pipelines/{id}/source?op=` can
serve real text and detect a stale build (`GIT_SHA` mismatch -> 409).

Also exposes `DEFAULT_RETRY_POLICY`: the single `dagster.RetryPolicy`
every `@op` in this code location is wired with. Transient blips — a
network hiccup, a busy ClickHouse — deserve two self-healing attempts
with exponential backoff and jitter (so synchronized retries do not
arrive as a single thundering herd); config, auth, SSRF, and
column-type failures are exempted at the call site via
`dagster.Failure(allow_retries=False)`, so this policy never masks a
real misconfiguration as a silent retry.

# Deviation from the plan sketch (see `test_op_source_metadata.py`)

The plan's sketch attaches this dict via `@op(metadata=...)`. Against the
installed `dagster==1.13.20`, `@op` has no `metadata` kwarg at all
(`TypeError: op() got an unexpected keyword argument 'metadata'`) and
`OpDefinition` has no `.metadata` attribute. Reading
`dagster_graphql/schema/solids.py`'s `GrapheneSolidDefinition.resolve_metadata`
shows the GraphQL `metadata` field this whole task exists to populate is
sourced from the op's `tags`, not a `metadata=` kwarg — so this helper's
dict is passed as `@op(tags=source_metadata(...))` at each call site.
"""
from __future__ import annotations

import os
from collections.abc import Sequence

from dagster import Backoff, Jitter, RetryPolicy

DEFAULT_RETRY_POLICY = RetryPolicy(
    max_retries=2,
    delay=30,
    backoff=Backoff.EXPONENTIAL,
    jitter=Jitter.PLUS_MINUS,
)


def source_metadata(
    source_ref: str,
    sql: str | None = None,
    reads: Sequence[str] = (),
    writes: Sequence[str] = (),
) -> dict[str, str]:
    """Build the `tags=` dict every `@op` decorator passes.

    `source_ref` must be `"dispar_orchestrate/<file>.py::<function>"` —
    the exact shape `pipeline_source.rs` (Phase C) validates against its
    allowlist. `sql` is the literal statement template for an op that
    executes one (e.g. `maintain_bronze_table`'s `REMOVE ORPHAN FILES`
    call) — omitted (not empty string) for ops with no single SQL
    statement to show.

    `reads`/`writes` are the data the op's own code reads and writes, one
    short phrase each ("Iceberg bronze.* (every table)", "POST
    /api/alerts/run"). They are what the console's pipeline flowchart
    draws on either side of the op, labelled as declared by the op, not
    observed from a run. Keep them next to the code they describe, so a
    change to what an op touches changes its declaration in the same diff.
    Dagster tags are strings, so each list travels newline-joined; a phrase
    must therefore not contain a newline.
    """
    metadata = {
        "source_ref": source_ref,
        "commit": os.environ.get("GIT_SHA", "unknown"),
    }
    if sql is not None:
        metadata["sql"] = sql
    for key, phrases in (("reads", reads), ("writes", writes)):
        if any("\n" in phrase for phrase in phrases):
            raise ValueError(f"a {key} phrase must not contain a newline")
        if phrases:
            metadata[key] = "\n".join(phrases)
    return metadata
