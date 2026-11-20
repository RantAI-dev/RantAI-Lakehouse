"""`DATA-6` T4: the latest-copy op, its run-config rules, its record and
its job — no network, fakes only, the same shape as
`test_gold_export.py`'s `GoldTrimOpTests`.

What is proved here, in the plan's numbering:

1-2. the copy is written in ONE commit — one `transaction()` entered,
    one `overwrite` (or one `create_table_transaction` + `append`), one
    commit, never a separate `delete`/`append` on the table itself;
3-5. fail closed both ways around the marker (D7) and the schema (D8);
6.   an out-of-order run is `superseded`, never backwards (D9);
7.   a copy that is not whole is `copy_incomplete` (D9);
8-9. the two write-failure reasons, with no exception text in the row;
10-12. the publishing switch at run time (D10), fail closed;
13.  the run-config rules (section 6's `invalid_request` row);
14-15. cleanup (D11) is recorded, never fatal to a replaced copy;
16-17. the constants and the job's serial tag / run limit.
"""

from __future__ import annotations

import contextlib
import datetime
import unittest
from dataclasses import dataclass, field
from types import SimpleNamespace
from unittest.mock import MagicMock, patch

import pytest
import pyarrow as pa
from dagster import Failure, build_op_context

from dispar_orchestrate import gold_latest_copy
from dispar_orchestrate.gold_export import GoldExportConfig
from dispar_orchestrate.ingest_factory import SERIAL_TAG

MART = "sales_mart"
TABLE = "sales_mart"
LATEST_TABLE = "sales_mart_latest"
COPY_MICROS = 1_795_168_800_123_456


def _cfg(marts: list[str] | None = None, ch: object = None) -> GoldExportConfig:
    return GoldExportConfig(
        ch=ch,  # type: ignore[arg-type]
        api_url="http://lakehouse-api.invalid:8080",
        marts=marts if marts is not None else [MART],
        run_token="a-real-token",
        rustfs_access_key="test-access-key",
        rustfs_secret_key="test-secret-key",
        gold_orphan_min_age_seconds="3600",
    )


def _req() -> gold_latest_copy.LatestCopyRequest:
    return gold_latest_copy.LatestCopyRequest(
        mart=MART,
        table=TABLE,
        latest_table=LATEST_TABLE,
        copy_exported_at_micros=COPY_MICROS,
        rows=5,
    )


@dataclass(frozen=True)
class _Field:
    name: str
    field_type: str
    required: bool


@dataclass(frozen=True)
class _Schema:
    fields: tuple[_Field, ...]


_MAIN_SCHEMA = _Schema(
    (
        _Field("_exported_at", "TimestampType()", True),
        _Field("region", "StringType()", False),
    )
)


def _arrow_frame(micros: int, rows: int) -> pa.Table:
    """The frame `main.scan(row_filter=...)` returns: `rows` copies of one
    `_exported_at` stamp (microseconds), exactly as the publish wrote it."""
    return pa.Table.from_arrays(
        [pa.array([micros] * rows, type=pa.timestamp("us"))],
        names=["_exported_at"],
    )


@dataclass
class _Snapshot:
    snapshot_id: int = 42


@dataclass
class _Transaction:
    """The staged write, as PyIceberg's real `Transaction` behaves: the
    deletes and appends are STAGED here and committed once, on exit."""

    table: object
    events: list[tuple] = field(default_factory=list)

    def __enter__(self) -> "_Transaction":
        self.events.append(("enter",))
        return self

    def __exit__(self, *exc: object) -> bool:
        self.events.append(("commit",))
        return False

    def overwrite(self, df: object) -> None:
        self.events.append(("overwrite", df))

    def append(self, df: object) -> None:
        self.events.append(("append", df))


