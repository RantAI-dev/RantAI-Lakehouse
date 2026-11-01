"""Unit tests for the catalog search benchmark (`catalog_search_bench.py`).

No network: the HTTP layer is a fake session, and ClickHouse is never
touched (only the `seed` path writes, and it is not exercised here).

What is checked is the two things a bug in would quietly ruin: the command
line (what `seed` and `run` are told to do) and the JSON `run` prints (what
`docs/plans/DATA-11-RESULT.md` is written from, unedited). The numbers
themselves are the product's business, not this file's -- the bench measures,
it states no verdict.

Runs under `python3 -m pytest ops/bench/test_catalog_search_bench.py` and
under `python3 -m unittest discover -s ops/bench -p 'test_*.py'`.
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import catalog_search_bench as m  # noqa: E402


class FakeResponse:
    """The parts of `requests.Response` the bench reads."""

    def __init__(self, status_code: int = 200, body: object = None, content: bytes = b"{}") -> None:
        self.status_code = status_code
        self.ok = status_code < 400
        self.headers = {"content-type": "application/json"}
        self.content = content
        self.text = content.decode(errors="replace")
        self._body = body

    def json(self) -> object:
        if self._body is None:
            raise ValueError("no JSON body")
        return self._body

    def raise_for_status(self) -> None:
        if self.status_code >= 400:
            raise m.requests.HTTPError(f"HTTP {self.status_code}")


class FakeSession:
    """The `requests.Session` the bench builds, faked: every answer is
    canned and every call is recorded, so a test can say what the bench
    asked for."""

    def __init__(self, login_body: dict | None = None, search_body: bytes = b'{"assets":[]}') -> None:
        self.headers: dict[str, str] = {}
        self.calls: list[tuple] = []
        self.login_body = {"mustChangePassword": False} if login_body is None else login_body
        self.search_body = search_body

    def post(self, url: str, json: object = None, timeout: int | None = None, **kwargs: object) -> FakeResponse:
        self.calls.append(("POST", url, json))
        if url.endswith("/api/auth/login"):
            return FakeResponse(200, self.login_body)
        if url.endswith("/api/auth/change-password"):
            return FakeResponse(204)
        raise AssertionError(f"unexpected POST: {url}")

    def get(self, url: str, params: dict | None = None, timeout: int | None = None, **kwargs: object) -> FakeResponse:
        self.calls.append(("GET", url, params))
        return FakeResponse(200, {"assets": []}, content=self.search_body)


def settings(**overrides: str) -> m.Settings:
    """The settings of a run against a stack that answers."""
    base = {
        "throwaway": "true",
        "clickhouse_url": "http://ch.test",
        "clickhouse_user": "default",
        "clickhouse_password": "",
        "api_url": "http://api.test",
        "email": "bench@example.com",
        "password": "bench-password-not-real-123",
        "tenant_id": "",
    }
    base.update(overrides)
    return m.Settings(**base)  # type: ignore[arg-type]


class ArgumentParsingTests(unittest.TestCase):
    """The command line: what the workflow's `seed` and `run` are told."""

    def test_seed_takes_its_size_from_the_command_line(self) -> None:
        args = m.build_parser().parse_args(["seed", "--datasets", "3", "--columns", "7"])
        self.assertEqual(args.command, "seed")
        self.assertEqual(args.datasets, 3)
        self.assertEqual(args.columns, 7)

    def test_run_defaults_to_the_size_the_feature_page_asks_for(self) -> None:
        """The target is a catalog of 10,000 tables of 20 columns, and 50
        searches per route (`docs/core/features/catalog-search.md`)."""
        seed = m.build_parser().parse_args(["seed"])
        self.assertEqual(seed.datasets, 10_000)
        self.assertEqual(seed.columns, 20)
        run = m.build_parser().parse_args(["run"])
        self.assertEqual(run.rounds, 50)
        self.assertEqual(run.datasets, 10_000)
        self.assertEqual(run.cold_wait, 35.0)

    def test_run_takes_its_own_size_from_the_command_line(self) -> None:
        args = m.build_parser().parse_args(["run", "--rounds", "4", "--datasets", "2", "--cold-wait", "0"])
        self.assertEqual(args.command, "run")
        self.assertEqual(args.rounds, 4)
        self.assertEqual(args.datasets, 2)
        self.assertEqual(args.cold_wait, 0.0)

    def test_a_command_is_required(self) -> None:
        with self.assertRaises(SystemExit):
            m.build_parser().parse_args([])


