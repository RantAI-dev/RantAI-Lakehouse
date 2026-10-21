"""Unit tests for `authored_factory.py`'s job factory (WS4 items E1/E3,
grand plan §6). Same non-negotiable property `test_agent_runs.py`
establishes for `agent_runs.py`: an unreachable/unauthenticated
`lakehouse-api` must leave the whole Dagster code location loadable, with
zero authored jobs and a loud warning -- never an import-time exception.

No real network: `requests.get` is monkeypatched with canned responses/
exceptions, the same style `test_agent_runs.py` uses. No real ClickHouse:
`authored_factory._ch_exec`/`_ch_query_json` are monkeypatched, the same
style `test_maintenance.py` uses for `_ch_query`.

The final class, `TestDependencySensor`, exercises R3 plan 2a
(`build_authored_dependency_sensor`): a `run_status_sensor` per pipeline
with non-empty `depends_on` that applies ALL semantics ("has every
upstream had a SUCCESS run that finished AFTER this downstream's
most-recent start?") and yields either a `RunRequest(run_key=<upstream
run id>)` or a `SkipReason` naming the upstream that is still behind.
`context.instance` is replaced by a stub that records the `RunsFilter`s
the sensor builds and returns a fixed `DagsterRun` per call; no real
Dagster instance, no real run storage.

Run with: `~/.cache/rantai-dagster-venv/bin/python -m pytest
dagster/dispar_orchestrate/test_authored_factory.py -v`
"""

from __future__ import annotations

import contextlib
import os
import unittest
from contextlib import contextmanager
from typing import Any, Iterator, Sequence
from unittest import mock

import requests
from dagster import (
    AssetKey,
    AssetMaterialization,
    Backoff,
    DagsterRunStatus,
    DefaultScheduleStatus,
    Jitter,
    JobSelector,
    RetryPolicy,
    RunRequest,
    RunsFilter,
    build_op_context,
    job,
    op,
)

from dispar_orchestrate import authored_factory, authored_transforms
from dispar_orchestrate.authored_factory import (
    DAGSTER_LOCATION,
    DAGSTER_REPO,
    _upstream_job_selector,
    build_authored_dependency_sensor,
)
from dispar_orchestrate.bronze_catalog import ClickHouseTarget


def _cfg(run_token: str = "a-real-token") -> authored_factory.AuthoredPipelineConfig:
    return authored_factory.AuthoredPipelineConfig(
        api_url="http://lakehouse-api.invalid:8080", run_token=run_token,
    )


@contextlib.contextmanager
def _enforce_passthrough():
    """Make the op's BLD-1 / D4 enforcement call answer with the SQL it was
    given, unchanged.

    Every test below that exercises the op's happy path needs this: the op
    now POSTs its `SELECT` to the API and runs whatever comes back, so
    without it each of them would make a real request to
    `http://lakehouse-api.invalid:8080` (a host that does not resolve) and
    fail on a connection error rather than on the thing it is testing.
    Returning the input verbatim is the neutral stand-in — it leaves every
    assertion about the SQL the op builds and writes exactly as valid as
    before D4. The tests that care about enforcement itself
    (`EnforceSqlTest`) mock the transport instead of this seam, so they
    exercise the real `_enforced_sql`.
    """
    with mock.patch.object(
        authored_factory, "_enforced_sql", side_effect=lambda cfg, pid, sql: sql,
    ) as enforced:
        yield enforced


class HeadersTest(unittest.TestCase):
    def test_empty_token_sends_no_headers(self) -> None:
        self.assertEqual(authored_factory._headers(_cfg(run_token="")), {})

    def test_real_token_sends_both_headers(self) -> None:
        headers = authored_factory._headers(_cfg(run_token="tok"))
        self.assertEqual(headers["Authorization"], "Bearer tok")
        self.assertEqual(headers["x-run-token"], "tok")


