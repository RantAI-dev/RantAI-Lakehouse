"""`DATA-50` (`LH-DATA-9-37`): a load into a Bronze table whose name is longer
than 50 characters fails, though the page and the API accept 128.

The cause is `dlt`'s, not a name of ours. A table's pipeline is named
`bronze_ingest_<table>` (`adapters/sink.py::_bronze_pipeline`), fourteen
characters more than the table, and `dlt` refuses a schema name longer than 64
characters (`InvalidSchemaName`): a table name past 50 therefore has no legal
default schema name. The refusal is reached on the first load of a table, where
`history.open_table` asks `dlt` for a destination client before the pipeline has
a schema, so `dlt` builds one named after the pipeline.

The first test pins the library's own limit (it passes before and after the fix
these tests prepare); the others are the failure QA saw, through the sink on the
local Iceberg harness of `test_load_modes_measurement.py` -- no network.
"""

from __future__ import annotations

import pytest
from dlt.common.schema.exceptions import InvalidSchemaName
from dlt.extract.utils import make_schema_with_default_name
from dlt.common.schema import Schema

from dispar_orchestrate.adapters.sink import LoadPlan, clear_table, load_via_sink
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture

pytestmark = pytest.mark.loader

PIPELINE_PREFIX = "bronze_ingest_"


def _table_name(length: int) -> str:
    """A table name of exactly `length` characters."""
    return "t" + "a" * (length - 1)


def test_dlt_refuses_a_schema_named_after_a_pipeline_of_more_than_64_characters() -> None:
    # 64 is `dlt`'s limit for a schema name, and the pipeline name is the table's
    # plus `PIPELINE_PREFIX`: 14 characters, so the table may be 50.
    assert make_schema_with_default_name(PIPELINE_PREFIX + "a" * 50) == PIPELINE_PREFIX + "a" * 50
    for table_length in (51, 52, 128):
        with pytest.raises(InvalidSchemaName):
            Schema(PIPELINE_PREFIX + "a" * table_length)
    # What `dlt` builds for the pipeline's real schema is a stable shortened name
    # with a hash tag, so nothing of ours is cut into a collision.
    first = make_schema_with_default_name(PIPELINE_PREFIX + "a" * 100 + "b")
    second = make_schema_with_default_name(PIPELINE_PREFIX + "a" * 100 + "c")
    assert len(first) == 64
    assert len(second) == 64
    assert first != second


@pytest.mark.xfail(
    strict=True,
    raises=InvalidSchemaName,
    reason="QA LH-DATA-9-37 (DATA-50): fixed in the next commit",
)
@pytest.mark.parametrize("length", [52, 128])
def test_a_first_typed_replace_into_a_long_table_name_loads(lab: Lab, length: int) -> None:
    name = _table_name(length)
    replaced = load_via_sink(
        [{"id": 1, "v": "a"}],
        name,
        lab.config,
        LoadPlan(mode="replace"),
        column_types={"id": "long", "v": "string"},
    )
    assert replaced.rows == 1
    assert replaced.has_failed_jobs is False
    appended = load_via_sink([{"id": 2, "v": "b"}], name, lab.config, LoadPlan(mode="append"))
    assert appended.rows == 1
    assert sorted(row["id"] for row in lab.rows(name)) == [1, 2]


@pytest.mark.xfail(
    strict=True,
    raises=InvalidSchemaName,
    reason="QA LH-DATA-9-37 (DATA-50): fixed in the next commit",
)
def test_clearing_a_long_named_table_that_was_never_loaded_says_there_is_nothing_to_empty(lab: Lab) -> None:
    assert clear_table(_table_name(128), lab.config).load_info_str == "table not loaded; nothing to empty"


def test_a_first_load_without_a_column_list_into_a_128_character_name_loads(lab: Lab) -> None:
    # The planner's reading of the cause: a load that does not open the table
    # first (`column_types=None`, so no `replace`) never asks `dlt` for a client.
    result = load_via_sink([{"id": 1}], _table_name(128), lab.config, LoadPlan(mode="append"))
    assert result.rows == 1