@dataclass
class _LatestTable:
    """A latest table. Deliberately NOT a MagicMock: an unexpected
    `delete(...)`/`append(...)` on the table itself raises here, so the
    one-commit tests fail loudly if the op ever writes that way."""

    properties: dict[str, str]
    schema_: _Schema
    held_micros: int | None = None
    events: list[tuple] = field(default_factory=list)
    tx: _Transaction | None = None

    def schema(self) -> _Schema:
        return self.schema_

    def scan(self, selected_fields=None, limit=None, row_filter=None) -> object:
        rows = 0 if self.held_micros is None else 1
        micros = self.held_micros or 0
        return MagicMock(to_arrow=MagicMock(return_value=_arrow_frame(micros, rows)))

    def transaction(self) -> _Transaction:
        self.tx = _Transaction(self)
        return self.tx

    def current_snapshot(self) -> _Snapshot:
        return _Snapshot()


@dataclass
class _MainTable:
    schema_: _Schema
    copy_frame: pa.Table

    def schema(self) -> _Schema:
        return self.schema_

    def scan(self, row_filter=None) -> object:
        self.last_filter = row_filter
        return MagicMock(to_arrow=MagicMock(return_value=self.copy_frame))


class _Catalog:
    def __init__(self, main: _MainTable, latest: _LatestTable | None) -> None:
        self.main = main
        self.latest = latest
        self.created: tuple | None = None

    def load_table(self, ident: str) -> object:
        if ident == f"gold.{TABLE}":
            return self.main
        if ident == f"gold.{LATEST_TABLE}":
            if self.latest is None:
                from pyiceberg.exceptions import NoSuchTableError

                raise NoSuchTableError(f"gold.{LATEST_TABLE}")
            return self.latest
        raise AssertionError(f"unexpected table load: {ident}")

    def create_table_transaction(self, ident: str, schema: object, properties: dict) -> _Transaction:
        self.created = (ident, schema, properties)
        # The commit creates the table: a reload after this point finds it,
        # exactly as the real catalog would.
        self.latest = _LatestTable(properties=dict(properties), schema_=schema)  # type: ignore[arg-type]
        tx = _Transaction(self)
        return tx


@contextlib.contextmanager
def _common_patches(catalog: _Catalog, recorded: list[dict]):
    """The fixture every refresh test needs: the publication list says
    the mart is on, the catalog is the fake, the record is captured,
    maintenance is not paused, and the trim's two cleanup helpers (T3)
    are fakes. Tests that assert on the cleanup use `mocks.expire` /
    `mocks.orphan`; tests that need a cleanup failure patch the name
    again inside this context."""
    with patch(
        "dispar_orchestrate.gold_latest_copy._enabled_publication_marts",
        return_value=[MART],
    ), patch(
        "dispar_orchestrate.gold_latest_copy.load_gold_catalog", return_value=catalog
    ), patch(
        "dispar_orchestrate.gold_latest_copy.record_gold_latest_copy_run",
        side_effect=lambda target, **kw: recorded.append(kw),
    ), patch(
        "dispar_orchestrate.gold_latest_copy.maintenance_paused", return_value=False
    ) as paused, patch(
        "dispar_orchestrate.gold_latest_copy.expire_all_but_current_snapshot",
        return_value=1,
    ) as expire, patch(
        "dispar_orchestrate.gold_latest_copy.remove_gold_orphan_files",
        return_value=1,
    ) as orphan:
        yield SimpleNamespace(paused=paused, expire=expire, orphan=orphan)