class FetchAuthoredPipelinesTest(unittest.TestCase):
    def test_empty_token_returns_empty_list_with_no_http_call(self) -> None:
        with mock.patch.object(authored_factory.requests, "get") as mocked_get:
            result = authored_factory._fetch_authored_pipelines(_cfg(run_token=""))
        self.assertEqual(result, [])
        mocked_get.assert_not_called()

    def test_unreachable_api_returns_empty_list_and_does_not_raise(self) -> None:
        with mock.patch.object(
            authored_factory.requests, "get", side_effect=requests.ConnectionError("refused"),
        ):
            result = authored_factory._fetch_authored_pipelines(_cfg())
        self.assertEqual(result, [])

    def test_non_200_returns_empty_list(self) -> None:
        """Covers a 403 the `authored-pipeline-scheduler`
        identity (`pipeline:write` only) gets from `pipeline:read`-gated
        `GET /api/pipelines` -- see `authored_factory.py`'s module doc."""
        resp = mock.Mock(status_code=403)
        resp.raise_for_status.side_effect = requests.HTTPError("403")
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            self.assertEqual(authored_factory._fetch_authored_pipelines(_cfg()), [])

    def test_non_json_body_returns_empty_list(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.raise_for_status.return_value = None
        resp.json.side_effect = ValueError("not json")
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            self.assertEqual(authored_factory._fetch_authored_pipelines(_cfg()), [])

    def test_non_list_pipelines_field_returns_empty_list(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.raise_for_status.return_value = None
        resp.json.return_value = {"pipelines": "not-a-list"}
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            self.assertEqual(authored_factory._fetch_authored_pipelines(_cfg()), [])

    def test_a_pipeline_missing_the_ready_status_is_excluded(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [
            {"id": "pl-a", "status": "draft", "definition": {}},
            {"id": "pl-b", "status": "ready", "definition": {
                "sourceZone": "silver", "sourceTable": "orders",
                "targetZone": "serving", "targetTable": "orders_clean",
                "transforms": ["select(id,name)"], "fbicEnabled": False,
                "incrementalColumn": None, "connectorId": None,
            }},
        ]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            result = authored_factory._fetch_authored_pipelines(_cfg())
        self.assertEqual([p["id"] for p in result], ["pl-b"])

    def test_a_ready_pipeline_with_no_definition_is_excluded(self) -> None:
        """A Dagster-job row unioned into the same `/api/pipelines` list
        (`dagster_pipeline_row`) has no `definition` at all -- must never
        be mistaken for a ready authored pipeline."""
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [
            {"id": "bronze_ingest_job", "status": "unknown", "kind": "batch"},
        ]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            result = authored_factory._fetch_authored_pipelines(_cfg())
        self.assertEqual(result, [])


def _ready_pipeline(**overrides: object) -> dict[str, object]:
    definition = {
        "sourceZone": "silver", "sourceTable": "orders",
        "targetZone": "serving", "targetTable": "orders_clean",
        "transforms": ["select(id,name)"], "fbicEnabled": False,
        "incrementalColumn": None, "connectorId": None,
        # Plan 1c (R2, day-1): the per-pipeline retry cap. Absent here
        # so legacy callers of `_ready_pipeline()` continue to assert
        # the migration's `DEFAULT 2` shape (matching the store's own
        # `tests/pipelines.rs::create_pipeline_defaults_max_retries_to_two`).
        "maxRetries": 2,
    }
    definition.update(overrides)
    return {"id": "pl-dedupe-select-abc123", "status": "ready", "definition": definition}


class BuildAuthoredJobsTest(unittest.TestCase):
    def test_build_authored_jobs_produces_one_job_per_ready_pipeline(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [_ready_pipeline()]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            jobs = authored_factory.build_authored_jobs(_cfg())
        self.assertEqual(len(jobs), 1)
        self.assertEqual(jobs[0].name, "authored__pl_dedupe_select_abc123")

    def test_dagster_safe_name_replaces_every_non_alnum_underscore_char(self) -> None:
        self.assertEqual(
            authored_factory._dagster_safe_name("pl-a.b c-1"), "pl_a_b_c_1",
        )


class BuildSelectSqlTest(unittest.TestCase):
    def test_select_rename_cast_filter_dedupe_compose_into_one_statement(self) -> None:
        transforms = [
            authored_transforms.parse_transform("select(id,amount)"),
            authored_transforms.parse_transform("rename(amount,amount_usd)"),
            authored_transforms.parse_transform("cast(amount,Float64)"),
            authored_transforms.parse_transform("filter(status = 'active')"),
            authored_transforms.parse_transform("dedupe(id)"),
        ]
        sql = authored_factory._build_select_sql("silver.`orders`", transforms)
        self.assertEqual(
            sql,
            "SELECT id, CAST(amount AS Float64) AS amount_usd FROM silver.`orders` "
            "WHERE status = 'active' ORDER BY id LIMIT 1 BY id",
        )

    def test_no_transforms_selects_star_with_no_where_or_order(self) -> None:
        sql = authored_factory._build_select_sql("silver.`orders`", [])
        self.assertEqual(sql, "SELECT * FROM silver.`orders`")


class BronzeSourceTest(unittest.TestCase):
    """BLD-1 / D5 (was BLD-3 / D1 + D2): a `bronze` source is Iceberg in
    Lakekeeper, not a ClickHouse database, so it is read through a
    `DataLakeCatalog` database as ``<db>.`bronze.<table>` ``, and every
    statement that reads that database carries
    `SETTINGS allow_database_iceberg = 1`.

    WHICH database changed in D5, and these tests pin the new answer: it is
    the API's own `ICEBERG_QUERY_DB`, because the API's policy rewriter
    only recognises a governed table under that name, and a pipeline that
    read Bronze through a private database was therefore never rewritten
    at all. This module no longer creates a catalog database either — the
    `clickhouse-iceberg-init` compose service owns that one — so the
    "ensured before the first read" assertion BLD-3 made is replaced by
    "this module creates none".

    No network and no ClickHouse: `_ch_exec`/`_ch_query_json` are
    monkeypatched exactly as `OpForPipelineTest` does, and the enforcement
    call is stubbed by `_enforce_passthrough`. Calls are recorded in order.
    """

    def _run_op(self, **definition_overrides: object) -> tuple[dict[str, Any], list[tuple[str, str]]]:
        pipeline = _ready_pipeline(**definition_overrides)
        run_fn = authored_factory._op_for_pipeline(pipeline)
        calls: list[tuple[str, str]] = []
        counts = iter([[{"n": "0"}], [{"n": "3"}]])
        with mock.patch.object(
            authored_factory,
            "_ch_exec",
            side_effect=lambda target, statement: calls.append(("sql", statement)),
        ), mock.patch.object(
            authored_factory,
            "_ch_query_json",
            side_effect=lambda target, statement: next(counts),
        ), mock.patch.object(
            authored_factory.ClickHouseTarget,
            "from_env",
            return_value=ClickHouseTarget("http://ch", "default", ""),
        ), mock.patch.dict(
            os.environ, {"ICEBERG_QUERY_DB": "icecat_api"}, clear=False,
        ), _enforce_passthrough():
            result = run_fn(build_op_context())
        return result, calls

    @staticmethod
    def _statements(calls: list[tuple[str, str]]) -> list[str]:
        """The SQL statements the op sent, in order."""
        return [statement for kind, statement in calls if kind == "sql"]

    @staticmethod
    def _bronze_pipeline(**overrides: object) -> dict[str, object]:
        """`_ready_pipeline` DEFINITION overrides for a Bronze-to-Silver
        pipeline, the shape `ops/g7/g7_pipeline_test.py` authors."""
        return {
            "sourceZone": "bronze",
            "sourceTable": "g3a_orders",
            "transforms": ["dedupe(id)", "select(id,name)"],
            "targetZone": "silver",
            "targetTable": "bronze_out",
            **overrides,
        }

    def test_a_bronze_source_is_read_through_the_api_iceberg_query_database(self) -> None:
        """D5: the source is ``icecat_api.`bronze.g3a_orders` `` -- the API's
        OWN `ICEBERG_QUERY_DB` plus the catalog's own flat `bronze.<table>`
        name -- never the ClickHouse database `bronze` that does not exist
        on this stack, and never a database private to this module (which
        is the change D5 makes, and the reason: the API's policy rewriter
        resolves a governed table by its name under this database, so
        reading through any other one left the query unenforced)."""
        _, calls = self._run_op(**self._bronze_pipeline())
        selects = [s for s in self._statements(calls) if "icecat_api" in s]
        self.assertTrue(selects, f"no statement read the catalog database: {calls}")
        for statement in selects:
            self.assertIn(
                "FROM icecat_api.`bronze.g3a_orders`",
                statement,
                f"bronze source not read through the catalog database: {statement}",
            )
        # The Bronze source is never spelled as a ClickHouse database.
        self.assertFalse(
            [s for s in self._statements(calls) if "`g3a_orders`" in s and "icecat_api" not in s]
        )

    def test_this_module_creates_no_catalog_database_of_its_own(self) -> None:
        """D5: `clickhouse-iceberg-init` creates the API's `ICEBERG_QUERY_DB`
        on every `up`, so a `CREATE DATABASE ... DataLakeCatalog` here would
        be a second copy of a statement that already exists -- and the one
        that exists is the one the rewriter must see."""
        _, calls = self._run_op(**self._bronze_pipeline())
        self.assertFalse(
            [s for s in self._statements(calls) if "CREATE DATABASE" in s and "icecat_api" in s],
            f"this module created the API's catalog database itself: {calls}",
        )
        self.assertFalse(
            hasattr(authored_factory, "CATALOG_DB"),
            "the private catalog-database name is gone in D5; the source is "
            "named from the API's ICEBERG_QUERY_DB instead",
        )

    def test_every_statement_reading_the_catalog_database_carries_allow_database_iceberg(self) -> None:
        """D2: ClickHouse refuses an Iceberg read without the setting, so it
        rides on the `CREATE TABLE ... AS SELECT ... LIMIT 0`, on the
        staging table's own `CREATE`, and on the staging `INSERT ... SELECT`
        -- the three statements that read the source. It rides LAST in each
        statement (ClickHouse parses `SETTINGS` as the SELECT's final
        clause), which is why `_reading` appends it rather than building it
        into `_build_select_sql`'s `FROM` clause."""
        _, calls = self._run_op(**self._bronze_pipeline())
        reads = [s for s in self._statements(calls) if "icecat_api" in s]
        self.assertEqual(len(reads), 3, f"expected CREATE, staging CREATE and INSERT: {reads}")
        for statement in reads:
            self.assertTrue(
                statement.endswith("SETTINGS allow_database_iceberg = 1"),
                f"statement does not carry the Iceberg read setting: {statement}",
            )
        kinds = [
            "staging CREATE" if statement.startswith("CREATE TABLE silver.`__staging") else
            "INSERT" if statement.startswith("INSERT INTO") else "target CREATE"
            for statement in reads
        ]
        self.assertEqual(
            sorted(kinds),
            ["INSERT", "staging CREATE", "target CREATE"],
        )

    def test_a_silver_source_is_read_exactly_as_before(self) -> None:
        """D3: a `silver` (or `gold`, or any ClickHouse zone) source is still
        ``silver.`orders` ``, no Iceberg read setting appears, and no
        statement grows anything it did not have before."""
        result, calls = self._run_op()
        self.assertEqual(result["rows"], 3)
        statements = self._statements(calls)
        self.assertFalse([s for s in statements if "icecat_api" in s])
        self.assertFalse([s for s in statements if "allow_database_iceberg" in s])
        self.assertTrue(
            any(
                "ENGINE = MergeTree ORDER BY tuple() AS SELECT id, name FROM silver.`orders` LIMIT 0" in s
                for s in statements
            ),
            statements,
        )
        self.assertTrue(
            any("INSERT INTO serving.`__staging_orders_clean` SELECT id, name FROM silver.`orders`" == s for s in statements),
            statements,
        )

    def test_resolve_source_marks_only_bronze_as_an_iceberg_read(self) -> None:
        """`_PipelineSource.reads_iceberg` is what makes `_reading` attach the
        setting, so it must be true for `bronze` alone. The catalog database
        is a PARAMETER (D5: the caller's `ICEBERG_QUERY_DB`, not a constant
        of this module's), so a different value is a different source name
        rather than an ignored argument."""
        bronze = authored_factory._resolve_source("bronze", "orders", "icecat_api")
        self.assertEqual(bronze.clickhouse_name, "icecat_api.`bronze.orders`")
        self.assertTrue(bronze.reads_iceberg)
        other = authored_factory._resolve_source("bronze", "orders", "some_other_db")
        self.assertEqual(other.clickhouse_name, "some_other_db.`bronze.orders`")
        for zone in ("silver", "gold", "serving"):
            source = authored_factory._resolve_source(zone, "orders", "icecat_api")
            self.assertEqual(source.clickhouse_name, f"{zone}.`orders`")
            self.assertFalse(source.reads_iceberg)

    def test_a_bronze_source_with_a_silver_target_is_still_executed(self) -> None:
        """The refusal is on the TARGET only: `bronze` -> `silver` is the
        path D1/D2 fixed, and it must keep running (the whole point of
        this change)."""
        _, calls = self._run_op(
            **{"sourceZone": "bronze", "sourceTable": "g3a_orders", "targetZone": "silver"}
        )
        self.assertTrue([s for s in self._statements(calls) if "silver.`orders_clean`" in s])


class EnforceSqlTest(unittest.TestCase):
    """BLD-1 / D4: the SQL the op RUNS is the API's rewrite of the SQL it
    BUILT, never the SQL it built.

    These mock the HTTP transport (`requests.post`), not `_enforced_sql`, so
    the real request shape -- URL, headers, body -- and the real refusal
    handling are exercised. Everything else (ClickHouse) is monkeypatched
    the way `OpForPipelineTest` does; no test here touches the network.
    """

    #: The rewrite a governed column gets back from the API. Deliberately
    #: NOT the SQL we sent: if the op ran its own SQL, every assertion
    #: below would fail.
    ENFORCED = "SELECT id, '***' AS name FROM silver.`orders`"

    def _post_returning(self, status: int, body: Any) -> Any:
        """A canned response whose `.json()` returns `body`."""
        resp = mock.Mock(status_code=status)
        resp.raise_for_status.return_value = None
        resp.json.return_value = body
        return resp

    def _post_raising(self, exc: Exception) -> Any:
        resp = mock.Mock()
        resp.raise_for_status.side_effect = exc
        return resp

    def _run_op_with(self, post: Any, **definition_overrides: object) -> Any:
        pipeline = _ready_pipeline(**definition_overrides)
        run_fn = authored_factory._op_for_pipeline(pipeline)
        counts = iter([[{"n": "0"}], [{"n": "3"}]])
        statements: list[str] = []
        with mock.patch.object(authored_factory, "_ch_exec", side_effect=lambda t, s: statements.append(s)), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")), \
             mock.patch.object(authored_factory.requests, "post", return_value=post) as mocked_post, \
             mock.patch.dict(os.environ, {"LAKEHOUSE_API_URL": "http://api.invalid:8080", "PIPELINE_RUN_TOKEN": "run-tok"}):
            result = run_fn(build_op_context())
        return result, statements, mocked_post

    def test_the_op_runs_the_sql_the_api_returned_and_never_its_own(self) -> None:
        """The whole point of D4: the `SELECT` this module builds is a
        REQUEST, not a command. Every statement that reads the source --
        the target `CREATE TABLE ... AS`, the staging `CREATE` and the
        staging `INSERT` -- must carry the API's text, and the unmasked
        original must appear in none of them."""
        result, statements, mocked_post = self._run_op_with(
            self._post_returning(200, {"sql": self.ENFORCED})
        )
        self.assertEqual(result["rows"], 3)
        self.assertEqual(mocked_post.call_count, 1, "exactly one enforcement call per run")
        sent = mocked_post.call_args.kwargs["json"]
        self.assertEqual(sent["sql"], "SELECT id, name FROM silver.`orders`")
        reads = [s for s in statements if "silver.`orders`" in s]
        self.assertEqual(len(reads), 3, f"expected CREATE, staging CREATE and INSERT: {reads}")
        for statement in reads:
            self.assertIn(self.ENFORCED, statement, f"op ran its own SQL: {statement}")
            self.assertNotIn("SELECT id, name FROM", statement, f"unmasked SQL reached ClickHouse: {statement}")

    def test_the_enforcement_call_is_authenticated_and_addressed_to_the_pipeline(self) -> None:
        """The route is the orchestrator-only one, so the run token goes
        with it, and the pipeline id in the URL is the row being run --
        URL-quoted, so an id could never inject a path segment."""
        _, _, mocked_post = self._run_op_with(self._post_returning(200, {"sql": self.ENFORCED}))
        url = mocked_post.call_args.args[0]
        headers = mocked_post.call_args.kwargs["headers"]
        self.assertEqual(url, "http://api.invalid:8080/api/pipelines/runnable/pl-dedupe-select-abc123/enforce")
        self.assertEqual(headers["Authorization"], "Bearer run-tok")
        self.assertEqual(headers["x-run-token"], "run-tok")
        self.assertEqual(headers["content-type"], "application/json")

    def _assert_refused_without_retry(self, post: Any) -> str:
        """A refusal fails the op on the first attempt, with the API's own
        fixed message, and no statement reaches ClickHouse."""
        from dagster import Failure

        with self.assertRaises(Failure) as ctx:
            self._run_op_with(post)
        self.assertFalse(ctx.exception.allow_retries)
        self.assertIsInstance(ctx.exception.__cause__, authored_factory.AuthoredEnforceRefused)
        return str(ctx.exception)

    def test_a_pipeline_with_no_author_fails_without_retry(self) -> None:
        """D2's 409: the pipeline has no recorded author, so the API refuses
        to rewrite anything. Retrying minutes later gets the same answer --
        only an administrator assigning an author changes it."""
        message = self._assert_refused_without_retry(
            self._post_returning(409, {"error": "this pipeline has no author to read as; an administrator must assign one"})
        )
        self.assertIn("no author to read as", message)

    def test_a_deactivated_author_fails_without_retry(self) -> None:
        """D2's other 409, same shape: the author's row exists but is not
        active, which is a decision, not an outage."""
        self._assert_refused_without_retry(
            self._post_returning(409, {"error": "this pipeline has no author to read as; an administrator must assign one"})
        )

    def test_a_refused_construct_fails_without_retry(self) -> None:
        """A 422 is the policy engine refusing a construct it will not
        rewrite -- also a decision, also not retried."""
        self._assert_refused_without_retry(
            self._post_returning(422, {"error": "Query menyentuh tabel yang diatur kebijakan melalui konstruksi yang tidak didukung"})
        )

    def test_a_transport_failure_is_left_retryable(self) -> None:
        """An unreachable API is infrastructure, not a decision: the
        exception propagates UNWRAPPED so the op's retry policy still sees
        a plain retryable failure, exactly as before D4 existed."""
        pipeline = _ready_pipeline()
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with mock.patch.object(authored_factory, "_ch_exec") as mocked_exec, \
             mock.patch.object(authored_factory, "_ch_query_json"), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")), \
             mock.patch.object(authored_factory.requests, "post", side_effect=requests.ConnectionError("refused")):
            with self.assertRaises(requests.ConnectionError):
                run_fn(build_op_context())
        mocked_exec.assert_not_called()

    def test_a_refusal_without_a_message_never_echoes_the_body(self) -> None:
        """The API's refusal body is fixed text by construction, so it is
        safe to put in an op failure. A body that is NOT the documented
        shape is not: we use our own fixed fallback rather than forwarding
        whatever arrived."""
        message = self._assert_refused_without_retry(self._post_returning(409, ["unexpected"]))
        self.assertIn(authored_factory._ENFORCE_REFUSAL_FALLBACK, message)
        self.assertNotIn("unexpected", message)

    def test_a_2xx_without_a_sql_is_refused_rather_than_run(self) -> None:
        """A response the op cannot read is never treated as "nothing to
        enforce", which would silently run the unmasked SQL."""
        self._assert_refused_without_retry(self._post_returning(200, {"sql": None}))


class OpForPipelineTest(unittest.TestCase):
    """Executes the built op's own function directly (not through a full
    Dagster job run) -- same level `test_maintenance.py` exercises its ops
    at: a real `dagster.build_op_context()`, since `@op`-decorated
    functions require a genuine execution context for direct invocation
    and reject a bare mock (see `test_maintenance.py`'s own note on this).
    `_ch_exec`/`_ch_query_json` are monkeypatched, the way
    `test_maintenance.py` monkeypatches `_ch_query`."""

    def test_ready_pipeline_reads_transforms_writes_and_reports_real_rows(self) -> None:
        """F1.4 (#57): `_write_clickhouse_table` is now ATOMIC: the body
        inserts the new rows into a `__staging_<run_id>` table under the
        same `target_zone`, then `EXCHANGE TABLES` swaps staging and
        target. The previous direct `INSERT INTO serving.`orders_clean``
        left a window where a partial mid-run write was visible to a
        parallel reader -- a non-atomic read against a consumer
        dashboard (this plan's `dagster_pipeline_orchestrator` is the
        unit, but the row schema here is the same `serving` zone used
        downstream). The two new asserts pin the staging-EXCHANGE
        sequence (an atomic INSERT on the staging table followed by
        EXCHANGE TABLES swapping the staging and target tables).
        """
        pipeline = _ready_pipeline()
        run_fn = authored_factory._op_for_pipeline(pipeline)

        # _write_clickhouse_table now does 2 count() calls (before
        # insert on staging, after exchange on target -- the staging
        # table is dropped after EXCHANGE, so the post-count must
        # read the swapped-in target).
        counts = iter([[{"n": "0"}], [{"n": "3"}]])  # before, after
        exec_calls: list[str] = []
        with mock.patch.object(authored_factory, "_ch_exec", side_effect=lambda t, s: exec_calls.append(s)), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")), \
             _enforce_passthrough():
            result = run_fn(build_op_context())

        self.assertEqual(result["rows"], 3)
        self.assertEqual(result["skipped_verbs"], [])
        # The INSERT must target a staging table, not the live target.
        insert_calls = [c for c in exec_calls if "INSERT INTO" in c]
        self.assertEqual(len(insert_calls), 1, "exactly one INSERT per run")
        self.assertNotIn("INSERT INTO serving.`orders_clean`", insert_calls[0])
        self.assertIn("INSERT INTO serving.`__staging_orders_clean", insert_calls[0])
        # The atomic swap follows the staging INSERT.
        exchange_calls = [c for c in exec_calls if "EXCHANGE TABLES" in c]
        self.assertEqual(len(exchange_calls), 1, "exactly one EXCHANGE per run")
        self.assertIn("serving.`__staging_orders_clean", exchange_calls[0])
        self.assertIn("serving.`orders_clean`", exchange_calls[0])

    def test_atomic_write_succeeds_on_first_attempt(self) -> None:
        """F1.4 (#57): the staging-EXCHANGE sequence completes in one
        op invocation and reports a real row delta. The `_run` op
        body's `SELECT count()` AFTER sequence must see the row count
        of the SWAPPED-IN target (which is what readers see)."""
        pipeline = _ready_pipeline()
        run_fn = authored_factory._op_for_pipeline(pipeline)

        # First count(): before-staging-insert (target is the empty one
        # from `_ensure_target_table`, so 0). Second count(): after
        # EXCHANGE, on the swapped-in target that started as staging
        # and now holds the 3 rows -- 3.
        counts = iter([[{"n": "0"}], [{"n": "3"}]])
        with mock.patch.object(authored_factory, "_ch_exec"), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")), \
             _enforce_passthrough():
            result = run_fn(build_op_context())

        self.assertEqual(result["rows"], 3)

    def test_atomic_write_is_idempotent_under_retry(self) -> None:
        """F1.4 (#57): a retry of the same op invocation must produce
        the SAME final state on `serving.orders_clean` as a single
        successful run. The atomic staging path makes the operation
        idempotent: on retry the staging table is dropped (if left
        over from a failed prior attempt) and re-created before the
        INSERT. The op-body then runs to completion on the second
        attempt, and the row delta is 3 -- not 6, which would have
        been the symptom of a direct INSERT under retry.

        We invoke the op body twice in the same test process (mirroring
        `test_op_source_metadata.py::test_default_retry_policy_retries_a_transient_network_error`
        which calls its op body twice to simulate a retry) and assert
        the second invocation sees a final row count equal to the
        first invocation's row count -- i.e. the swap puts the target
        back to its post-write shape, not to its pre-write + staging
        shape."""
        pipeline = _ready_pipeline()
        run_fn = authored_factory._op_for_pipeline(pipeline)

        # Each invocation: before=0, after=3. We don't share state
        # between calls because `_write_clickhouse_table` drops the
        # staging table before the INSERT, so each run begins from
        # the same starting point. One iter per invocation (not a
        # factory), because the lambda closes over the iter -- if a
        # fresh iter were returned on every call, the second
        # `_ch_query_json` would also read `{"n":"0"}` and the row
        # delta would be 0.
        counts_1 = iter([[{"n": "0"}], [{"n": "3"}]])
        # First invocation -- staging-write + EXCHANGE.
        with mock.patch.object(authored_factory, "_ch_exec"), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts_1)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")), \
             _enforce_passthrough():
            first = run_fn(build_op_context())
        self.assertEqual(first["rows"], 3)

        counts_2 = iter([[{"n": "0"}], [{"n": "3"}]])
        # Second invocation -- same op body, must still report 3
        # rows of delta (no double-count, no leftover staging rows).
        with mock.patch.object(authored_factory, "_ch_exec"), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts_2)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")), \
             _enforce_passthrough():
            second = run_fn(build_op_context())
        self.assertEqual(second["rows"], 3)

    def test_fbic_enabled_pipeline_records_a_skipped_verb_not_a_call(self) -> None:
        pipeline = _ready_pipeline(fbicEnabled=True)
        run_fn = authored_factory._op_for_pipeline(pipeline)

        counts = iter([[{"n": "0"}], [{"n": "1"}]])
        with mock.patch.object(authored_factory, "_ch_exec"), \
             mock.patch.object(authored_factory, "_ch_query_json", side_effect=lambda t, s: next(counts)), \
             mock.patch.object(authored_factory.ClickHouseTarget, "from_env", return_value=ClickHouseTarget("http://ch", "default", "")), \
             _enforce_passthrough():
            result = run_fn(build_op_context())

        self.assertEqual(result["skipped_verbs"], [authored_factory.FBIC_UNSUPPORTED_REASON])

    def test_a_rejected_transform_raises_a_non_retryable_failure_wrapping_transform_error(
        self,
    ) -> None:
        """PART D: `TransformError` from `parse_transform` is a config
        rejection -- the transform string is wrong, retrying with the
        same string gets the same answer. The `_run` op body wraps it
        in `Failure(allow_retries=False)`, the original `TransformError`
        is preserved as `__cause__` (the message names the offending
        field), and the run fails on the first attempt without burning
        60s on retries that cannot succeed."""
        from dagster import Failure

        pipeline = _ready_pipeline(transforms=["exec(rm -rf /)"])
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with self.assertRaises(Failure) as ctx:
            run_fn(build_op_context())
        self.assertFalse(ctx.exception.allow_retries)
        self.assertIsInstance(ctx.exception.__cause__, authored_transforms.TransformError)

    def test_a_bronze_target_is_refused_before_any_sql_is_sent(self) -> None:
        """BLD-3 / D4: Bronze is written by ingestion, so a pipeline with a
        `bronze` target is a config rejection, not a run failure to retry --
        retrying gets the same refusal. `Failure(allow_retries=False)`
        wraps `AuthoredJobError` (the shape the connector-source guard
        above uses), the guard fires BEFORE any SQL is built, and the
        wording is the one constant the API and the assistant's schema
        refusal also use."""
        from dagster import Failure

        pipeline = _ready_pipeline(targetZone="bronze")
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with mock.patch.object(authored_factory, "_ch_exec") as mocked_exec, \
             mock.patch.object(authored_factory, "_ch_query_json") as mocked_query:
            with self.assertRaises(Failure) as ctx:
                run_fn(build_op_context())
        self.assertFalse(ctx.exception.allow_retries)
        self.assertIsInstance(ctx.exception.__cause__, authored_factory.AuthoredJobError)
        self.assertIn(authored_factory.BRONZE_TARGET_REFUSED_REASON, str(ctx.exception))
        self.assertEqual(
            authored_factory.BRONZE_TARGET_REFUSED_REASON,
            "a pipeline cannot write to bronze; bronze is written by ingestion",
        )
        mocked_exec.assert_not_called()
        mocked_query.assert_not_called()

    def test_connector_sourced_pipeline_raises_a_non_retryable_failure_wrapping_authored_job_error(
        self,
    ) -> None:
        """PART D: an authored pipeline that names a `connectorId` is
        not implemented by this build (see `CONNECTOR_SOURCE_UNSUPPORTED_REASON`)
        -- a missing-feature gap, never a transient failure. Wrapped in
        `Failure(allow_retries=False)` so the run fails loudly on the
        first attempt."""
        from dagster import Failure

        pipeline = _ready_pipeline(connectorId="conn-1")
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with self.assertRaises(Failure) as ctx:
            run_fn(build_op_context())
        self.assertFalse(ctx.exception.allow_retries)
        self.assertIn("connector-sourced", str(ctx.exception))
        self.assertIsInstance(ctx.exception.__cause__, authored_factory.AuthoredJobError)

    def test_an_unsafe_identifier_raises_a_non_retryable_failure_and_reaches_no_sql(
        self,
    ) -> None:
        """PART D: an unsafe ClickHouse identifier in any of the four
        identifier fields is a config rejection -- retrying with the
        same string gets the same answer. Wrapped in
        `Failure(allow_retries=False)`, and `_ch_exec` is never
        called (the guard fires before any SQL is built)."""
        from dagster import Failure

        pipeline = _ready_pipeline(targetTable="orders; DROP TABLE x")
        run_fn = authored_factory._op_for_pipeline(pipeline)
        with mock.patch.object(authored_factory, "_ch_exec") as mocked_exec:
            with self.assertRaises(Failure) as ctx:
                run_fn(build_op_context())
        self.assertFalse(ctx.exception.allow_retries)
        self.assertIsInstance(ctx.exception.__cause__, authored_factory.AuthoredJobError)
        mocked_exec.assert_not_called()


class RunnableFetchTest(unittest.TestCase):
    def test_the_factory_reads_the_runnable_route_and_keeps_paused_pipelines(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [
            _ready_pipeline(),
            {**_ready_pipeline(), "id": "pl-paused-1", "status": "paused"},
            {**_ready_pipeline(), "id": "pl-draft-1", "status": "draft"},
        ]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp) as mocked_get:
            result = authored_factory._fetch_authored_pipelines(_cfg())
        self.assertTrue(mocked_get.call_args.args[0].endswith("/api/pipelines/runnable"))
        self.assertEqual(
            [p["id"] for p in result], ["pl-dedupe-select-abc123", "pl-paused-1"]
        )


class AuthoredScheduleTest(unittest.TestCase):
    def _job(self, pipeline: dict[str, object]) -> object:
        return authored_factory.build_authored_job(pipeline)

    def test_a_ready_pipeline_with_a_cron_gets_a_running_schedule(self) -> None:
        pipeline = {**_ready_pipeline(), "schedule": "0 2 * * *"}
        schedule = authored_factory.build_authored_schedule(pipeline, self._job(pipeline))
        self.assertIsNotNone(schedule)
        self.assertEqual(schedule.name, "authored__pl_dedupe_select_abc123_schedule")
        self.assertEqual(schedule.cron_schedule, "0 2 * * *")
        self.assertEqual(schedule.default_status, DefaultScheduleStatus.RUNNING)

    def test_a_paused_pipeline_gets_a_stopped_schedule(self) -> None:
        pipeline = {**_ready_pipeline(), "status": "paused", "schedule": "*/15 * * * *"}
        schedule = authored_factory.build_authored_schedule(pipeline, self._job(pipeline))
        self.assertEqual(schedule.default_status, DefaultScheduleStatus.STOPPED)

    def test_pipeline_with_target_lag_builds_no_schedule(self) -> None:
        """BLD-2 D3: a pipeline with targetLag (either kind) builds no schedule,
        even when its schedule is a valid cron expression. Removing the lag
        returns the pipeline to the schedule."""
        for lag in ({"kind": "lag", "minutes": 10}, {"kind": "downstream"}):
            pipeline = {**_ready_pipeline(), "schedule": "0 2 * * *", "targetLag": lag}
            self.assertIsNone(authored_factory.build_authored_schedule(pipeline, self._job(pipeline)))
        restored = {**_ready_pipeline(), "schedule": "0 2 * * *", "targetLag": None}
        self.assertIsNotNone(authored_factory.build_authored_schedule(restored, self._job(restored)))

    def test_an_on_demand_pipeline_gets_no_schedule(self) -> None:
        for label in ("manual", "On demand", "", None):
            pipeline = {**_ready_pipeline(), "schedule": label}
            self.assertIsNone(authored_factory.build_authored_schedule(pipeline, self._job(pipeline)))

    def test_a_cron_dagster_refuses_is_skipped_not_raised(self) -> None:
        pipeline = {**_ready_pipeline(), "schedule": "99 99 * * *"}
        self.assertIsNone(authored_factory.build_authored_schedule(pipeline, self._job(pipeline)))

    def test_jobs_and_schedules_come_from_one_fetch(self) -> None:
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [
            {**_ready_pipeline(), "schedule": "0 2 * * *"},
            {**_ready_pipeline(), "id": "pl-manual-1", "schedule": "manual"},
        ]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp) as mocked_get:
            jobs, schedules, deps = authored_factory.build_authored_definitions(_cfg())
        self.assertEqual(mocked_get.call_count, 1)
        self.assertEqual(len(jobs), 2)
        self.assertEqual([s.name for s in schedules], ["authored__pl_dedupe_select_abc123_schedule"])
        # F1.1: the third element is the `(pipeline, job)` list the
        # sensor builder now consumes — one entry per fetched pipeline,
        # and the `job` is the SAME `JobDefinition` instance the
        # `jobs` list holds. A second `build_authored_job` call here is
        # what F1.1 (BLOCKER) was about: the same name on two distinct
        # `JobDefinition` objects crashes `Definitions` with
        # `DagsterInvalidDefinitionError: Duplicate job definition found`.
        self.assertEqual(len(deps), 2)
        jobs_set = {id(j) for j in jobs}
        self.assertTrue(
            all(id(dep_job) in jobs_set for _dep_pipeline, dep_job in deps),
            "the sensor builder must reuse the jobs the jobs list holds",
        )


# ── R3 plan 2a: dependency-sensor tests (appended below) ──────────────
#
# The sensor body is a closure over `pipeline["id"]` and `pipeline["dependsOn"]`;
# every other field of the pipeline dict is irrelevant to it. These tests
# build the minimum dict the body reads on top of HEAD's `_ready_pipeline`
# (so the runnable-payload fixture stays the single source of truth for the
# `definition` shape) and never call the network -- `context.instance` is
# replaced by `StubInstance`, which records every `RunsFilter` the sensor
# builds and returns a fixed list per call.


class _FakeDagsterRun:
    """Stand-in for `dagster.DagsterRun` carrying only the fields the
    sensor body reads (`job_name`, `start_time`, `end_time`, `run_id`,
    `status`, and -- for a `file_ingest_job` run -- its resolved
    `run_config`). The factory module never constructs one of these -- it
    only consumes whatever `context.instance.get_runs` returns -- so a
    minimal stub is enough."""

    def __init__(
        self,
        job_name: str,
        run_id: str,
        status: DagsterRunStatus,
        start_time: float | None,
        end_time: float | None,
        run_config: Any = None,
    ) -> None:
        self.job_name = job_name
        self.run_id = run_id
        self.status = status
        self.start_time = start_time
        self.end_time = end_time
        self.run_config = run_config


class _FakeRunRecord:
    """Drop-in for `dagster._core.storage.dagster_run.RunRecord`.

    In Dagster 1.13.20 the run's `start_time`/`end_time` live HERE and not
    on the `DagsterRun` (`DATA-19` gate finding, `authored_factory`'s
    "Why RECORDS and not runs"), so the stub is faithful to that shape
    rather than to what an older Dagster returned. A stub with the times
    on the run itself is what let `R3 plan 2a`'s sensor pass its unit
    tests and fail on every real tick."""

    def __init__(self, run: _FakeDagsterRun) -> None:
        self.dagster_run = run
        self.start_time = run.start_time
        self.end_time = run.end_time


class _FakeMaterialization:
    """Drop-in for `dagster._core.events.log.EventLogRecord` -- the element
    `DagsterInstance.fetch_materializations` returns inside its
    `EventRecordsResult`. Only the four properties the sensor's per-table
    filter reads are carried (`run_id`, `timestamp`, `asset_key`, and the
    `asset_materialization` event itself), because the real call filters by
    asset key ONLY: it has no `job_name` and no status field, which is why
    the run's status has to come from the run side (`authored_factory`
    `_latest_record_for_ingest_table`)."""

    def __init__(self, run_id: str, timestamp: float, asset_key: AssetKey) -> None:
        self.run_id = run_id
        self.timestamp = timestamp
        self.asset_key = asset_key


class _FakeEventRecordsResult:
    """The `EventRecordsResult` shape: one `records` attribute, newest
    first, because `fetch_materializations` defaults to `ascending=False`
    (verified in the plan's section 5.1)."""

    def __init__(self, records: Sequence[_FakeMaterialization]) -> None:
        self.records = list(records)


class _StubInstance:
    """Drop-in for `context.instance`. Records every `(filter, limit)`
    pair the sensor asks for and returns the record list the test set up
    for that pair, newest first, honouring the `limit` the sensor chose.
    Indexed by `RunsFilter.job_name` so the sensor's per-upstream query
    and per-downstream query can be answered independently, and by asset
    key path for `fetch_materializations`.

    Only `get_run_records` and `fetch_materializations` exist here on
    purpose: a sensor that went back to `get_runs` would raise
    `AttributeError` in every test rather than quietly reading times off
    an object that no longer has them."""

    def __init__(
        self,
        runs_by_job: dict[str, list[_FakeDagsterRun]],
        materializations: dict[tuple[str, ...], list[_FakeMaterialization]] | None = None,
    ) -> None:
        self._runs_by_job = runs_by_job
        self._materializations = materializations or {}
        self.calls: list[tuple[RunsFilter | None, int | None]] = []
        self.materialization_calls: list[tuple[Any, int]] = []

    def get_run_records(
        self,
        filters: RunsFilter | None = None,
        cursor: str | None = None,
        limit: int | None = None,
        bucket_by: Any = None,
        ascending: bool = False,
    ) -> Sequence[_FakeRunRecord]:
        self.calls.append((filters, limit))
        if filters and filters.job_name:
            runs = list(self._runs_by_job.get(filters.job_name, []))
        else:
            runs = [r for job_runs in self._runs_by_job.values() for r in job_runs]
        if filters and filters.run_ids:
            # BLD-4 review R1 BLOCKER 1 / SHOULD-FIX 5
            run_ids_set = set(filters.run_ids)
            runs = [r for r in runs if r.run_id in run_ids_set]
        if filters and filters.statuses:
            # BLD-4 review R1 SHOULD-FIX 5
            statuses_set = set(filters.statuses)
            runs = [r for r in runs if r.status in statuses_set]
        if limit is not None:
            runs = runs[:limit]
        return [_FakeRunRecord(run) for run in runs]

    def fetch_materializations(
        self,
        records_filter: Any,
        limit: int,
        cursor: Any = None,
        ascending: bool = False,
    ) -> _FakeEventRecordsResult:
        self.materialization_calls.append((records_filter, limit))
        key = self._asset_key_path(records_filter)
        records = list(self._materializations.get(key, []))
        if limit is not None:
            records = records[:limit]
        return _FakeEventRecordsResult(records)

    @staticmethod
    def _asset_key_path(records_filter: Any) -> tuple[str, ...]:
        """The asset key a filter names, as a tuple. A plain `AssetKey` and
        an `AssetRecordsFilter` (whose key is its `asset_key` attribute) are
        both accepted, so a test stays honest about which one the sensor
        build passes."""
        asset_key = getattr(records_filter, "asset_key", None) or records_filter
        return tuple(asset_key.path)


class _StubContext:
    """Carries the two attributes the sensor body reads:
    `dagster_run` (the upstream run that triggered THIS tick) and
    `instance` (the run-storage stub)."""

    def __init__(self, dagster_run: _FakeDagsterRun, instance: _StubInstance) -> None:
        self.dagster_run = dagster_run
        self.instance = instance


def _dep_pipeline(pid: str, depends_on: list[str]) -> dict[str, Any]:
    """A pipeline dict shaped for the sensor body. Mirrors the REAL
    wire format the Rust `RunnablePipeline` serializes
    (`#[serde(rename_all = "camelCase")]` in
    `rust/crates/lakehouse-store/src/pipelines.rs`): the upstream-list
    field is `dependsOn` on the wire, never `depends_on`. The body
    reads `pipeline["id"]` and `pipeline["dependsOn"]`; everything
    else is HEAD's `_ready_pipeline` shape so the `definition`
    fixture stays the single source of truth."""
    return {**_ready_pipeline(), "id": pid, "dependsOn": depends_on}


def _dep_pipeline_status(pid: str, depends_on: list[str], status: str) -> dict[str, Any]:
    """A pipeline dict with an explicit `status` field, so the F1.2
    paused-skip test can exercise the `paused` branch without changing
    the default `ready` shape every other test in this class relies on."""
    return {**_dep_pipeline(pid, depends_on), "status": status}


def _build_dep_sensor(pid: str, depends_on: list[str]):
    """Helper: build the pipeline dict, then build and return the
    sensor `RunStatusSensorDefinition`. The sensor body is a closure
    over `pid` and `depends_on`, so this is the only factory the
    tests need. `request_job` MUST be a real `JobDefinition` (not a
    `JobSelector`) because Dagster 1.13.20's `SensorDefinition`
    superclass coerces it through `AutomationTarget.from_coercible`,
    which only handles `JobDefinition`/`UnresolvedAssetJobDefinition`
    -- a placeholder `@job` named the same way the factory would
    name it is enough."""
    @job(name=f"authored__{pid.replace('-', '_')}")
    def _placeholder_job() -> None:
        return None
    return build_authored_dependency_sensor(
        _dep_pipeline(pid, depends_on), authored_job=_placeholder_job
    )


class TestDependencySensor:
    """Sensor-body tests for R3 plan 2a. Plain pytest class (no
    `unittest.TestCase`) so the per-test asserts render with pytest's
    own introspection instead of `self.assertEqual`'s."""

    def test_returns_none_when_target_lag_is_set(self) -> None:
        """BLD-2 D3: a pipeline with targetLag (either kind) builds no dependency sensor,
        even when dependsOn is populated. Removing target lag restores the sensor."""
        @job(name="authored__pl_lag_dep")
        def _job() -> None:
            return None
        for lag in ({"kind": "lag", "minutes": 10}, {"kind": "downstream"}):
            pipeline = {**_dep_pipeline("pl-lag-dep", ["pl-up-a"]), "targetLag": lag}
            assert build_authored_dependency_sensor(pipeline, _job) is None
        restored = {**_dep_pipeline("pl-lag-dep", ["pl-up-a"]), "targetLag": None}
        assert build_authored_dependency_sensor(restored, _job) is not None

    def test_returns_none_when_depends_on_is_empty(self) -> None:
        """A pipeline with `depends_on=[]` produces NO `run_status_sensor`:
        `Definitions(sensors=...)` is the union of every chain's first
        downstream, and "no chain" means "no sensor". An empty list there
        is the factory's whole point for pipelines without upstream wiring.
        """
        assert build_authored_dependency_sensor(_dep_pipeline("pl-empty", []), None) is None

    def test_returns_none_when_a_chained_pipeline_is_paused(self) -> None:
        """F1.2 SHOULD-FIX (#57): a `paused` pipeline with a non-empty
        `dependsOn` produces NO `run_status_sensor`. Dagster keeps the
        sensor's RUNNING/STOPPED state across a code-location reload,
        so a sensor shipped RUNNING continues to fire even after the
        pipeline's source `dependsOn` disappears — pausing the
        pipeline must therefore skip the sensor at code-load time, AND
        the API's `authored_status` route must stop the stored sensor
        next to the schedule. The first half is asserted here; the
        API half is the Rust `routes::pipelines::authored_status` wire
        in F1.2's second step."""
        @job(name="authored__pl_paused_down")
        def _placeholder_job() -> None:
            return None
        assert (
            build_authored_dependency_sensor(
                _dep_pipeline_status("pl-paused-down", ["pl-up-a"], "paused"),
                _placeholder_job,
            )
            is None
        )

    def test_returns_a_sensor_when_depends_on_is_set(self) -> None:
        """A non-empty `depends_on` produces a sensor with the
        `authored__<safe_id>_after` name and `JobSelector`s for `monitored_jobs`
        (verified by inspecting the `RunStatusSensorDefinition`'s internal
        `_monitored_jobs` -- the public surface has no `monitored_jobs`
        property in dagster 1.13.20)."""
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        assert sensor is not None
        assert sensor.name == "authored__pl_down_1_after"
        assert sensor.default_status.name == "RUNNING"
        assert list(sensor._monitored_jobs) == [  # type: ignore[attr-defined]
            JobSelector(
                location_name=DAGSTER_LOCATION,
                repository_name=DAGSTER_REPO,
                job_name="authored__pl_up_a",
            )
        ]
        assert sensor.job is not None
        assert sensor.job.name == "authored__pl_down_1"

    def test_upstream_job_selector_resolves_authored_id_through_dagster_safe_name(self) -> None:
        """`pl-up-a` -> `authored__pl_up_a`; the same `_dagster_safe_name`
        rule the job/schedule names use, so the `run_status_sensor`'s
        `monitored_jobs` actually matches what the factory built."""
        assert _upstream_job_selector("pl-up-a") == JobSelector(
            location_name=DAGSTER_LOCATION,
            repository_name=DAGSTER_REPO,
            job_name="authored__pl_up_a",
        )

    def test_upstream_job_selector_passes_a_dagster_native_id_through_verbatim(self) -> None:
        """Dagster-native upstreams (e.g. `ingest_job`) are already valid
        job names; the factory uses the id as the `job_name` field on the
        selector rather than re-running `_dagster_safe_name`."""
        assert _upstream_job_selector("ingest_job") == JobSelector(
            location_name=DAGSTER_LOCATION,
            repository_name=DAGSTER_REPO,
            job_name="ingest_job",
        )

    def test_yields_run_request_with_run_key_equal_to_sorted_upstream_run_ids_when_all_fresh(self) -> None:
        """Happy path: the downstream has one prior run (start_time=100);
        upstream has one SUCCESS run that ended at 200 (>100). Yield
        `RunRequest(run_key=authored-deps:<sorted upstream run ids>)`.

        F1.3 (#57): the OLD `run_key = upstream_run.run_id` would launch
        one downstream per upstream in a multi-upstream chain (each
        upstream's own tick yielded a key derived only from itself; the
        daemon's dedup was per-key, not per-round). The new key is the
        sorted tuple of every upstream's latest SUCCESS run id, prefixed
        with `authored-deps:`, so two ticks that see the same set of
        upstream successes ask for the same downstream launch once.

        With one upstream the new value is the same as the old (modulo
        the prefix); the multi-upstream "one run per round" property is
        asserted in `test_yields_one_run_request_for_two_upstream_events`.
        """
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                "authored__pl_up_a": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        request = out[0]
        assert isinstance(request, RunRequest)
        assert request.run_key == "authored-deps:upstream-1"

    def test_yields_skip_reason_naming_the_stale_upstream_when_one_is_behind(self) -> None:
        """One upstream's latest SUCCESS ends at 50 (< downstream start
        100). The chain must NOT fire; the `SkipReason` must name the
        upstream that is behind (so the UI can render "waiting on pl-up-b").
        A bare "wait" would force the operator to read the sensor's
        `monitored_jobs` to figure out which dependency stalled."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                # `pl-up-a` is fresh (200 > 100); `pl-up-b` is stale (50 <
                # 100). The first stale upstream encountered is named in
                # the skip reason -- the body returns immediately on the
                # first failure to keep the per-tick work bounded.
                "authored__pl_up_a": [upstream_run],
                "authored__pl_up_b": [
                    _FakeDagsterRun(
                        job_name="authored__pl_up_b",
                        run_id="upstream-b-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=40.0,
                        end_time=50.0,
                    ),
                ],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a", "pl-up-b"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        reason = out[0]
        assert not isinstance(reason, RunRequest)
        # `SkipReason.skip_message` is the documented attribute (str).
        assert "pl-up-b" in reason.skip_message
        # The body should have asked for the stale upstream's runs and
        # stopped -- no need to query `pl-up-a` again on the same tick.
        upstream_b_calls = [
            c
            for c in instance.calls
            if c[0] is not None and c[0].job_name == "authored__pl_up_b"
        ]
        assert upstream_b_calls, "the sensor must have queried the stale upstream"

    def test_yields_run_request_immediately_on_first_downstream_run(self) -> None:
        """When the downstream has NEVER run, `downstream_start` is `None`.
        The chain must fire on this upstream's tick: there is no prior
        downstream start to compare against, so "after the downstream's
        start" is trivially satisfied. Without this rule, the very first
        run of any chained downstream would never fire -- it would always
        see its upstream's latest SUCCESS as "before the downstream's
        start" because there is no downstream start."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=10.0,
            end_time=20.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [],
                "authored__pl_up_a": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "authored-deps:upstream-1"

    def test_queries_only_success_runs_for_upstreams(self) -> None:
        """The freshness walk asks `context.instance.get_runs` with
        `statuses=[DagsterRunStatus.SUCCESS]` for every upstream. A FAILURE
        run on the upstream does not refresh the chain -- otherwise a
        failing upstream whose last successful run was ages ago would
        silence the sensor into a false-green. The downstream query has no
        status filter (it just needs the latest start)."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                "authored__pl_up_a": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        list(body(_StubContext(upstream_run, instance)))
        upstream_queries = [
            f
            for f, _limit in instance.calls
            if f is not None and f.job_name == "authored__pl_up_a"
        ]
        assert upstream_queries, "the sensor must have queried the upstream"
        for f in upstream_queries:
            assert f.statuses == [DagsterRunStatus.SUCCESS]

    def test_treats_missing_upstream_success_run_as_stale(self) -> None:
        """An upstream that has NEVER had a SUCCESS run is stale by
        definition. The `latest is None` branch must produce a
        `SkipReason` -- the chain cannot fire on hope."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                # No `authored__pl_up_a` entry -> instance.get_runs
                # returns `[]`. The walk must still treat this as stale.
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        assert "pl-up-a" in out[0].skip_message

    def test_uses_job_selector_with_dagster_native_id_when_upstream_is_native(self) -> None:
        """When `depends_on` lists a Dagster-native job (e.g. `ingest_job`,
        not a `pl-` id), the sensor watches that job's SUCCESS runs
        verbatim -- Dagster itself resolves the selector to a job that is
        not in this code location, which is the whole point of
        `JobSelector(job_name=id)` (vs the authored branch, which goes
        through `_dagster_safe_name`).

        The pipeline built here reads `silver.orders`, which after BLD-4 / D2
        a `ingest_job` upstream can never feed: that job writes Bronze, so a
        non-Bronze source has no table to match and the chain waits instead
        of firing. This test used to assert the tick yielded a `RunRequest`
        on the unchanged silver pipeline; that was the behaviour the feature
        removes, so the assertion is the D2 fail-closed skip
        (`AGENTS.md` rule 2: the test was wrong about what should happen, and
        says why it changed). The firing half of this pair lives in
        `TestIngestUpstreamIsFilteredByTable`;
        `test_upstream_job_selector_passes_a_dagster_native_id_through_verbatim`
        pins the selector itself."""
        upstream_run = _FakeDagsterRun(
            job_name="ingest_job",
            run_id="ingest-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                "ingest_job": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["ingest_job"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        assert not isinstance(out[0], RunRequest)
        assert "ingest_job" in out[0].skip_message
        assert "connector load" in out[0].skip_message

    def test_first_downstream_run_does_not_fire_when_one_upstream_never_succeeded(self) -> None:
        """F1.3 (#57): on the chain's first-ever tick (the downstream has
        no runs yet), the sensor body MUST check that EVERY upstream has had
        a SUCCESS run -- even when the downstream has never run before,
        "first run" does not waive ALL-semantics, only the "must-be-newer-
        than-downstream-start" comparison.

        The buggy old code took `if downstream_start is None: yield
        RunRequest(...)` and returned before the upstream walk. With one
        upstream that has never run, that path launched the downstream on
        the first success of any OTHER upstream and never waited for the
        missing one -- silently dead-chains that the UI then had to
        diagnose. The fix walks every upstream first and yields
        `SkipReason` when one has never had a SUCCESS."""
        triggering_upstream = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-a-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=10.0,
            end_time=20.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [],
                # `pl-up-a` has one SUCCESS run; `pl-up-b` has never
                # had a SUCCESS run. ALL semantics means the chain must
                # NOT fire on `pl-up-a`'s tick.
                "authored__pl_up_a": [triggering_upstream],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a", "pl-up-b"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(triggering_upstream, instance)))
        assert len(out) == 1
        assert not isinstance(out[0], RunRequest)
        assert "pl-up-b" in out[0].skip_message
        assert "never had a SUCCESS run" in out[0].skip_message

    def test_yields_skip_reason_when_downstream_latest_run_is_queued(self) -> None:
        """F1.3 (#57): a downstream whose latest run has not started yet
        (status `QUEUED` / `NOT_STARTED` -- the `start_time` is None)
        must NOT launch another downstream run. The old `get_runs`
        comment said "filtered by status" but the call had NO status
        filter, so `latest_downstream.start_time` was None for a queued
        run and the body treated it as "no downstream run yet" -- a
        false-green that double-fires the chain until the queued run
        finishes. The fix short-circuits on `start_time is None`
        before any upstream walk."""
        upstream_run = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        queued_downstream = _FakeDagsterRun(
            job_name="authored__pl_down_1",
            run_id="downstream-queued-1",
            # `start_time = None` is the queued / not-yet-started shape.
            status=DagsterRunStatus.QUEUED,
            start_time=None,
            end_time=None,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [queued_downstream],
                "authored__pl_up_a": [upstream_run],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        out = list(body(_StubContext(upstream_run, instance)))
        assert len(out) == 1
        assert not isinstance(out[0], RunRequest)
        assert "queued run" in out[0].skip_message
        assert "downstream-queued-1" in out[0].skip_message

    def test_yields_one_run_request_for_two_upstream_events_in_one_round(self) -> None:
        """F1.3 (#57): the chain's `run_key` MUST be derived from the
        SORTED set of the upstreams' latest SUCCESS run ids, so two
        upstream ticks that both see the same ALL-fresh upstream set
        ask the daemon for the same downstream launch exactly once.
        The old code keyed each upstream on its own run id and the
        daemon's dedup kept each upstream from launching the same
        downstream twice FOR THAT UPCELLING -- but two ticks that both
        asked to launch yielded ONE downstream per tick, regardless of
        dedup, because the keys were different. The daemon therefore saw
        two consecutive RunRequests for a downstream that should have
        launched once.

        This test simulates the per-tick body twice (one body per
        upstream tick) and asserts both ticks yield the SAME
        `run_key`, so the daemon's own dedup keeps the second tick
        from launching a second downstream run.
        """
        upstream_a = _FakeDagsterRun(
            job_name="authored__pl_up_a",
            run_id="upstream-a-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        upstream_b = _FakeDagsterRun(
            job_name="authored__pl_up_b",
            run_id="upstream-b-1",
            status=DagsterRunStatus.SUCCESS,
            start_time=180.0,
            end_time=200.0,
        )
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    ),
                ],
                "authored__pl_up_a": [upstream_a],
                "authored__pl_up_b": [upstream_b],
            }
        )
        sensor = _build_dep_sensor("pl-down-1", ["pl-up-a", "pl-up-b"])
        body = sensor._run_status_sensor_fn  # type: ignore[attr-defined]
        # Tick #1 fires on upstream_a's SUCCESS run.
        out_a = list(body(_StubContext(upstream_a, instance)))
        assert len(out_a) == 1 and isinstance(out_a[0], RunRequest)
        # Tick #2 fires on upstream_b's SUCCESS run (after the daemon
        # has had a chance to launch the first downstream).
        out_b = list(body(_StubContext(upstream_b, instance)))
        assert len(out_b) == 1 and isinstance(out_b[0], RunRequest)
        # Same key for both ticks = one daemon-decided downstream run.
        assert out_a[0].run_key == out_b[0].run_key
        assert out_a[0].run_key == "authored-deps:upstream-a-1,upstream-b-1"


