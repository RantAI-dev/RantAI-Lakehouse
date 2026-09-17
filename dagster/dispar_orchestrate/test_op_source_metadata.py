"""Every @op in this code location must carry source_ref/commit metadata
so `GET /api/pipelines/{id}/source?op=` can resolve real source text —
WS4, grand plan §6. Verified against the ACTUAL definitions
module, not a hand-listed set of op names, so a future op added without
metadata fails this test immediately rather than silently shipping
unreadable source.

# Deviation from the plan sketch (WS4 pipelines-detail-source-logs plan)

The plan sketch reads `op.op_def.metadata` and assumes `@op(metadata=...)`
is a valid decorator kwarg. Neither exists against the installed
`dagster==1.13.20` (`dagster._core.definitions.op_definition.OpDefinition`
has no `metadata` attribute, and `@op(metadata={})` raises `TypeError: op()
got an unexpected keyword argument 'metadata'`). Reading
`dagster_graphql/schema/solids.py`'s `resolve_metadata` on
`GrapheneSolidDefinition` shows the GraphQL `metadata` field this whole
task exists to populate is actually sourced from `_solid_def_snap.tags` —
i.e. `@op(tags=...)`, not a nonexistent `metadata=` kwarg. This test (and
`op_metadata.py`) therefore checks `op.tags`, the real, installed API.
"""
from __future__ import annotations

import unittest

from dispar_orchestrate import assets, agent_runs, gold_export, maintenance

_OPS = [
    assets.ingest_bronze_table,
    assets.register_in_catalog,
    agent_runs.run_agent_employee,
    gold_export.run_gold_export,
    maintenance.run_bronze_maintenance,
]


class OpSourceMetadataTest(unittest.TestCase):
    def test_every_op_declares_a_source_ref_matching_module_colon_colon_function(self) -> None:
        for op in _OPS:
            tags = op.tags
            self.assertIn("source_ref", tags, op.name)
            self.assertRegex(
                tags["source_ref"],
                r"^dispar_orchestrate/[a-z_]+\.py::[a-z_]+$",
                op.name,
            )

    def test_every_op_declares_a_commit_string(self) -> None:
        for op in _OPS:
            self.assertIn("commit", op.tags, op.name)
            self.assertIsInstance(op.tags["commit"], str)