class LatestCopyRefreshTests(unittest.TestCase):
    """The step order, the one-commit writes and the closed reason set."""

    def _refresh(self, cfg, req, catalog, recorded, *, extra=()):
        with _common_patches(catalog, recorded):
            for e in extra:
                e.start()
            try:
                return gold_latest_copy.refresh_latest_copy(cfg, req)
            finally:
                for e in extra:
                    e.stop()

    def test_an_existing_latest_table_is_replaced_by_one_overwrite_in_one_transaction(self) -> None:
        latest = _LatestTable(
            properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE},
            schema_=_MAIN_SCHEMA,
            held_micros=COPY_MICROS - 2_000_000,
        )
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, latest)
        recorded: list[dict] = []
        cfg = _cfg()

        result = self._refresh(cfg, _req(), catalog, recorded)

        self.assertEqual(result, {"status": "success", "cleanup": None})
        self.assertIsNotNone(latest.tx)
        events = latest.tx.events
        self.assertEqual(
            [e[0] for e in events], ["enter", "overwrite", "commit"],
            f"one transaction, one overwrite, one commit: {events}",
        )
        self.assertIs(events[1][1], main.copy_frame, "the read frame is what is overwritten")
        self.assertEqual(
            catalog.created, None, "an existing table is never created again"
        )
        self.assertEqual(len(recorded), 1)
        row = recorded[0]
        self.assertEqual(row["status"], "success")
        self.assertIsNone(row["reason"])
        self.assertEqual(row["copy_exported_at_micros"], COPY_MICROS)
        self.assertEqual(row["rows"], 5)
        self.assertEqual(row["snapshot_id"], 42)
        self.assertIsNone(row["cleanup"])

    def test_a_first_update_creates_the_table_with_its_rows_in_one_commit(self) -> None:
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, None)
        recorded: list[dict] = []

        result = self._refresh(_cfg(), _req(), catalog, recorded)

        self.assertEqual(result, {"status": "success", "cleanup": None})
        ident, schema, properties = catalog.created
        self.assertEqual(ident, f"gold.{LATEST_TABLE}")
        self.assertIs(schema, _MAIN_SCHEMA, "the latest table gets the main table's schema")
        self.assertEqual(
            properties,
            {
                gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE,
                "format-version": "2",
            },
        )

    def test_a_table_without_the_marker_is_not_touched_and_records_name_taken(self) -> None:
        latest = _LatestTable(properties={}, schema_=_MAIN_SCHEMA, held_micros=None)
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, latest)
        recorded: list[dict] = []

        result = self._refresh(_cfg(), _req(), catalog, recorded)

        self.assertEqual(result, {"status": "failed", "reason": "name_taken"})
        self.assertIsNone(latest.tx, "nothing was written")
        self.assertEqual(len(recorded), 1)
        self.assertEqual(recorded[0]["status"], "failed")
        self.assertEqual(recorded[0]["reason"], "name_taken")

    def test_a_marker_naming_another_table_records_name_taken(self) -> None:
        latest = _LatestTable(
            properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: "another_mart"},
            schema_=_MAIN_SCHEMA,
            held_micros=None,
        )
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, latest)
        recorded: list[dict] = []

        result = self._refresh(_cfg(), _req(), catalog, recorded)

        self.assertEqual(result, {"status": "failed", "reason": "name_taken"})
        self.assertIsNone(latest.tx, "nothing was written")
        self.assertEqual(recorded[0]["reason"], "name_taken")

    def test_different_columns_record_schema_incompatible_and_write_nothing(self) -> None:
        drifted = _Schema(
            (
                _Field("_exported_at", "TimestampType()", True),
                _Field("region", "StringType()", True),  # required drifted
            )
        )
        latest = _LatestTable(
            properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE},
            schema_=drifted,
            held_micros=None,
        )
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, latest)
        recorded: list[dict] = []

        result = self._refresh(_cfg(), _req(), catalog, recorded)

        self.assertEqual(result, {"status": "failed", "reason": "schema_incompatible"})
        self.assertIsNone(latest.tx, "nothing was written")
        self.assertEqual(recorded[0]["reason"], "schema_incompatible")

    def test_an_older_or_equal_request_is_superseded_and_writes_nothing(self) -> None:
        for held in (COPY_MICROS, COPY_MICROS + 1):
            with self.subTest(held=held):
                latest = _LatestTable(
                    properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE},
                    schema_=_MAIN_SCHEMA,
                    held_micros=held,
                )
                main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
                catalog = _Catalog(main, latest)
                recorded: list[dict] = []

                result = self._refresh(_cfg(), _req(), catalog, recorded)

                self.assertEqual(
                    result, {"status": "superseded", "reason": "newer_copy_present"}
                )
                self.assertIsNone(latest.tx, "the table never moves backwards")
                self.assertEqual(recorded[0]["status"], "superseded")
                self.assertEqual(recorded[0]["reason"], "newer_copy_present")

    def test_a_row_count_that_differs_from_the_publish_records_copy_incomplete(self) -> None:
        for rows in (0, 4):
            with self.subTest(rows=rows):
                latest = _LatestTable(
                    properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE},
                    schema_=_MAIN_SCHEMA,
                    held_micros=COPY_MICROS - 10_000_000,
                )
                main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, rows))
                catalog = _Catalog(main, latest)
                recorded: list[dict] = []

                result = self._refresh(_cfg(), _req(), catalog, recorded)

                self.assertEqual(result, {"status": "failed", "reason": "copy_incomplete"})
                self.assertIsNone(latest.tx, "an incomplete copy is never written")

    def test_the_row_filter_uses_the_publish_stamp_as_epoch_micros(self) -> None:
        from pyiceberg.expressions import EqualTo

        latest = _LatestTable(
            properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE},
            schema_=_MAIN_SCHEMA,
            held_micros=COPY_MICROS - 10_000_000,
        )
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, latest)
        recorded: list[dict] = []

        self._refresh(_cfg(), _req(), catalog, recorded)

        term = main.last_filter.term.name
        self.assertEqual(term, "_exported_at")
        self.assertEqual(main.last_filter.literal.value, COPY_MICROS)
        self.assertIsInstance(main.last_filter, EqualTo)

    def test_any_other_write_error_records_write_failed_and_no_exception_text(self) -> None:
        marker = RuntimeError("S3 PUT failed for object key secret-value-xyz")

        def explode_tx(latest: _LatestTable) -> None:
            original = latest.transaction

            def tx() -> object:
                t = original()
                t.overwrite = lambda df: (_ for _ in ()).throw(marker)  # type: ignore[misc]
                return t

            latest.transaction = tx  # type: ignore[method-assign]

        latest = _LatestTable(
            properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE},
            schema_=_MAIN_SCHEMA,
            held_micros=COPY_MICROS - 10_000_000,
        )
        explode_tx(latest)
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, latest)

        # The record is driven for real against a captured INSERT, so the
        # SQL itself is what is asserted — the row's reason is a fixed
        # string and the exception's own text never reaches it.
        inserts: list[str] = []
        with patch(
            "dispar_orchestrate.gold_latest_copy._enabled_publication_marts",
            return_value=[MART],
        ), patch(
            "dispar_orchestrate.gold_latest_copy.load_gold_catalog", return_value=catalog
        ), patch(
            "dispar_orchestrate.gold_latest_copy._ch_exec",
            side_effect=lambda target, sql: inserts.append(sql),
        ):
            result = gold_latest_copy.refresh_latest_copy(_cfg(ch=object()), _req())

        self.assertEqual(result, {"status": "failed", "reason": "write_failed"})
        recorded_insert = inserts[-1]
        self.assertIn("'write_failed'", recorded_insert)
        self.assertNotIn("secret-value-xyz", recorded_insert)
        self.assertNotIn("S3 PUT failed", recorded_insert)

    def test_publishing_switched_off_skips_and_writes_nothing(self) -> None:
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, None)
        recorded: list[dict] = []
        cfg = _cfg(marts=[])

        with patch(
            "dispar_orchestrate.gold_latest_copy._enabled_publication_marts",
            return_value=[],
        ), patch(
            "dispar_orchestrate.gold_latest_copy.record_gold_latest_copy_run",
            side_effect=lambda target, **kw: recorded.append(kw),
        ), patch(
            "dispar_orchestrate.gold_latest_copy.load_gold_catalog"
        ) as load_catalog:
            result = gold_latest_copy.refresh_latest_copy(cfg, _req())

        self.assertEqual(result, {"status": "skipped", "reason": "publishing_off"})
        load_catalog.assert_not_called()
        self.assertEqual(len(recorded), 1)
        self.assertEqual(recorded[0]["status"], "skipped")
        self.assertEqual(recorded[0]["reason"], "publishing_off")

    def test_a_mart_in_the_env_override_is_updated_even_when_not_switched_on(self) -> None:
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, None)
        recorded: list[dict] = []
        cfg = _cfg(marts=[MART])

        with patch(
            "dispar_orchestrate.gold_latest_copy._enabled_publication_marts",
            return_value=[],
        ), patch(
            "dispar_orchestrate.gold_latest_copy.load_gold_catalog", return_value=catalog
        ), patch(
            "dispar_orchestrate.gold_latest_copy.record_gold_latest_copy_run",
            side_effect=lambda target, **kw: recorded.append(kw),
        ), patch(
            "dispar_orchestrate.gold_latest_copy.maintenance_paused", return_value=True
        ):
            result = gold_latest_copy.refresh_latest_copy(cfg, _req())

        self.assertEqual(result["status"], "success")
        self.assertIsNotNone(catalog.created, "the override publishes anyway")

    def test_an_unreadable_publication_list_fails_closed(self) -> None:
        import requests as _requests

        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, None)
        recorded: list[dict] = []

        with patch(
            "dispar_orchestrate.gold_latest_copy._enabled_publication_marts",
            side_effect=_requests.ConnectionError("the API is down"),
        ), patch(
            "dispar_orchestrate.gold_latest_copy.load_gold_catalog", return_value=catalog
        ), patch(
            "dispar_orchestrate.gold_latest_copy.record_gold_latest_copy_run",
            side_effect=lambda target, **kw: recorded.append(kw),
        ):
            result = gold_latest_copy.refresh_latest_copy(_cfg(), _req())

        self.assertEqual(
            result, {"status": "failed", "reason": "publication_state_unavailable"}
        )
        self.assertEqual(len(recorded), 1)
        self.assertEqual(recorded[0]["reason"], "publication_state_unavailable")

    def test_paused_maintenance_keeps_the_new_copy_and_records_the_pause_as_cleanup(self) -> None:
        latest = _LatestTable(
            properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE},
            schema_=_MAIN_SCHEMA,
            held_micros=COPY_MICROS - 10_000_000,
        )
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, latest)
        recorded: list[dict] = []

        with patch(
            "dispar_orchestrate.gold_latest_copy._enabled_publication_marts",
            return_value=[MART],
        ), patch(
            "dispar_orchestrate.gold_latest_copy.load_gold_catalog", return_value=catalog
        ), patch(
            "dispar_orchestrate.gold_latest_copy.record_gold_latest_copy_run",
            side_effect=lambda target, **kw: recorded.append(kw),
        ), patch(
            "dispar_orchestrate.gold_latest_copy.maintenance_paused", return_value=True
        ) as paused, patch(
            "dispar_orchestrate.gold_latest_copy.expire_all_but_current_snapshot"
        ) as expire, patch(
            "dispar_orchestrate.gold_latest_copy.remove_gold_orphan_files"
        ) as orphan:
            result = gold_latest_copy.refresh_latest_copy(_cfg(), _req())

        self.assertEqual(result, {"status": "success", "cleanup": "maintenance_paused"})
        paused.assert_called_once()
        expire.assert_not_called()
        orphan.assert_not_called()
        self.assertEqual(recorded[0]["cleanup"], "maintenance_paused")

    def test_a_failed_snapshot_expiry_is_recorded_as_cleanup_and_the_status_stays_success(self) -> None:
        latest = _LatestTable(
            properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE},
            schema_=_MAIN_SCHEMA,
            held_micros=COPY_MICROS - 10_000_000,
        )
        main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
        catalog = _Catalog(main, latest)
        recorded: list[dict] = []

        with patch(
            "dispar_orchestrate.gold_latest_copy._enabled_publication_marts",
            return_value=[MART],
        ), patch(
            "dispar_orchestrate.gold_latest_copy.load_gold_catalog", return_value=catalog
        ), patch(
            "dispar_orchestrate.gold_latest_copy.record_gold_latest_copy_run",
            side_effect=lambda target, **kw: recorded.append(kw),
        ), patch(
            "dispar_orchestrate.gold_latest_copy.maintenance_paused", return_value=False
        ), patch(
            "dispar_orchestrate.gold_latest_copy.expire_all_but_current_snapshot",
            side_effect=RuntimeError("the catalog refused the expiry"),
        ) as expire, patch(
            "dispar_orchestrate.gold_latest_copy.remove_gold_orphan_files"
        ) as orphan:
            result = gold_latest_copy.refresh_latest_copy(_cfg(), _req())

        self.assertEqual(result, {"status": "success", "cleanup": "expire_snapshots_failed"})
        expire.assert_called_once()
        orphan.assert_not_called()
        self.assertEqual(recorded[0]["status"], "success")
        self.assertEqual(recorded[0]["cleanup"], "expire_snapshots_failed")