def _file_ingest_run(
    run_id: str, table: str | None, end_time: float = 200.0, start_time: float = 180.0
) -> _FakeDagsterRun:
    """A `file_ingest_job` SUCCESS run that loaded `table`, carrying the
    SAME run-config shape `routes::uploads::run_config` launches the job
    with (`ops.ingest_uploaded_file.config.bronze_table_name`). `table`
    of `None` is a run launched without that config at all."""
    config = (
        {
            "ops": {
                "ingest_uploaded_file": {
                    "config": {
                        "upload_id": "up-1",
                        "storage_keys": ["uploads/tenant/up-1.csv"],
                        "format": "delimited",
                        "bronze_table_name": table,
                        "load_mode": "replace",
                        "encoding": "utf-8",
                        "delimiter": ",",
                        "header_row": 0,
                        "columns": [],
                        "on_bad_value": "stop",
                    }
                }
            }
        }
        if table is not None
        else {}
    )
    return _FakeDagsterRun(
        job_name="file_ingest_job",
        run_id=run_id,
        status=DagsterRunStatus.SUCCESS,
        start_time=start_time,
        end_time=end_time,
        run_config=config,
    )


def _build_bronze_dep_sensor(pid: str, depends_on: list[str], source_zone: str, source_table: str):
    """A sensor for a pipeline whose SOURCE is `source_zone.source_table`
    -- the field DATA-19 / D1 filters a `file_ingest_job` upstream by."""
    pipeline = _dep_pipeline(pid, depends_on)
    pipeline["definition"] = {**_ready_pipeline()["definition"], "sourceZone": source_zone, "sourceTable": source_table}

    @job(name=f"authored__{pid.replace('-', '_')}")
    def _placeholder_job() -> None:
        return None

    return build_authored_dependency_sensor(pipeline, authored_job=_placeholder_job)


