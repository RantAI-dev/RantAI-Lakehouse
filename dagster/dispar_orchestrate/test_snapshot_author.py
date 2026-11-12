"""`DATA-15` F5, `DATA-52`: our own `pyiceberg` commits record who made them, as the
snapshot summary key `rantai.author`, which the console's Snapshots card reads as the
version's author. The commit sites are `sink.empty_table` (via `clear_table`), the history
mode's transaction (`history.write_versions`), a type-changing replace
(`sink._replace_changing_types`) and the streamed write of a `replace` or an `append`
(`adapters/iceberg_stream.py`), on the harness of `test_load_modes_measurement.py`: a local
Iceberg catalog, no network.

A merge load is `dlt`'s own write, and it records no author; a commit made without an
author records none rather than a guess.
"""

from __future__ import annotations

import dataclasses
from typing import Any

import pytest

from dispar_orchestrate import ingest_factory
from dispar_orchestrate.adapters import iceberg_stream
from dispar_orchestrate.adapters.history import AUTHOR_PROPERTY, snapshot_properties
from dispar_orchestrate.adapters.sink import LoadPlan, SinkConfig, clear_table, load_via_sink
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture

pytestmark = pytest.mark.loader


def _authors(lab: Lab, name: str) -> list[str | None]:
    """The author of each snapshot, oldest first (`None` where none is recorded)."""
    snapshots = sorted(lab.table(name).snapshots(), key=lambda s: s.timestamp_ms)
    return [s.summary.additional_properties.get(AUTHOR_PROPERTY) if s.summary else None for s in snapshots]


def test_snapshot_properties_are_empty_without_an_author_and_one_key_with_one() -> None:
    assert snapshot_properties("") == {}
    assert snapshot_properties("connector c, run 1a2b") == {"rantai.author": "connector c, run 1a2b"}


def test_a_run_is_labelled_with_its_connector_and_the_start_of_its_run_id_never_an_invented_one() -> None:
    assert ingest_factory._run_author("conn-x", "1a2b3c4d-5e6f") == "connector conn-x, run 1a2b3c4d"
    assert ingest_factory._run_author("conn-x", None) == "connector conn-x"


def test_clearing_a_table_records_the_runs_author_on_the_delete_commit(lab: Lab) -> None:
    load_via_sink([{"id": 1, "v": "a"}], "t", lab.config, LoadPlan(mode="replace"))
    author_config: SinkConfig = dataclasses.replace(lab.config, author="connector conn-x, run 1a2b")

    clear_table("t", author_config)

    authors = _authors(lab, "t")
    assert authors[-1] == "connector conn-x, run 1a2b"
    assert authors[0] is None, "the dlt load before it records no author"


def test_clearing_a_table_without_a_run_records_no_author(lab: Lab) -> None:
    load_via_sink([{"id": 1, "v": "a"}], "t", lab.config, LoadPlan(mode="replace"))
    clear_table("t", lab.config)
    assert set(_authors(lab, "t")) == {None}


def test_a_replace_and_an_append_record_the_author_of_the_load(lab: Lab) -> None:
    # `DATA-52` (QA `LH-DATA-15-08`): both of these go through the streamed write, which
    # used to carry no author and left the version saying "unknown".
    config: SinkConfig = dataclasses.replace(lab.config, author="a person (file upload)")
    load_via_sink([{"id": 1, "v": "a"}], "t", config, LoadPlan(mode="replace"))
    load_via_sink([{"id": 2, "v": "b"}], "t", config, LoadPlan(mode="append"))

    assert _authors(lab, "t") == ["a person (file upload)"] * 2


def test_a_load_without_an_author_records_none_and_nothing_stays_registered(lab: Lab) -> None:
    load_via_sink([{"id": 1, "v": "a"}], "t", lab.config, LoadPlan(mode="replace"))
    assert iceberg_stream._snapshot_properties == {}
    load_via_sink([{"id": 2, "v": "b"}], "t", lab.config, LoadPlan(mode="append"))
    assert set(_authors(lab, "t")) == {None}
    assert iceberg_stream._snapshot_properties == {}

    def failing() -> Any:
        yield {"id": 3, "v": "c"}
        raise RuntimeError("the source failed part way")

    # The registration is removed on the way out, so a later load of the table records
    # its own author and not this one's.
    with pytest.raises(Exception):
        load_via_sink(failing(), "t", lab.config, LoadPlan(mode="append"))
    assert iceberg_stream._snapshot_properties == {}


def test_a_history_load_records_the_author_on_its_appended_and_updating_commits(lab: Lab) -> None:
    config = dataclasses.replace(lab.config, author="connector conn-x, run 9z8y")
    plan = LoadPlan(mode="history", key=("id",))
    load_via_sink([{"id": 1, "v": "a"}], "h", config, plan)
    # A change closes the old version (an upsert) and appends the new one.
    load_via_sink([{"id": 1, "v": "b"}], "h", config, plan)

    authors: list[Any] = _authors(lab, "h")
    assert "connector conn-x, run 9z8y" in authors
    assert authors[-1] == "connector conn-x, run 9z8y"


def test_every_snapshot_of_a_first_history_load_names_the_run(lab: Lab) -> None:
    # `DATA-52`, review round 1 R2: creating the table (and adding a column) is a
    # commit of the load too (`history.ensure_columns`), and it carried no author.
    config = dataclasses.replace(lab.config, author="connector conn-x, run 9z8y")

    load_via_sink([{"id": 1, "v": "a"}], "h", config, LoadPlan(mode="history", key=("id",)))

    assert set(_authors(lab, "h")) == {"connector conn-x, run 9z8y"}
