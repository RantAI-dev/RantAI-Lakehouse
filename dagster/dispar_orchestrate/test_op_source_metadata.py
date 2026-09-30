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

    def test_every_op_carries_the_default_retry_policy(self) -> None:
        """PART D of `parts/1b-dagster-one-op-per-unit-and-retries.md`:
        every op in the code location must carry `DEFAULT_RETRY_POLICY` so
        a transient blip self-heals via Dagster's own retry mechanism
        instead of failing the whole job. The two self-healing attempts
        are bounded (`max_retries=2`, `delay=30`,
        `backoff=EXPONENTIAL`, `jitter=PLUS_MINUS`) -- a config/auth/SSRF
        raise never benefits from retries because it is wrapped in
        `Failure(allow_retries=False)` at the call site before it can
        reach the policy."""
        expected = op_metadata.DEFAULT_RETRY_POLICY
        for op in _every_op():
            with self.subTest(op=op.name):
                self.assertIsNotNone(op.retry_policy, f"{op.name} declares no retry_policy")
                self.assertEqual(
                    op.retry_policy.max_retries,
                    expected.max_retries,
                    f"{op.name} has max_retries={op.retry_policy.max_retries}, expected {expected.max_retries}",
                )
                self.assertEqual(
                    op.retry_policy.delay,
                    expected.delay,
                    f"{op.name} has delay={op.retry_policy.delay}, expected {expected.delay}",
                )
                self.assertEqual(
                    op.retry_policy.backoff,
                    expected.backoff,
                    f"{op.name} has backoff={op.retry_policy.backoff}, expected {expected.backoff}",
                )
                self.assertEqual(
                    op.retry_policy.jitter,
                    expected.jitter,
                    f"{op.name} has jitter={op.retry_policy.jitter}, expected {expected.jitter}",
                )

    def test_default_retry_policy_retries_a_transient_network_error(self) -> None:
        """`DEFAULT_RETRY_POLICY.delay=30` would slow the suite by a full
        minute per attempt, so this test runs the policy in-process with a
        local `max_retries=1, delay=0` policy and asserts the SAME
        RetryPolicy shape Dagster uses actually retries a
        `requests.ConnectionError` (a transient-shaped failure the
        production policy exists to absorb) -- proves the policy object
        is wired correctly, independent of the bounded-by-environment
        default."""
        from dagster import RetryPolicy, job, op
        import requests as _requests

        attempt_count = {"n": 0}

        @op(retry_policy=RetryPolicy(max_retries=1, delay=0))
        def network_op() -> None:
            attempt_count["n"] += 1
            raise _requests.ConnectionError("lakehouse-api unreachable")

        @job
        def network_job() -> None:
            network_op()

        result = network_job.execute_in_process(raise_on_error=False)
        self.assertFalse(result.success)
        # The first attempt plus the one retry the policy allows must both
        # have actually run; without the policy, the op would have been
        # called once and stopped.
        self.assertEqual(attempt_count["n"], 2)
        retry_events = [
            e for e in result.all_events if e.event_type_value == "STEP_UP_FOR_RETRY"
        ]
        self.assertEqual(len(retry_events), 1, "the retry policy must fire exactly once")

    def test_a_failure_with_allow_retries_false_is_not_retried(self) -> None:
        """The other half of PART D's contract: a `Failure(allow_retries=False)`
        is the explicit "this will not get better with a retry" signal,
        and the policy must respect it even though `max_retries=1` would
        otherwise let it fire. Same `max_retries=1, delay=0` test-local
        policy as the transient-error test, to keep the suite fast."""
        from dagster import RetryPolicy, job, op, Failure

        attempt_count = {"n": 0}

        @op(retry_policy=RetryPolicy(max_retries=1, delay=0))
        def config_op() -> None:
            attempt_count["n"] += 1
            raise Failure(description="bad config", allow_retries=False)

        @job
        def config_job() -> None:
            config_op()

        result = config_job.execute_in_process(raise_on_error=False)
        self.assertFalse(result.success)
        # allow_retries=False short-circuits the policy: exactly one attempt.
        self.assertEqual(attempt_count["n"], 1)
        retry_events = [
            e for e in result.all_events if e.event_type_value == "STEP_UP_FOR_RETRY"
        ]
        self.assertEqual(retry_events, [], "Failure(allow_retries=False) must never trigger a retry")