def _tick(sensor: Any, instance: _StubInstance, upstream_run: _FakeDagsterRun) -> list[Any]:
    return list(sensor._run_status_sensor_fn(_StubContext(upstream_run, instance)))


class TestFileIngestUpstreamIsFilteredByTable:
    """DATA-19 / D1: `file_ingest_job` is a per-TABLE job, so a pipeline
    that names it counts only the SUCCESS runs whose own run config says
    they loaded THAT pipeline's source table. A load of any other table is
    a run of a different chain and must not trigger this one -- otherwise
    every upload anywhere re-runs every Bronze pipeline in the
    deployment."""

    def _instance(self, table_runs: list[_FakeDagsterRun]) -> _StubInstance:
        return _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    )
                ],
                "file_ingest_job": table_runs,
            }
        )

    def test_a_successful_load_of_the_source_table_triggers_the_pipeline(self) -> None:
        """The whole point of the feature: a re-upload of the pipeline's
        OWN source table is a fresh SUCCESS run, so the chain fires with
        a run_key naming that run."""
        instance = self._instance([_file_ingest_run("up-1", "orders")])
        sensor = _build_bronze_dep_sensor("pl-down-1", ["file_ingest_job"], "bronze", "orders")
        out = _tick(sensor, instance, _file_ingest_run("tick", "orders"))

        assert len(out) == 1
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "authored-deps:up-1"

    def test_a_load_of_another_table_does_not_trigger_the_pipeline(self) -> None:
        """A newer SUCCESS run of `file_ingest_job` that loaded a DIFFERENT
        table is not a run of this chain. The unfiltered rule (newest run
        wins) would have fired here, and would have fired again on every
        upload in the deployment."""
        instance = self._instance(
            [
                _file_ingest_run("other-new", "invoices", end_time=250.0),
                _file_ingest_run("ours", "orders", end_time=200.0),
            ]
        )
        sensor = _build_bronze_dep_sensor("pl-down-1", ["file_ingest_job"], "bronze", "orders")
        out = _tick(sensor, instance, _file_ingest_run("tick", "other-new"))

        assert len(out) == 1
        # It did NOT skip: its own table's load IS fresh, so the chain is
        # allowed to run -- and the run_key names THAT run, never the
        # other table's.
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "authored-deps:ours"

    def test_only_a_load_of_another_table_is_not_even_a_success_for_this_chain(self) -> None:
        """With NO run for the source table at all, the chain waits and
        says why -- the reason names the table and the rule, because
        "waiting on file_ingest_job" alone reads as "the upload has not
        run yet" when the real answer is "a DIFFERENT upload has, twice".
        """
        instance = self._instance(
            [_file_ingest_run("other-1", "invoices"), _file_ingest_run("other-2", "shipments")]
        )
        sensor = _build_bronze_dep_sensor("pl-down-1", ["file_ingest_job"], "bronze", "orders")
        out = _tick(sensor, instance, _file_ingest_run("tick", "invoices"))

        assert len(out) == 1
        reason = out[0]
        assert not isinstance(reason, RunRequest)
        assert "orders" in reason.skip_message
        assert "file_ingest_job" in reason.skip_message

    def test_a_run_without_the_config_is_not_a_match(self) -> None:
        """A `file_ingest_job` run launched without the op config (a job
        launch, a hand-made run, a future config shape) reads as "not
        about our table", never as a match on an empty name."""
        instance = self._instance([_file_ingest_run("no-config", None)])
        sensor = _build_bronze_dep_sensor("pl-down-1", ["file_ingest_job"], "bronze", "orders")
        out = _tick(sensor, instance, _file_ingest_run("tick", "orders"))

        assert len(out) == 1
        assert not isinstance(out[0], RunRequest)

    def test_a_pipeline_whose_source_is_not_bronze_never_fires_on_an_upload(self) -> None:
        """A stored row can predate the API's refusal
        (`routes::authored_pipelines::validate_per_table_upstream`). A
        non-Bronze source has no Bronze table to match, so the filter
        matches nothing and the chain waits: unsupported, honestly, never
        "works, approximately" by firing on every upload."""
        instance = self._instance([_file_ingest_run("up-1", "orders")])
        sensor = _build_bronze_dep_sensor("pl-down-1", ["file_ingest_job"], "silver", "orders_clean")
        out = _tick(sensor, instance, _file_ingest_run("tick", "orders"))

        assert len(out) == 1
        reason = out[0]
        assert not isinstance(reason, RunRequest)
        assert "silver" in reason.skip_message or "orders_clean" in reason.skip_message

    def test_the_filtered_upstream_reads_more_than_one_run(self) -> None:
        """The other upstreams still ask for exactly ONE run; the filtered
        one asks for a bounded window, because "the newest run that loaded
        our table" is not in the first one whenever a later upload
        loaded another table."""
        instance = self._instance(
            [_file_ingest_run(f"other-{i}", "invoices") for i in range(3)]
            + [_file_ingest_run("ours", "orders", end_time=150.0)]
        )
        sensor = _build_bronze_dep_sensor("pl-down-1", ["file_ingest_job"], "bronze", "orders")
        _tick(sensor, instance, _file_ingest_run("tick", "orders"))

        limits = [limit for filters, limit in instance.calls if filters and filters.job_name == "file_ingest_job"]
        assert limits == [authored_factory.PER_TABLE_UPSTREAM_SCAN_LIMIT]

    def test_the_all_upstreams_rule_holds_with_a_filtered_and_an_authored_upstream(self) -> None:
        """Section 6, question 2: mixing a per-table `file_ingest_job`
        with an authored pipeline changes WHICH run of the first one
        counts, not the ALL rule: every upstream must be fresh, the first
        stale one ends the tick with its own reason, and one round of
        successes asks for one downstream run keyed on both."""
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    )
                ],
                # Fresh for THIS chain, but the newest run of the job
                # loaded another table.
                "file_ingest_job": [
                    _file_ingest_run("other-new", "invoices", end_time=250.0),
                    _file_ingest_run("ours", "orders", end_time=200.0),
                ],
                "authored__pl_up_a": [
                    _FakeDagsterRun(
                        job_name="authored__pl_up_a",
                        run_id="upstream-a-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=180.0,
                        end_time=210.0,
                    )
                ],
            }
        )
        sensor = _build_bronze_dep_sensor("pl-down-1", ["file_ingest_job", "pl-up-a"], "bronze", "orders")

        fired = _tick(sensor, instance, _file_ingest_run("tick", "orders"))
        assert len(fired) == 1
        assert fired[0].run_key == "authored-deps:ours,upstream-a-1"

        # The same graph with the AUTHORED upstream behind: the tick ends
        # on that upstream, by name, and asks for nothing.
        instance._runs_by_job["authored__pl_up_a"] = [
            _FakeDagsterRun(
                job_name="authored__pl_up_a",
                run_id="upstream-a-1",
                status=DagsterRunStatus.SUCCESS,
                start_time=40.0,
                end_time=50.0,
            )
        ]
        behind = _tick(sensor, instance, _file_ingest_run("tick", "orders"))
        assert len(behind) == 1
        assert not isinstance(behind[0], RunRequest)
        assert "pl-up-a" in behind[0].skip_message


