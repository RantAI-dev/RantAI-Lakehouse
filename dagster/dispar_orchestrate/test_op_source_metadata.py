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

from dispar_orchestrate import authored_factory, op_metadata
from dispar_orchestrate.definitions import defs


def _every_op():
    """Every op of every job the code location registers, read from `defs`
    itself. This list used to be written by hand, and four ops
    (`run_alerts_op`, `run_capacity_snapshot`, `run_replication_slot_check`,
    `run_ingest`) shipped without metadata, so the console could show
    neither their source nor what they read and write."""
    seen = {}
    for job in defs.resolve_all_job_defs():
        for node in job.graph.node_dict.values():
            seen.setdefault(node.definition.name, node.definition)
    return list(seen.values())


class OpSourceMetadataTest(unittest.TestCase):
    def test_the_walk_finds_every_registered_op(self) -> None:
        names = {op.name for op in _every_op()}
        self.assertGreaterEqual(len(names), 9, names)
        self.assertIn("run_alerts_op", names)
        self.assertIn("run_ingest", names)

    def test_every_op_declares_a_source_ref_matching_module_colon_colon_function(self) -> None:
        for op in _every_op():
            tags = op.tags
            self.assertIn("source_ref", tags, op.name)
            self.assertRegex(
                tags["source_ref"],
                r"^dispar_orchestrate/[a-z_]+\.py::[a-z_]+$",
                op.name,
            )

    def test_every_op_declares_a_commit_string(self) -> None:
        for op in _every_op():
            self.assertIn("commit", op.tags, op.name)
            self.assertIsInstance(op.tags["commit"], str)

    def test_every_op_declares_what_it_writes(self) -> None:
        for op in _every_op():
            self.assertTrue(op.tags.get("writes", "").strip(), f"{op.name} declares no writes")

    def test_reads_and_writes_travel_as_newline_joined_phrases(self) -> None:
        tags = op_metadata.source_metadata(
            "dispar_orchestrate/x.py::y", reads=["a", "b"], writes=["c"]
        )
        self.assertEqual(tags["reads"], "a\nb")
        self.assertEqual(tags["writes"], "c")
        self.assertNotIn("reads", op_metadata.source_metadata("dispar_orchestrate/x.py::y"))

    def test_a_phrase_with_a_newline_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            op_metadata.source_metadata("dispar_orchestrate/x.py::y", reads=["a\nb"])

    def test_an_authored_op_declares_its_own_source_and_target_tables(self) -> None:
        op_def = authored_factory._op_for_pipeline(
            {
                "id": "pl-demo-1",
                "definition": {
                    "sourceZone": "silver",
                    "sourceTable": "events",
                    "targetZone": "gold",
                    "targetTable": "events_clean",
                    "transforms": [],
                },
            }
        )
        self.assertEqual(op_def.tags["reads"], "ClickHouse silver.events")
        self.assertEqual(op_def.tags["writes"], "ClickHouse gold.events_clean")