class LatestCopyRequestTests(unittest.TestCase):
    """The run-config rules (section 6's `invalid_request` row)."""

    def _valid(self) -> dict:
        return {
            "mart": MART,
            "table": TABLE,
            "latest_table": LATEST_TABLE,
            "copy_exported_at_micros": COPY_MICROS,
            "rows": 5,
        }

    def test_a_valid_config_builds_the_request(self) -> None:
        req = gold_latest_copy.LatestCopyRequest.from_op_config(self._valid())
        self.assertEqual(req.mart, MART)
        self.assertEqual(req.latest_table, LATEST_TABLE)
        self.assertEqual(req.rows, 5)

    def test_a_missing_field_is_refused(self) -> None:
        for key in ("mart", "table", "latest_table", "copy_exported_at_micros", "rows"):
            with self.subTest(key=key):
                config = self._valid()
                del config[key]
                with pytest.raises(gold_latest_copy.InvalidLatestCopyRequest):
                    gold_latest_copy.LatestCopyRequest.from_op_config(config)

    def test_zero_rows_and_zero_micros_are_refused(self) -> None:
        for key, value in (("rows", 0), ("copy_exported_at_micros", 0)):
            with self.subTest(key=key):
                config = self._valid()
                config[key] = value
                with pytest.raises(gold_latest_copy.InvalidLatestCopyRequest):
                    gold_latest_copy.LatestCopyRequest.from_op_config(config)

    def test_a_latest_table_that_does_not_match_is_refused(self) -> None:
        config = self._valid()
        config["latest_table"] = "somewhere_else_latest"
        with pytest.raises(gold_latest_copy.InvalidLatestCopyRequest):
            gold_latest_copy.LatestCopyRequest.from_op_config(config)