def _ingest_run(
    run_id: str,
    end_time: float = 200.0,
    start_time: float = 180.0,
    status: DagsterRunStatus = DagsterRunStatus.SUCCESS,
) -> _FakeDagsterRun:
    """An `ingest_job` run: ONE static job for every connector, launched
    with the `connector_id` of the connector it loads
    (`ingest_factory._ingest_run_config`), so its config names the
    CONNECTOR and never the tables the load wrote. That is exactly why the
    per-table filter for this job reads materializations, not the config."""
    return _FakeDagsterRun(
        job_name="ingest_job",
        run_id=run_id,
        status=status,
        start_time=start_time,
        end_time=end_time,
        run_config={"ops": {"run_ingest": {"config": {"connector_id": "conn-1"}}}},
    )


def _materialized(run_id: str, table: str, timestamp: float = 250.0) -> _FakeMaterialization:
    """A materialization event logged inside run `run_id` for the Bronze
    table `table`, shaped as `ingest_factory.ingest_source_object` logs it
    (`AssetMaterialization(asset_key=f"bronze.{obj['target']}")`). The asset
    key is built through `authored_factory._bronze_asset_key` so the test
    cannot disagree with the filter about the key's shape."""
    return _FakeMaterialization(
        run_id=run_id,
        timestamp=timestamp,
        asset_key=authored_factory._bronze_asset_key(table),
    )


