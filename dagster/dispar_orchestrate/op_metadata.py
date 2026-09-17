"""Shared `@op` source-provenance metadata (WS4, grand plan §6): every op
in this code location declares where its own source lives and which
commit it was built from, so `GET /api/pipelines/{id}/source?op=` can
serve real text and detect a stale build (`GIT_SHA` mismatch -> 409).

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


def source_metadata(source_ref: str, sql: str | None = None) -> dict[str, str]:
    """Build the `tags=` dict every `@op` decorator passes.

    `source_ref` must be `"dispar_orchestrate/<file>.py::<function>"` —
    the exact shape `pipeline_source.rs` (Phase C) validates against its
    allowlist. `sql` is the literal statement template for an op that
    executes one (e.g. `run_bronze_maintenance`'s `REMOVE ORPHAN FILES`
    call) — omitted (not empty string) for ops with no single SQL
    statement to show.
    """
    metadata = {
        "source_ref": source_ref,
        "commit": os.environ.get("GIT_SHA", "unknown"),
    }
    if sql is not None:
        metadata["sql"] = sql
    return metadata