def _op_context(config: dict | None) -> object:
    return build_op_context(
        op_config=config
        if config is not None
        else {
            "mart": MART,
            "table": TABLE,
            "latest_table": LATEST_TABLE,
            "copy_exported_at_micros": COPY_MICROS,
            "rows": 5,
        }
    )


def test_an_invalid_run_config_records_invalid_request_and_fails_without_retry() -> None:
    """Two schema-valid but wrong shapes, one outcome: the row is
    `failed`/`invalid_request` with what is known, and the run fails
    without a retry (the same wrong config would come back). A config
    MISSING a field never reaches the op — Dagster refuses it at the
    context — so that rule is proved straight against
    `LatestCopyRequest.from_op_config` in `LatestCopyRequestTests`."""
    bad_configs = [
        # rows below one
        {
            "mart": MART,
            "table": TABLE,
            "latest_table": LATEST_TABLE,
            "copy_exported_at_micros": COPY_MICROS,
            "rows": 0,
        },
        # a latest_table that does not match the table
        {
            "mart": MART,
            "table": TABLE,
            "latest_table": "somewhere_else_latest",
            "copy_exported_at_micros": COPY_MICROS,
            "rows": 5,
        },
    ]
    for config in bad_configs:
        recorded: list[dict] = []
        with patch(
            "dispar_orchestrate.gold_export.GoldExportConfig.from_env",
            return_value=_cfg(ch=object()),
        ), patch(
            "dispar_orchestrate.gold_latest_copy.record_gold_latest_copy_run",
            side_effect=lambda target, **kw: recorded.append(kw),
        ):
            with pytest.raises(Failure) as caught:
                gold_latest_copy.refresh_gold_latest_copy(_op_context(config))
        assert caught.value.allow_retries is False
        assert caught.value.description == "invalid_request"
        assert len(recorded) == 1
        assert recorded[0]["status"] == "failed"
        assert recorded[0]["reason"] == "invalid_request"