class TestIngestUpstreamIsFilteredByTable:
    """BLD-4 / D1: `ingest_job` is ONE job for every connector, so ANY
    connector's successful load fires every pipeline that names it, whatever
    table the pipeline reads. The anchor is instead the newest SUCCESS run
    that materialized the pipeline's own Bronze source table, read from the
    run's own materialization (`_latest_record_for_ingest_table`)."""

    def _instance(
        self,
        materializations: list[_FakeMaterialization],
        runs: list[_FakeDagsterRun] | None = None,
    ) -> _StubInstance:
        """The stub instance one tick of this chain sees: the downstream's
        own most-recent run, `ingest_job`'s SUCCESS runs, and the
        materializations `materializations` names -- indexed by each
        event's OWN asset key, exactly as the real event log is, so a
        materialization of `bronze.invoices` is never found under a query
        for `bronze.orders`. `runs` defaults to the one run
        `materializations` say loaded the table."""
        by_key: dict[tuple[str, ...], list[_FakeMaterialization]] = {}
        for materialization in materializations:
            by_key.setdefault(tuple(materialization.asset_key.path), []).append(
                materialization
            )
        return _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    )
                ],
                "ingest_job": runs if runs is not None else [],
            },
            materializations=by_key,
        )

    def _sensor(self, depends_on: list[str], source_zone: str = "bronze", source_table: str = "orders"):
        return _build_bronze_dep_sensor(
            "pl-down-1", depends_on, source_zone, source_table
        )

    def test_a_load_of_the_source_table_triggers_the_pipeline(self) -> None:
        """The whole point of the feature: the run that materialized the
        pipeline's own table is a fresh SUCCESS run, so the chain fires with
        a run_key naming THAT run."""
        ours = _ingest_run("ours")
        instance = self._instance([_materialized("ours", "orders")], runs=[ours])
        out = _tick(self._sensor(["ingest_job"]), instance, _ingest_run("tick"))

        assert len(out) == 1
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "authored-deps:ours"

    def test_a_load_of_another_table_does_not_trigger_the_pipeline(self) -> None:
        """A newer SUCCESS run of `ingest_job` that loaded a DIFFERENT table
        (same connector, other objects) is not a run of this chain. The
        unfiltered rule -- newest run of the job wins -- is what fired on
        every connector load in the deployment."""
        ours = _ingest_run("ours", end_time=200.0)
        other = _ingest_run("other-new", end_time=250.0)
        instance = self._instance(
            [
                _materialized("ours", "orders"),
                # The newer run really did load -- and did load
                # `invoices`, which is the whole point: one connector,
                # several objects, one run.
                _materialized("other-new", "invoices"),
            ],
            runs=[other, ours],
        )
        out = _tick(self._sensor(["ingest_job"]), instance, _ingest_run("tick"))

        assert len(out) == 1
        # It did NOT skip: its own table's load IS fresh, so the chain is
        # allowed to run -- and the run_key names THAT run, never the other
        # table's.
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "authored-deps:ours"

    def test_a_run_that_materialized_the_table_and_failed_does_not_trigger(self) -> None:
        """The anchor is the RUN's status, not the event's existence: the
        event log is append-only per step, so a run whose mapped step
        loaded the table and whose LATER step failed still carries the
        materialization (proven on a real instance in
        `TestIngestMaterializationKeyTest`). Such a run is not a load this
        chain may follow, so the tick looks past it to the SUCCESS run."""
        ours = _ingest_run("ours", end_time=200.0)
        # BLD-4 review R1 SHOULD-FIX 5: feed the failed run to the stub
        broken = _ingest_run("broken", end_time=250.0, status=DagsterRunStatus.FAILURE)
        instance = self._instance(
            [
                _materialized("broken", "orders", timestamp=260.0),
                _materialized("ours", "orders", timestamp=200.0),
            ],
            runs=[broken, ours],
        )
        out = _tick(self._sensor(["ingest_job"]), instance, _ingest_run("tick"))

        assert len(out) == 1
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "authored-deps:ours"

    def test_only_failed_loads_of_the_table_inside_the_window_skip(self) -> None:
        """When every materialization in the window belongs to a run that
        FAILED, the chain waits and says why: the table's last load did not
        succeed, which is a different fact from "never loaded"."""
        # BLD-4 review R1 SHOULD-FIX 5: feed the failed run to the stub
        broken = _ingest_run("broken", end_time=250.0, status=DagsterRunStatus.FAILURE)
        instance = self._instance(
            [_materialized("broken", "orders", timestamp=260.0)],
            runs=[broken],
        )
        out = _tick(self._sensor(["ingest_job"]), instance, _ingest_run("tick"))

        assert len(out) == 1
        assert not isinstance(out[0], RunRequest)
        assert "orders" in out[0].skip_message
        assert "ingest_job" in out[0].skip_message

    def test_more_than_twenty_success_runs_of_other_tables_after_own_run_still_triggers(self) -> None:
        """BLD-4 review R1 BLOCKER 1: The SUCCESS-run window is keyed on the
        table's materializations, not the deployment's newest runs. More than 20
        SUCCESS runs of other tables after this table's own SUCCESS run must
        still find this table's run and trigger the pipeline. Under the old
        deployment-wide window, 20 newer runs of other tables pushed ours out
        of the window, falsely skipping."""
        ours = _ingest_run("ours", end_time=200.0)
        other_runs = [
            _ingest_run(f"other-{i}", end_time=210.0 + i)
            for i in range(25)
        ]
        other_materializations = [
            _materialized(f"other-{i}", "invoices", timestamp=210.0 + i)
            for i in range(25)
        ]
        all_runs = list(reversed(other_runs)) + [ours]
        instance = self._instance(
            [_materialized("ours", "orders", timestamp=200.0)] + other_materializations,
            runs=all_runs,
        )
        out = _tick(self._sensor(["ingest_job"]), instance, _ingest_run("tick"))

        assert len(out) == 1
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "authored-deps:ours"

    def test_stale_anchor_skip_names_the_table_when_per_table_upstream_is_behind(self) -> None:
        """BLD-4 review R1 SHOULD-FIX 1: When a per-table upstream's latest
        SUCCESS run for this table is behind the downstream's start, the skip
        reason must name the table (D4) rather than falsely claiming the job
        itself has no SUCCESS run after the downstream start when other tables
        have loaded."""
        ours = _ingest_run("ours", end_time=50.0)
        other = _ingest_run("other-new", end_time=200.0)
        instance = self._instance(
            [
                _materialized("ours", "orders", timestamp=50.0),
                _materialized("other-new", "invoices", timestamp=200.0),
            ],
            runs=[other, ours],
        )
        out = _tick(self._sensor(["ingest_job"]), instance, other)

        assert len(out) == 1
        assert not isinstance(out[0], RunRequest)
        assert "orders" in out[0].skip_message
        assert "ingest_job" in out[0].skip_message
        assert "stale" in out[0].skip_message

    def test_nothing_inside_the_window_skips_with_the_table_named(self) -> None:
        """D4: the reason names the table, the job and the window size,
        because "waiting on ingest_job" alone reads as "no connector load
        has run yet" when the real answer is "loads of other tables
        have"."""
        other = _ingest_run("other-1", end_time=250.0)
        instance = self._instance(
            [_materialized("other-1", "invoices", timestamp=260.0)],
            runs=[other],
        )
        out = _tick(self._sensor(["ingest_job"]), instance, _ingest_run("tick"))

        assert len(out) == 1
        reason = out[0]
        assert not isinstance(reason, RunRequest)
        assert "orders" in reason.skip_message
        assert "ingest_job" in reason.skip_message
        assert f"in its last {authored_factory.PER_TABLE_UPSTREAM_SCAN_LIMIT} loads" in reason.skip_message
        assert "a load of another table never triggers this pipeline" in reason.skip_message

    def test_a_pipeline_whose_source_is_not_bronze_never_fires_on_a_connector_load(self) -> None:
        """D2, stored row that predates the API's refusal: a non-Bronze
        source has no Bronze table to match, so the filter matches nothing
        and the chain waits. Unsupported, honestly, never "works,
        approximately" by firing on every connector load."""
        ours = _ingest_run("ours")
        instance = self._instance(
            [_materialized("ours", "orders")], runs=[ours]
        )
        out = _tick(
            self._sensor(["ingest_job"], source_zone="silver", source_table="orders_clean"),
            instance,
            _ingest_run("tick"),
        )

        assert len(out) == 1
        reason = out[0]
        assert not isinstance(reason, RunRequest)
        assert "silver" in reason.skip_message or "orders_clean" in reason.skip_message
        assert "no connector load can trigger it" in reason.skip_message

    def test_the_filtered_upstream_asks_a_bounded_window_of_both_kinds(self) -> None:
        """One window for the whole filter (`AGENTS.md` rule 4: one window,
        not two): the materialization query and the SUCCESS-run query both
        use `PER_TABLE_UPSTREAM_SCAN_LIMIT`, and the SKIP-side query is made
        only when the asset key has materializations at all."""
        ours = _ingest_run("ours")
        instance = self._instance([_materialized("ours", "orders")], runs=[ours])
        _tick(self._sensor(["ingest_job"]), instance, _ingest_run("tick"))

        assert len(instance.materialization_calls) == 1
        records_filter, limit = instance.materialization_calls[0]
        # The asset key the filter asks for: TWO components, never the
        # one-component `bronze.orders` string (see
        # `TestIngestMaterializationKeyTest`).
        assert tuple(records_filter.path) == ("bronze", "orders")
        assert limit == authored_factory.PER_TABLE_UPSTREAM_SCAN_LIMIT
        # BLD-4 review R1 BLOCKER 1: run lookup is keyed on materializations' run_ids
        run_calls = [
            (filters, limit)
            for filters, limit in instance.calls
            if filters and filters.run_ids == ["ours"]
        ]
        assert run_calls == [
            (
                RunsFilter(
                    run_ids=["ours"],
                    statuses=[DagsterRunStatus.SUCCESS],
                ),
                authored_factory.PER_TABLE_UPSTREAM_SCAN_LIMIT,
            )
        ]

    def test_an_asset_key_that_was_never_loaded_makes_no_run_query(self) -> None:
        """A table nobody ever loaded is one instance call for THIS upstream,
        not two: no materialization means no SUCCESS-run query to join
        against."""
        instance = self._instance([])
        _tick(self._sensor(["ingest_job"]), instance, _ingest_run("tick"))

        assert len(instance.materialization_calls) == 1
        # BLD-4 review R1 BLOCKER 1
        ingest_run_calls = [
            (filters, limit)
            for filters, limit in instance.calls
            if filters and (filters.job_name == "ingest_job" or filters.run_ids is not None)
        ]
        assert ingest_run_calls == []

    def test_the_all_upstreams_rule_holds_with_an_ingest_and_an_authored_upstream(self) -> None:
        """Mixing a per-table `ingest_job` with an authored pipeline changes
        WHICH run of the first one counts, not the ALL rule: every upstream
        must be fresh, the first stale one ends the tick with its own
        reason, and one round of successes asks for one downstream run keyed
        on both."""
        ours = _ingest_run("ours")
        other = _ingest_run("other-new", end_time=250.0)
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    )
                ],
                "ingest_job": [other, ours],
                "authored__pl_up_a": [
                    _FakeDagsterRun(
                        job_name="authored__pl_up_a",
                        run_id="upstream-a-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=180.0,
                        end_time=210.0,
                    )
                ],
            },
            materializations={
                ("bronze", "orders"): [_materialized("ours", "orders")],
            },
        )
        sensor = self._sensor(["ingest_job", "pl-up-a"])

        fired = _tick(sensor, instance, _ingest_run("tick"))
        assert len(fired) == 1
        assert fired[0].run_key == "authored-deps:ours,upstream-a-1"

        # The same graph with the AUTHORED upstream behind: the tick ends on
        # that upstream, by name, and asks for nothing.
        instance._runs_by_job["authored__pl_up_a"] = [
            _FakeDagsterRun(
                job_name="authored__pl_up_a",
                run_id="upstream-a-1",
                status=DagsterRunStatus.SUCCESS,
                start_time=40.0,
                end_time=50.0,
            )
        ]
        behind = _tick(sensor, instance, _ingest_run("tick"))
        assert len(behind) == 1
        assert not isinstance(behind[0], RunRequest)
        assert "pl-up-a" in behind[0].skip_message

    def test_a_file_ingest_upstream_behaves_exactly_as_before(self) -> None:
        """BLD-4 / D3: nothing else changes. A `file_ingest_job` upstream on
        the same Bronze pipeline keeps DATA-19's rule -- its own run config
        says which table it loaded -- and the two per-table upstreams carry
        the same D4 sentence shape for a chain that names both."""
        ours = _file_ingest_run("ours", "orders")
        instance = _StubInstance(
            {
                "authored__pl_down_1": [
                    _FakeDagsterRun(
                        job_name="authored__pl_down_1",
                        run_id="downstream-1",
                        status=DagsterRunStatus.SUCCESS,
                        start_time=100.0,
                        end_time=120.0,
                    )
                ],
                "file_ingest_job": [ours],
                "ingest_job": [_ingest_run("ingest-ours")],
            },
            materializations={
                ("bronze", "orders"): [_materialized("ingest-ours", "orders")],
            },
        )
        out = _tick(
            self._sensor(["file_ingest_job", "ingest_job"]),
            instance,
            _file_ingest_run("tick", "orders"),
        )

        assert len(out) == 1
        assert isinstance(out[0], RunRequest)
        assert out[0].run_key == "authored-deps:ingest-ours,ours"