class SettingsTests(unittest.TestCase):
    """The environment contract: the names the workflow sets."""

    def test_the_settings_read_the_variables_the_workflow_sets(self) -> None:
        env = {
            "BENCH_THROWAWAY_CATALOG": "true",
            "BENCH_CLICKHOUSE_URL": "http://localhost:8123/",
            "CLICKHOUSE_USER": "default",
            "CLICKHOUSE_PASSWORD": "",
            "LAKEHOUSE_API_URL": "http://localhost:8080/",
            "AUTH_BOOTSTRAP_EMAIL": "ci@example.com",
            "AUTH_BOOTSTRAP_PASSWORD": "ci-password-not-real-123",
        }
        with mock.patch.dict(m.os.environ, env, clear=True):
            got = m.Settings.from_env()
        self.assertTrue(got.throwaway)
        self.assertEqual(got.clickhouse_url, "http://localhost:8123")
        self.assertEqual(got.api_url, "http://localhost:8080")
        self.assertEqual(got.email, "ci@example.com")
        self.assertEqual(got.password, "ci-password-not-real-123")
        self.assertEqual(got.tenant_id, "")

    def test_only_the_literal_true_opens_the_throwaway_promise(self) -> None:
        with mock.patch.dict(m.os.environ, {"BENCH_THROWAWAY_CATALOG": "yes"}, clear=True):
            self.assertFalse(m.Settings.from_env().throwaway)


class RegistryOwnerTests(unittest.TestCase):
    """The registry schema's one owner, loaded by path."""

    def test_the_owner_loads_and_carries_the_schema_the_bench_writes_into(self) -> None:
        """The owner file is a `dataclass` module: it has to be in
        `sys.modules` before it is executed, or every dataclass in it
        fails at import time."""
        owner = m._registry_owner()
        self.assertEqual(
            [schema.table_name for schema in owner.EXPECTED_SCHEMAS],
            [
                "bronze_meta.dataset_catalog",
                "bronze_meta.dataset_sync",
                "bronze_meta.dataset_column",
                "bronze_meta_sec.dataset_catalog",
                "bronze_meta_sec.dataset_sync",
                "bronze_meta_sec.dataset_column",
            ],
        )
        self.assertTrue(hasattr(owner, "_assert_or_create_all"))
        self.assertTrue(issubclass(owner.SchemaDriftError, RuntimeError))

    def test_a_missing_clickhouse_url_is_a_failure_before_anything_is_written(self) -> None:
        with self.assertRaises(m.BenchFailure):
            m.ensure_registry_schema(settings(clickhouse_url=""))

    def test_a_schema_that_drifted_from_its_owner_is_a_refusal(self) -> None:
        class Drifted:
            class SchemaDriftError(RuntimeError):
                pass

            class ClickHouseTarget:
                def __init__(self, url: str, user: str, password: str) -> None:
                    self.url = url

            EXPECTED_SCHEMAS = ()

            @staticmethod
            def _assert_or_create_all(target: object, schemas: object) -> None:
                raise Drifted.SchemaDriftError("engine='MergeTree'")

        with mock.patch.object(m, "_registry_owner", lambda: Drifted):
            with self.assertRaises(m.BenchFailure):
                m.ensure_registry_schema(settings())