def test_a_commit_conflict_records_commit_conflict_and_the_op_fails_retryably() -> None:
    from pyiceberg.exceptions import CommitFailedException

    latest = _LatestTable(
        properties={gold_latest_copy.LATEST_COPY_OF_PROPERTY: TABLE},
        schema_=_MAIN_SCHEMA,
        held_micros=COPY_MICROS - 10_000_000,
    )
    original = latest.transaction

    def conflicting_tx() -> object:
        tx = original()
        tx.overwrite = lambda df: (_ for _ in ()).throw(
            CommitFailedException("another writer committed first")
        )
        return tx

    latest.transaction = conflicting_tx  # type: ignore[method-assign]
    main = _MainTable(_MAIN_SCHEMA, copy_frame=_arrow_frame(COPY_MICROS, 5))
    catalog = _Catalog(main, latest)
    recorded: list[dict] = []

    with patch(
        "dispar_orchestrate.gold_export.GoldExportConfig.from_env", return_value=_cfg()
    ), patch(
        "dispar_orchestrate.gold_latest_copy._enabled_publication_marts",
        return_value=[MART],
    ), patch(
        "dispar_orchestrate.gold_latest_copy.load_gold_catalog", return_value=catalog
    ), patch(
        "dispar_orchestrate.gold_latest_copy.record_gold_latest_copy_run",
        side_effect=lambda target, **kw: recorded.append(kw),
    ):
        with pytest.raises(Failure) as caught:
            gold_latest_copy.refresh_gold_latest_copy(_op_context(None))

    assert caught.value.description == "commit_conflict"
    assert caught.value.allow_retries is True
    assert recorded[0]["status"] == "failed"
    assert recorded[0]["reason"] == "commit_conflict"