class TestIngestMaterializationKeyTest:
    """The assumptions D1 rests on, proven on the REAL dagster 1.13.20
    classes and a REAL event log instead of on a stub -- the shape of the
    bug that hid from `R3 plan 2a`'s unit tests and surfaced on
    `ops/g9`'s step 10.

    Every test here builds the job the same way `ingest_factory.ingest_job`
    does: a fan-out op whose `.map(...)` step logs one
    `AssetMaterialization(asset_key="bronze.<table>")` inside the run that
    loaded it."""

    @staticmethod
    def _job(fail_after: bool):
        """A job shaped like `ingest_job`: `run_ingest` yields one DynamicOutput
        per selected object and `ingest_source_object` (mapped) logs the
        Bronze materialization. With `fail_after`, a final step raises, so
        the run FAILS after the load's own event is already in the log."""
        from dagster import DynamicOut, DynamicOutput

        @op(out=DynamicOut())
        def run_ingest():
            for target in ("orders",):
                yield DynamicOutput(target, mapping_key=target)

        @op(name="ingest_source_object")
        def ingest_source_object(context, target: str):
            context.log_event(
                AssetMaterialization(asset_key=f"bronze.{target}", metadata={"rows": 3})
            )
            return target

        @op
        def boom(context, _targets):
            raise RuntimeError("a later step of the same run fails")

        @job(name="ingest_job")
        def _ingest_job() -> None:
            if fail_after:
                boom(run_ingest().map(ingest_source_object).collect())
            else:
                run_ingest().map(ingest_source_object).collect()

        return _ingest_job

    @contextmanager
    def _materialized_run(self, fail_after: bool) -> Iterator[Any]:
        """A REAL event log with one `ingest_job`-shaped run in it: yields
        `(instance, run_result)` and closes the instance when the test is
        done. No network and no ClickHouse -- this is the part of an ingest
        that produces the materialization and nothing else."""
        from dagster._core.test_utils import instance_for_test

        with instance_for_test() as instance:
            run = self._job(fail_after).execute_in_process(
                instance=instance, raise_on_error=False
            )
            yield instance, run

    def test_the_job_name_is_the_one_the_factory_filters_by(self) -> None:
        """The literal in `authored_factory` cannot drift from the job
        itself: `ingest_factory` has no `JOB_NAME` constant to import
        (`file_ingest.JOB_NAME` is the one that exists), so this pins the
        two names against each other instead."""
        from dispar_orchestrate.ingest_factory import ingest_job

        assert authored_factory.INGEST_JOB == ingest_job.name == "ingest_job"

    def test_an_asset_materialization_string_is_a_two_component_key(self) -> None:
        """`AssetMaterialization(asset_key="bronze.orders")` parses the
        string on the PERIOD, so the key is `['bronze', 'orders']`. The
        filter is built from that: the one-component
        `AssetKey(["bronze.orders"])` is a DIFFERENT key that matches
        nothing, which would leave every chain waiting forever."""
        key = AssetMaterialization(asset_key="bronze.orders").asset_key
        assert key.path == ["bronze", "orders"]
        assert key.to_string() == '["bronze", "orders"]'
        assert key.to_user_string() == "bronze/orders"
        # The filter's own key, and the two shapes it must not be.
        assert authored_factory._bronze_asset_key("orders").path == ["bronze", "orders"]
        assert AssetKey(["bronze.orders"]) != authored_factory._bronze_asset_key("orders")
        assert AssetKey(["bronze.orders"]).path == ["bronze.orders"]

    def test_the_real_event_log_holds_only_the_two_component_key(self) -> None:
        """The same fact from the storage side: a REAL run's materialization
        is stored under `['bronze', 'orders']`, so `fetch_materializations`
        answers the filter's key and NOT the one-component spelling. This is
        the assertion the plan's 5.2 asks for -- against the real class and
        a real event log, not a stub."""
        from dagster._core.test_utils import instance_for_test

        with self._materialized_run(fail_after=False) as (instance, run):
            assert run.success
            two_component = instance.fetch_materializations(
                authored_factory._bronze_asset_key("orders"), limit=5
            )
            assert [record.run_id for record in two_component.records] == [
                run.dagster_run.run_id
            ]
            assert two_component.records[0].asset_key.path == ["bronze", "orders"]
            one_component = instance.fetch_materializations(
                AssetKey(["bronze.orders"]), limit=5
            )
            assert one_component.records == []

    def test_a_materialization_survives_a_later_step_failure_but_is_not_an_anchor(self) -> None:
        """5.3, on the real classes: the run whose FIRST mapped step loaded
        the table and whose LATER step failed still has the materialization
        in the event log -- the log is append-only per step -- so the filter
        must read the RUN's status rather than trusting the event. The run is
        FAILURE, `get_run_records(statuses=[SUCCESS])` is empty, and the
        anchor for the chain is `None`."""
        with self._materialized_run(fail_after=True) as (instance, run):
            assert not run.success
            assert run.dagster_run.status == DagsterRunStatus.FAILURE
            records = instance.fetch_materializations(
                authored_factory._bronze_asset_key("orders"), limit=5
            )
            assert [record.run_id for record in records.records] == [run.dagster_run.run_id]
            assert (
                authored_factory._latest_record_for_ingest_table(
                    instance, "orders"
                )
                is None
            ), "a FAILED run's materialization must not anchor the chain"
            assert authored_factory._latest_record_for_ingest_table(instance, "") is None
            assert authored_factory._latest_record_for_ingest_table(instance, "invoices") is None

    def test_the_anchor_is_the_newest_success_run_that_materialized_the_table(self) -> None:
        """The whole filter, on a real event log: two SUCCESS runs (one of
        this table, one of `invoices`) and one FAILED run of this table. The
        anchor is the SUCCESS run of this table -- not the failed run's newer
        event, not the other table's newer run."""
        from dagster._core.test_utils import instance_for_test

        with instance_for_test() as instance:
            ours = self._job(fail_after=False).execute_in_process(instance=instance)
            assert ours.success
            broken = self._job(fail_after=True).execute_in_process(
                instance=instance, raise_on_error=False
            )
            assert not broken.success

            anchor = authored_factory._latest_record_for_ingest_table(instance, "orders")
            assert anchor is not None
            assert anchor.dagster_run.run_id == ours.dagster_run.run_id
            assert anchor.dagster_run.status == DagsterRunStatus.SUCCESS
            assert anchor.end_time is not None
            # And the run it picked is the one whose materialization the
            # event log holds for this table, not for the other one.
            materializations = instance.fetch_materializations(
                authored_factory._bronze_asset_key("orders"), limit=20
            )
            assert anchor.dagster_run.run_id in {
                record.run_id for record in materializations.records
            }

    def test_a_table_nobody_loaded_has_no_anchor(self) -> None:
        """A table with no materialization at all answers `None` from ONE
        instance call, so a chain waiting on an unloaded table costs the
        sensor nothing extra per tick."""
        with self._materialized_run(fail_after=False) as (instance, run):
            assert authored_factory._latest_record_for_ingest_table(instance, "") is None
            assert authored_factory._latest_record_for_ingest_table(instance, "invoices") is None


class TestFileIngestRunConfigReader:
    """The config path the filter reads, and its refusals."""

    def test_the_path_is_the_one_the_api_launches_with(self) -> None:
        assert authored_factory.FILE_INGEST_OP_CONFIG == (
            "ops",
            "ingest_uploaded_file",
            "config",
            "bronze_table_name",
        )

    def test_a_resolved_config_reads_the_table_it_loaded(self) -> None:
        run = _file_ingest_run("r", "stock_raw")
        assert authored_factory._run_config_value(
            run, authored_factory.FILE_INGEST_OP_CONFIG
        ) == "stock_raw"

    def test_a_missing_or_reshaped_config_answers_none(self) -> None:
        path = authored_factory.FILE_INGEST_OP_CONFIG
        none = _FakeDagsterRun("file_ingest_job", "r", DagsterRunStatus.SUCCESS, 1.0, 2.0)
        assert authored_factory._run_config_value(none, path) is None
        assert authored_factory._run_config_value(_file_ingest_run("r", None), path) is None
        # A config that is a list, one whose `ops` is not a mapping, and
        # a table name that is not a non-empty string all answer None
        # rather than raising: a run storage that changed shape must not
        # take a sensor tick down with it.
        for config in (
            [],
            {"ops": "none"},
            {"ops": {"ingest_uploaded_file": {"config": {"bronze_table_name": 7}}}},
            {"ops": {"ingest_uploaded_file": {"config": {"bronze_table_name": ""}}}},
        ):
            assert (
                authored_factory._run_config_value(
                    _FakeDagsterRun("file_ingest_job", "r", DagsterRunStatus.SUCCESS, 1.0, 2.0, config),
                    path,
                )
                is None
            ), config

    def test_an_empty_table_name_matches_no_record(self) -> None:
        records = [_FakeRunRecord(_file_ingest_run("r", "orders"))]
        assert authored_factory._latest_record_for_table(records, "orders") is records[0]
        assert authored_factory._latest_record_for_table(records, "") is None
        assert authored_factory._latest_record_for_table(records, "invoices") is None

    def test_a_real_runs_config_resolves_to_the_table_the_load_wrote(self) -> None:
        """The assumption D1 rests on, proven on a REAL run instead of a
        stub: `execute_in_process` against a temp instance writes a
        `file_ingest_job` run whose stored `run_config` -- the exact
        attribute `_run_config_value` reads, the exact value
        `context.instance.get_runs` hands the sensor -- is read back and
        resolves to the table that was launched.

        This is why no tag was added to the launch (plan D1): the config
        Dagster already stores carries the table. If a future Dagster
        resolves the config into a different shape, this test fails here,
        with the real payload, instead of the filter silently matching
        nothing in production.
        """
        from dagster._core.test_utils import instance_for_test

        from dispar_orchestrate.file_ingest import CONFIG_SCHEMA

        @op(name="ingest_uploaded_file", config_schema=CONFIG_SCHEMA)
        def _ingest_uploaded_file(context):  # noqa: ANN001, ANN202 - a Dagster op
            return {"bronze_table_name": context.op_config["bronze_table_name"]}

        @job(name="file_ingest_job")
        def _file_ingest_job() -> None:
            _ingest_uploaded_file()

        # `routes::uploads::run_config`'s own shape, verbatim.
        launch = {
            "ops": {
                "ingest_uploaded_file": {
                    "config": {
                        "upload_id": "up-1",
                        "storage_keys": ["uploads/t/up-1.csv"],
                        "format": "delimited",
                        "bronze_table_name": "stock_raw",
                        "load_mode": "replace",
                        "encoding": "utf-8",
                        "delimiter": ",",
                        "header_row": 0,
                        "columns": [],
                        "on_bad_value": "stop",
                    }
                }
            }
        }
        with instance_for_test() as instance:
            result = _file_ingest_job.execute_in_process(
                run_config=launch, instance=instance, raise_on_error=False
            )
            assert result.success
            runs = instance.get_runs(RunsFilter(job_name="file_ingest_job"), limit=1)
            assert len(runs) == 1
            stored = runs[0]
            assert (
                authored_factory._run_config_value(
                    stored, authored_factory.FILE_INGEST_OP_CONFIG
                )
                == "stock_raw"
            ), f"the stored run config resolved to {stored.run_config!r}"

            # And the record the SENSOR reads carries the times and the
            # run whose config this is: `get_run_records` is what the
            # sensor asks for, since Dagster 1.13 moved `start_time` /
            # `end_time` off the run and onto the record.
            records = instance.get_run_records(
                RunsFilter(job_name="file_ingest_job"), limit=1
            )
            assert len(records) == 1
            record = records[0]
            assert record.dagster_run.run_id == stored.run_id
            # THE finding this gate's step 10 caught, pinned so it cannot
            # come back: the run carries no times and the record does.
            assert not hasattr(stored, "start_time")
            assert not hasattr(stored, "end_time")
            assert isinstance(record.start_time, float)
            assert isinstance(record.end_time, float)
            assert record.start_time < record.end_time
            assert (
                authored_factory._latest_record_for_table([record], "stock_raw")
                is record
            )
            assert (
                authored_factory._latest_record_for_table([record], "invoices_raw")
                is None
            )