class RunTests(unittest.TestCase):
    """The JSON `run` prints: the shape the result document is written from."""

    def test_the_run_prints_the_json_shape_the_result_document_reads(self) -> None:
        fake = FakeSession()
        with mock.patch.object(m.requests, "Session", lambda: fake):
            result = m.run(settings(), rounds=4, datasets=2, cold_wait=0.0)
        self.assertEqual(
            set(result),
            {"rounds", "datasetsAssumed", "coldWaitSeconds", "catalog", "catalogQuery"},
        )
        self.assertEqual(result["rounds"], 4)
        self.assertEqual(result["datasetsAssumed"], 2)
        self.assertEqual(result["coldWaitSeconds"], 0.0)
        for route in ("catalog", "catalogQuery"):
            self.assertEqual(
                set(result[route]),
                {"firstMs", "searches", "p50Ms", "p95Ms", "maxMs"},
            )
            # One search is reported on its own (it builds the search copy);
            # the rest are summarised.
            self.assertEqual(result[route]["searches"], 3)
        # The bench prints exactly this object, so it must be serialisable.
        self.assertEqual(json.loads(json.dumps(result)), result)

    def test_the_run_searches_both_routes(self) -> None:
        fake = FakeSession()
        with mock.patch.object(m.requests, "Session", lambda: fake):
            m.run(settings(), rounds=2, datasets=2, cold_wait=0.0)
        searched = [call[1] for call in fake.calls if call[0] == "GET"]
        # Two rounds on each of the two routes.
        self.assertEqual(
            searched,
            [
                "http://api.test/api/catalog",
                "http://api.test/api/catalog",
                "http://api.test/api/catalog/query",
                "http://api.test/api/catalog/query",
            ],
        )

    def test_a_refused_search_is_a_failure_and_not_a_number(self) -> None:
        """A refusal answers `supported: false`; the bench must not read
        that as a fast search."""
        fake = FakeSession(search_body=b'{"assets":[],"supported":false}')
        with mock.patch.object(m.requests, "Session", lambda: fake):
            with self.assertRaises(m.BenchFailure):
                m.run(settings(), rounds=2, datasets=2, cold_wait=0.0)

    def test_a_login_that_must_rotate_its_password_rotates_and_signs_in_again(self) -> None:
        """The bootstrap admin is seeded with a pending rotation, and the API
        refuses every other route until it is done."""
        fake = FakeSession(login_body={"mustChangePassword": True})
        with mock.patch.object(m.requests, "Session", lambda: fake):
            m.run(settings(), rounds=2, datasets=2, cold_wait=0.0)
        posted = [(call[1], call[2]) for call in fake.calls if call[0] == "POST"]
        self.assertEqual(
            posted,
            [
                ("http://api.test/api/auth/login", {"email": "bench@example.com", "password": "bench-password-not-real-123"}),
                ("http://api.test/api/auth/change-password", {"newPassword": "bench-password-not-real-123"}),
                ("http://api.test/api/auth/login", {"email": "bench@example.com", "password": "bench-password-not-real-123"}),
            ],
        )

    def test_a_login_that_needs_no_rotation_signs_in_once(self) -> None:
        fake = FakeSession()
        with mock.patch.object(m.requests, "Session", lambda: fake):
            m.run(settings(), rounds=2, datasets=2, cold_wait=0.0)
        posted = [call[1] for call in fake.calls if call[0] == "POST"]
        self.assertEqual(posted, ["http://api.test/api/auth/login"])

    def test_a_failed_login_is_a_failure(self) -> None:
        class RefusingSession(FakeSession):
            def post(self, url: str, json: object = None, timeout: int | None = None, **kwargs: object) -> FakeResponse:
                self.calls.append(("POST", url, json))
                return FakeResponse(401, {"error": "no"})

        with mock.patch.object(m.requests, "Session", lambda: RefusingSession()):
            with self.assertRaises(m.BenchFailure):
                m.run(settings(), rounds=2, datasets=2, cold_wait=0.0)

    def test_the_summary_of_the_rounds_is_the_shape_the_result_document_reads(self) -> None:
        summary = m._summary(12.34, [1.0, 2.0, 3.0])
        self.assertEqual(set(summary), {"firstMs", "searches", "p50Ms", "p95Ms", "maxMs"})
        self.assertEqual(summary["firstMs"], 12.3)
        self.assertEqual(summary["searches"], 3)
        self.assertEqual(summary["p50Ms"], 2.0)
        self.assertEqual(summary["maxMs"], 3.0)


if __name__ == "__main__":
    unittest.main()