def test_the_constants_match_the_rust_names() -> None:
    """DATA-6 D1/D7: the suffix, the marker and the job name are pinned
    to the exact strings `lakehouse_iceberg::gold` and
    `lakehouse_api::gold_latest_copy` hold — a drift would make the Rust
    refusal and this module's write disagree about the same table."""
    assert gold_latest_copy.LATEST_TABLE_SUFFIX == "_latest"
    assert gold_latest_copy.LATEST_COPY_OF_PROPERTY == "rantai.latest-copy-of"


def test_the_job_is_serial_and_carries_its_run_limit() -> None:
    job = gold_latest_copy.gold_latest_copy_job
    assert job.name == "gold_latest_copy_job"
    assert job.tags.get(SERIAL_TAG) == "gold_latest_copy_job"
    assert "dagster/max_runtime" in job.tags


def test_the_exported_at_cell_is_exact_epoch_micros() -> None:
    """D9's superseded comparison is integer arithmetic, never float
    seconds: a naive timestamp cell converts exactly."""
    naive = datetime.datetime(2026, 11, 20, 10, 0, 0, 123456)
    assert gold_latest_copy._exported_at_as_micros(naive) == COPY_MICROS
    aware = naive.replace(tzinfo=datetime.timezone.utc)
    assert gold_latest_copy._exported_at_as_micros(aware) == COPY_MICROS


if __name__ == "__main__":
    unittest.main()