class TestLagSensor:
    """BLD-2 D3 + D6: target-lag sensor tests."""

    @job(name="authored__pl_lag_order")
    def _order_job() -> None:
        return None

    def test_returns_none_when_target_lag_is_absent_or_not_lag_kind(self) -> None:
        """Pipelines without targetLag or with kind='downstream' build no lag sensor."""
        assert authored_factory.build_authored_lag_sensor({"id": "pl-none", "targetLag": None}, self._order_job) is None
        assert authored_factory.build_authored_lag_sensor({"id": "pl-down", "targetLag": {"kind": "downstream"}}, self._order_job) is None

    def test_returns_none_when_pipeline_is_paused(self) -> None:
        """A paused pipeline gets no lag sensor."""
        pipeline = {
            "id": "pl-paused",
            "status": "paused",
            "targetLag": {"kind": "lag", "minutes": 15},
        }
        assert authored_factory.build_authored_lag_sensor(pipeline, self._order_job) is None

    def test_builds_running_sensor_with_30s_interval_when_ready(self) -> None:
        """A ready pipeline with kind='lag' gets a sensor named authored__<safe_id>_lag with 30s interval."""
        pipeline = {
            "id": "pl-lag-order",
            "status": "ready",
            "targetLag": {"kind": "lag", "minutes": 15},
        }
        sensor = authored_factory.build_authored_lag_sensor(pipeline, self._order_job)
        assert sensor is not None
        assert sensor.name == "authored__pl_lag_order_lag"
        assert sensor.minimum_interval_seconds == 30
        assert sensor.default_status == authored_factory.DefaultSensorStatus.RUNNING
        assert sensor.job.name == "authored__pl_lag_order"

    def _eval_sensor(self, response_mock: Any) -> list[Any]:
        from dagster import build_sensor_context
        pipeline = {
            "id": "pl-lag-order",
            "status": "ready",
            "targetLag": {"kind": "lag", "minutes": 15},
        }
        cfg = authored_factory.AuthoredPipelineConfig(
            api_url="http://fake-api:8080",
            run_token="test-token",
        )
        sensor = authored_factory.build_authored_lag_sensor(pipeline, self._order_job, cfg=cfg)
        assert sensor is not None
        with mock.patch.object(authored_factory.requests, "post", response_mock):
            return list(sensor(build_sensor_context()))

    def test_sensor_yields_one_run_request_on_run_decision(self) -> None:
        """Decision 'run' yields one RunRequest with the given run key and job."""
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {
            "decision": "run",
            "runKey": "lag-run-key-abc",
            "jobName": "authored__pl_lag_order",
        }
        requests_ = self._eval_sensor(mock.Mock(return_value=resp))
        assert len(requests_) == 1
        req = requests_[0]
        assert req.run_key == "lag-run-key-abc"
        assert req.job_name == "authored__pl_lag_order"

    def test_sensor_yields_nothing_on_wait_decision(self) -> None:
        """Decision 'wait' yields nothing."""
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {
            "decision": "wait",
            "reason": "inputs unchanged",
        }
        requests_ = self._eval_sensor(mock.Mock(return_value=resp))
        assert len(requests_) == 0

    def test_sensor_skips_on_4xx_without_raising(self) -> None:
        """4xx is logged and skipped, yields nothing, never raises."""
        resp = mock.Mock(status_code=404, text="Pipeline not found")
        requests_ = self._eval_sensor(mock.Mock(return_value=resp))
        assert len(requests_) == 0

    def test_sensor_skips_on_5xx_without_raising(self) -> None:
        """5xx is skipped for this tick, yields nothing, never raises."""
        resp = mock.Mock(status_code=503, text="Service Unavailable")
        requests_ = self._eval_sensor(mock.Mock(return_value=resp))
        assert len(requests_) == 0

    def test_sensor_skips_on_transport_error_without_raising(self) -> None:
        """Transport error is skipped for this tick, yields nothing, never raises."""
        requests_ = self._eval_sensor(mock.Mock(side_effect=requests.ConnectionError("unreachable")))
        assert len(requests_) == 0


    @job(name="authored__pl_u")
    def _upstream_job() -> None:
        return None

    def test_sensor_yields_run_request_with_different_allowed_job_name(self) -> None:
        """BLD-2 review R1 BLOCKER 2: sensor can launch upstream job when API directs."""
        from dagster import build_sensor_context
        pipeline = {
            "id": "pl-lag-order",
            "status": "ready",
            "targetLag": {"kind": "lag", "minutes": 15},
        }
        cfg = authored_factory.AuthoredPipelineConfig(
            api_url="http://fake-api:8080",
            run_token="test-token",
        )
        sensor = authored_factory.build_authored_lag_sensor(
            pipeline,
            self._order_job,
            allowed_jobs=[self._order_job, self._upstream_job],
            cfg=cfg,
        )
        assert sensor is not None
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {
            "decision": "run",
            "runKey": "lag-run-key-upstream",
            "jobName": "authored__pl_u",
        }
        with mock.patch.object(authored_factory.requests, "post", mock.Mock(return_value=resp)):
            requests_ = list(sensor(build_sensor_context()))
        assert len(requests_) == 1
        req = requests_[0]
        assert req.run_key == "lag-run-key-upstream"
        assert req.job_name == "authored__pl_u"

    def test_sensor_skips_when_job_name_not_allowed(self) -> None:
        """BLD-2 review R1 BLOCKER 2 / R2 SHOULD-FIX 7: unexpected jobName is logged and skipped."""
        from dagster import build_sensor_context
        pipeline = {
            "id": "pl-lag-order",
            "status": "ready",
            "targetLag": {"kind": "lag", "minutes": 15},
        }
        cfg = authored_factory.AuthoredPipelineConfig(
            api_url="http://fake-api:8080",
            run_token="test-token",
        )
        sensor = authored_factory.build_authored_lag_sensor(pipeline, self._order_job, cfg=cfg)
        assert sensor is not None
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {
            "decision": "run",
            "runKey": "lag-run-key-abc",
            "jobName": "authored__unknown_job",
        }
        ctx = build_sensor_context()
        with mock.patch.object(authored_factory.requests, "post", mock.Mock(return_value=resp)):
            with mock.patch.object(ctx.log, "warning") as mock_warn:
                requests_ = list(sensor(ctx))
        assert len(requests_) == 0
        mock_warn.assert_called_once()
        assert "unexpected jobName 'authored__unknown_job'" in mock_warn.call_args[0][0]

    def test_sensor_skips_when_run_key_missing(self) -> None:
        """BLD-2 review R1 BLOCKER 2 / R2 SHOULD-FIX 7: missing runKey is logged and skipped."""
        from dagster import build_sensor_context
        pipeline = {
            "id": "pl-lag-order",
            "status": "ready",
            "targetLag": {"kind": "lag", "minutes": 15},
        }
        cfg = authored_factory.AuthoredPipelineConfig(
            api_url="http://fake-api:8080",
            run_token="test-token",
        )
        sensor = authored_factory.build_authored_lag_sensor(pipeline, self._order_job, cfg=cfg)
        assert sensor is not None
        resp = mock.Mock(status_code=200)
        resp.json.return_value = {
            "decision": "run",
            "jobName": "authored__pl_lag_order",
        }
        ctx = build_sensor_context()
        with mock.patch.object(authored_factory.requests, "post", mock.Mock(return_value=resp)):
            with mock.patch.object(ctx.log, "warning") as mock_warn:
                requests_ = list(sensor(ctx))
        assert len(requests_) == 0
        mock_warn.assert_called_once()
        assert "without runKey" in mock_warn.call_args[0][0]

    def test_sensor_logs_warning_when_run_token_unset(self) -> None:
        """BLD-2 review R1 SHOULD-FIX 17 / R2 SHOULD-FIX 12: sensor logs warning once when run_token is unset."""
        from dagster import build_sensor_context
        pipeline = {
            "id": "pl-lag-order",
            "status": "ready",
            "targetLag": {"kind": "lag", "minutes": 15},
        }
        cfg = authored_factory.AuthoredPipelineConfig(
            api_url="http://fake-api:8080",
            run_token="",
        )
        sensor = authored_factory.build_authored_lag_sensor(pipeline, self._order_job, cfg=cfg)
        assert sensor is not None
        ctx = build_sensor_context()
        with mock.patch.object(ctx.log, "warning") as mock_warn:
            reqs = list(sensor(ctx))
        assert len(reqs) == 0
        mock_warn.assert_called_once()
        assert "DISPAR_RUN_TOKEN is unset" in mock_warn.call_args[0][0]

        # Second evaluation with the same cursor must NOT log again
        ctx2 = build_sensor_context(cursor=ctx.cursor)
        with mock.patch.object(ctx2.log, "warning") as mock_warn2:
            reqs2 = list(sensor(ctx2))
        assert len(reqs2) == 0
        mock_warn2.assert_not_called()

    def test_build_authored_lag_sensors_attaches_only_read_downstream_jobs(self) -> None:
        """BLD-2 review R2 SHOULD-FIX 7: lag sensor attaches only jobs of downstream pipelines it actually reads."""
        # P: reads silver.u1, tenant t1
        p_pipe = {
            "id": "pl-p",
            "status": "ready",
            "tenantId": "t1",
            "source": "silver.u1",
            "target": "gold.p",
            "targetLag": {"kind": "lag", "minutes": 10},
        }
        # U1: reads bronze.u2, produces silver.u1, downstream, tenant t1
        u1_pipe = {
            "id": "pl-u1",
            "status": "ready",
            "tenantId": "t1",
            "source": "bronze.u2",
            "target": "silver.u1",
            "targetLag": {"kind": "downstream"},
        }
        # U2: reads bronze.orders, produces bronze.u2, downstream, tenant t1 (transitive upstream of P)
        u2_pipe = {
            "id": "pl-u2",
            "status": "ready",
            "tenantId": "t1",
            "source": "bronze.orders",
            "target": "bronze.u2",
            "targetLag": {"kind": "downstream"},
        }
        # U3: downstream, tenant t1, but produces silver.unrelated (nobody reads it)
        u3_pipe = {
            "id": "pl-u3",
            "status": "ready",
            "tenantId": "t1",
            "source": "bronze.other",
            "target": "silver.unrelated",
            "targetLag": {"kind": "downstream"},
        }
        # U4: downstream, produces silver.u1, but tenant t2 (different tenant!)
        u4_pipe = {
            "id": "pl-u4",
            "status": "ready",
            "tenantId": "t2",
            "source": "bronze.orders",
            "target": "silver.u1",
            "targetLag": {"kind": "downstream"},
        }

        @job(name="authored__pl_p")
        def j_p() -> None:
            pass

        @job(name="authored__pl_u1")
        def j_u1() -> None:
            pass

        @job(name="authored__pl_u2")
        def j_u2() -> None:
            pass

        @job(name="authored__pl_u3")
        def j_u3() -> None:
            pass

        @job(name="authored__pl_u4")
        def j_u4() -> None:
            pass

        pipeline_jobs = [
            (p_pipe, j_p),
            (u1_pipe, j_u1),
            (u2_pipe, j_u2),
            (u3_pipe, j_u3),
            (u4_pipe, j_u4),
        ]
        sensors = authored_factory.build_authored_lag_sensors(pipeline_jobs)
        # Exactly one sensor built (for P; downstream pipelines get no lag sensor)
        assert len(sensors) == 1
        p_sensor = sensors[0]
        # Inspect jobs attached to P sensor
        attached_job_names = {j.name for j in p_sensor.jobs}
        assert "authored__pl_p" in attached_job_names
        assert "authored__pl_u1" in attached_job_names, "Direct downstream upstream must be attached"
        assert "authored__pl_u2" in attached_job_names, "Transitive downstream upstream must be attached"
        assert "authored__pl_u3" not in attached_job_names, "Unrelated downstream must NOT be attached"
        assert "authored__pl_u4" not in attached_job_names, "Different tenant downstream must NOT be attached"

    def test_an_unassigned_lag_pipeline_links_no_downstream_even_to_another_unassigned_pipeline(
        self,
    ) -> None:
        """BLD-2 review R4 SHOULD-FIX 2: an unassigned pipeline's runtime
        `tenantId` is None, and per R3 a NULL tenant matches no membership —
        so an unassigned lag pipeline has no reader edge at all, not even to
        another unassigned pipeline, and it must attach nothing beyond its
        own job. The R3 text claimed two unassigned pipelines match each
        other; this test pins that they cannot."""
        # Reading pipeline with no tenant at all.
        p_pipe = {
            "id": "pl-p-unassigned",
            "status": "ready",
            # No "tenantId" key: unassigned.
            "source": "silver.u1",
            "target": "gold.p",
            "targetLag": {"kind": "lag", "minutes": 10},
        }
        # Unassigned downstream pipeline producing silver.u1 — the tenant
        # filter would pair them under the old None == None rule.
        u_pipe = {
            "id": "pl-u-unassigned",
            "status": "ready",
            # No "tenantId" key: unassigned.
            "source": "bronze.x",
            "target": "silver.u1",
            "targetLag": {"kind": "downstream"},
        }

        @job(name="authored__pl_p_unassigned")
        def j_p() -> None:
            pass

        @job(name="authored__pl_u_unassigned")
        def j_u() -> None:
            pass

        sensors = authored_factory.build_authored_lag_sensors([(p_pipe, j_p), (u_pipe, j_u)])
        assert len(sensors) == 1
        attached_job_names = {j.name for j in sensors[0].jobs}
        assert attached_job_names == {"authored__pl_p_unassigned"}, (
            "an unassigned lag pipeline links nothing: only its own job is attached"
        )


class PerPipelineRetryPolicyTest(unittest.TestCase):
    """Plan 1c (R2, day-1): `_op_for_pipeline` reads each authored
    pipeline's `definition.maxRetries` (migration `0051_pipeline_max_retries.sql`)
    and passes it as `RetryPolicy.max_retries`, while keeping the
    store-wide `delay`/`backoff`/`jitter` from `op_metadata.DEFAULT_RETRY_POLICY`.
    Two boundaries to assert: `0` (the migration's floor; means
    "never retry") and `5` (the `CHECK` constraint's ceiling).

    The op the factory builds is itself a `dagster.OpDefinition`, and
    `OpDefinition.retry_policy` is the seam the test pins — the
    decorator argument is stored there verbatim and is read by
    `dagster`'s job executor at run time, so a wrong value here would
    surface as a real production-policy bug rather than a typing
    mistake."""

    def test_max_retries_zero_means_never_retry(self) -> None:
        # Plan 1c floor (`0..=5`): `maxRetries=0` is "never retry",
        # the row explicitly opts out of any self-healing. The op
        # builder still constructs a `RetryPolicy` rather than `None`,
        # so the rest of the policy vocabulary stays uniform.
        pipeline = _ready_pipeline(maxRetries=0)
        op = authored_factory._op_for_pipeline(pipeline)
        self.assertEqual(op.retry_policy.max_retries, 0)

    def test_max_retries_five_is_the_check_constraint_ceiling(self) -> None:
        # Plan 1c ceiling: `5` is the highest value the route layer
        # accepts (`!(0..=5).contains(...) -> 400`) and the column
        # CHECK permits. The op builder must surface it intact, with
        # the same `delay`/`backoff`/`jitter` as the default.
        pipeline = _ready_pipeline(maxRetries=5)
        op = authored_factory._op_for_pipeline(pipeline)
        self.assertEqual(op.retry_policy.max_retries, 5)
        base = RetryPolicy(
            max_retries=2, delay=30, backoff=Backoff.EXPONENTIAL, jitter=Jitter.PLUS_MINUS,
        )
        # The 1c override is ONLY the count, not delay/backoff/jitter —
        # a synchronised retry storm across all authored pipelines
        # cannot return because one of them widened the policy window.
        self.assertEqual(op.retry_policy.delay, base.delay)
        self.assertEqual(op.retry_policy.backoff, base.backoff)
        self.assertEqual(op.retry_policy.jitter, base.jitter)


class CodeLocationLoadTest(unittest.TestCase):
    """F1.1 BLOCKER: a pipeline with `dependsOn` MUST load under a real
    `Definitions(jobs, schedules, sensors)` — Dagster 1.13.20 refuses two
    `JobDefinition`s with the same name on a single location load, and
    building the job twice (once for `jobs`, once inside the sensor
    builder's `build_authored_job(pipeline)` call) used to do exactly
    that, taking the WHOLE code location down with it.

    The load call is the real-Dagster-load seam the plan requires: every
    other test in this file monkeypatches the HTTP fetch and inspects the
    factory's return values, which never crosses the `Definitions`
    validation step that fails in production. `load_all_definitions()`
    is what `dagster code-server start` runs at reload time — failing
    here is what was failing there."""

    def test_a_chain_pipeline_loads_the_code_location_without_a_duplicate_job(self) -> None:
        from dagster import Definitions

        upstream = {**_ready_pipeline(), "id": "pl-up-a", "dependsOn": []}
        downstream = {**_ready_pipeline(), "id": "pl-down-1", "dependsOn": ["pl-up-a"]}

        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [upstream, downstream]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            jobs, schedules, deps = authored_factory.build_authored_definitions(_cfg())

        sensors = authored_factory.build_authored_dependency_sensors(deps)
        defs = Definitions(jobs=jobs, schedules=schedules, sensors=sensors)
        # `load_all_definitions` walks the whole graph (jobs, schedules,
        # sensors, their dependencies) and is what raises
        # `DagsterInvalidDefinitionError: Duplicate job definition found`
        # when the sensor builder reuses a name that is already taken by
        # `authored_jobs`. A second `build_authored_job(pipeline)` here
        # is the mutation that re-introduces the BLOCKER (see the
        # `CodeLocationLoadMutationTest` below).
        repo = defs.get_repository_def()
        repo.load_all_definitions()  # must NOT raise


class CodeLocationLoadMutationTest(unittest.TestCase):
    """F1.1 mutation evidence. Building the downstream's job a second
    time inside the sensor builder is the exact regression the BLOCKER
    fix removed — this test re-introduces it and asserts that
    `load_all_definitions()` fails with the same `DagsterInvalidDefinitionError`
    the production code path raised. The assertion checks for the
    typename AND the offending job name so a future refactor that moves
    the failure somewhere else still requires a real review."""

    def test_building_the_job_twice_breaks_load_with_duplicate_definition_error(self) -> None:
        from dagster import Definitions, DagsterInvalidDefinitionError

        upstream = {**_ready_pipeline(), "id": "pl-up-a", "dependsOn": []}
        downstream = {**_ready_pipeline(), "id": "pl-down-1", "dependsOn": ["pl-up-a"]}

        resp = mock.Mock(status_code=200)
        resp.json.return_value = {"pipelines": [upstream, downstream]}
        resp.raise_for_status.return_value = None
        with mock.patch.object(authored_factory.requests, "get", return_value=resp):
            jobs, _schedules, _deps = authored_factory.build_authored_definitions(_cfg())

        # Mutation: build the downstream's job a SECOND time — the
        # regression the fix removed. The two `JobDefinition` objects
        # share the name `authored__pl_down_1`; `Definitions` refuses.
        duplicate_job = authored_factory.build_authored_job(downstream)
        sensors = authored_factory.build_authored_dependency_sensor(
            downstream, duplicate_job,
        )

        defs = Definitions(jobs=jobs, schedules=[], sensors=[sensors] if sensors else [])
        with self.assertRaises(DagsterInvalidDefinitionError) as ctx:
            defs.get_repository_def().load_all_definitions()
        message = str(ctx.exception)
        self.assertIn("Duplicate", message)
        self.assertIn("authored__pl_down_1", message)
