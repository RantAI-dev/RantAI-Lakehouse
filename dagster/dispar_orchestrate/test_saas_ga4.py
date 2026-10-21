"""`saas/vendors/ga4.py` (`SRC-3` V-GA4.2): reports as objects, the date
dimension as the cursor, the schema from the property's metadata before a load. Through the real `SaasSession` over a FAKE GA4 Data API
(`test_saas_vendored_google_analytics.FakeGa4`). No network. The loader test runs
`load_via_sink` for real on `test_load_modes_measurement.Lab`; CI runs it in
`dagster-loader-tests`."""

from __future__ import annotations

from types import SimpleNamespace
from typing import Any

import pytest
from dlt.common import pendulum

from dispar_orchestrate import schema_observer
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import load_via_sink
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.vendors import ga4
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture
from dispar_orchestrate.test_saas_vendored_google_analytics import FakeGa4, session_over

REPORT = "traffic = country, date | sessions, purchaseRevenue"
DIAL = {"vendor": "ga4", "account": "123", "objects": [REPORT], "auth": {"type": "integration"}}
PARTS = SimpleNamespace(account="123")


def days_ago(n: int) -> str:
    return pendulum.yesterday().subtract(days=n).strftime("%Y%m%d")


def test_the_vendor_module_is_found_by_the_registry() -> None:
    assert vendors.module_for("ga4") is ga4


def test_a_report_is_written_as_label_dimensions_and_metrics_and_gets_the_date_when_it_has_no_time_dimension() -> None:
    assert ga4.parse_report("t = country | sessions") == ("t", ["country", "date"], ["sessions"])
    assert ga4.parse_report("m = yearMonth, country | sessions, customEvent:plan") == ("m", ["yearMonth", "country"], ["sessions", "customEvent:plan"])


@pytest.mark.parametrize(
    "text",
    ["", "country | sessions", "t = country sessions", "t = country |", "1t = country | sessions", "t = coun try | sessions", "t = country | sessions; DROP", "t = a, a | sessions",
     "t = ../x | sessions", "t = country | " + ",".join(f"m{i}" for i in range(30))],
)  # fmt: skip
def test_a_text_that_is_not_a_report_is_a_fixed_rejected_setting(text: str) -> None:
    with pytest.raises(saas_adapter.SaasSetupRefused) as raised:
        ga4.parse_report(text)
    assert "DROP" not in str(raised.value) and "../" not in str(raised.value)


def test_the_connectors_reports_are_its_objects_each_incremental_on_its_time_dimension_keyed_by_its_dimensions() -> None:
    (decl,) = ga4.list_objects(None, PARTS, DIAL)
    assert (decl.name, decl.cursor, decl.primary_key, decl.deletes, decl.lag_seconds) == (REPORT, "date", ("country", "date"), False, 3 * 86_400)
    assert decl.load_description() == "incremental"
    monthly = ga4.declaration(None, PARTS, DIAL, "m = yearMonth | sessions")
    assert monthly.cursor == "yearMonth" and monthly.primary_key == ("yearMonth",)


def test_describe_types_the_columns_from_the_properties_metadata_and_refuses_a_name_it_does_not_have() -> None:
    session = session_over(FakeGa4())
    fields = {x.name: x for x in ga4.describe(session, PARTS, DIAL, REPORT)}
    assert fields["date"].type_name == "timestamp" and fields["country"].type_name == "text"
    assert fields["sessions_INTEGER"].type_name == "bigint" and fields["purchaseRevenue_CURRENCY"].type_name == "double"
    assert fields["country"].nullable is False and fields["sessions_INTEGER"].nullable is True
    with pytest.raises(saas_adapter.SaasSetupRefused):
        ga4.describe(session, PARTS, DIAL, "t = country | nosuchmetric")


def test_a_first_run_reads_from_the_start_of_analytics_and_a_later_one_from_the_lagged_cursor() -> None:
    fake = FakeGa4([(days_ago(n), "ID", 10 - n, float(n)) for n in range(6)])
    session = session_over(fake)
    decl = ga4.declaration(session, PARTS, DIAL, REPORT)
    first = list(ga4.read(session, PARTS, DIAL, decl, since=None))
    assert len(first) == 6 and fake.posts[0][1]["dateRanges"][0]["startDate"] == "2015-08-14"
    cursor = max(r["date"] for r in first)
    fake.posts.clear()
    lagged = cursor.subtract(seconds=decl.lag_seconds)  # what dlt hands over: the cursor minus the lag
    again = list(ga4.read(session, PARTS, DIAL, decl, since=lagged))
    assert fake.posts[0][1]["dateRanges"][0]["startDate"] == lagged.to_date_string()
    assert sorted(r["date"].strftime("%Y%m%d") for r in again) == sorted(days_ago(n) for n in range(4)), "the last days are read again, nothing older"


def test_a_lag_needs_a_cursor_and_is_never_negative() -> None:
    from dispar_orchestrate.saas.objects import ObjectDecl

    with pytest.raises(ValueError):
        ObjectDecl(name="x", primary_key=("id",), full_reason="no filter", lag_seconds=60)
    with pytest.raises(ValueError):
        ObjectDecl(name="x", primary_key=("id",), cursor="t", lag_seconds=-1)
    assert ObjectDecl(name="x", primary_key=("id",), cursor="t", lag_seconds=60).lag_seconds == 60


# ---- the whole path: adapter, dlt incremental, merge on the dimensions, real Iceberg -------------------


def accept_all(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj() -> dict:
    return {"name": REPORT, "target": "ga_traffic", "loadMode": "incremental_dedup", "incrementalKey": "date", "primaryKey": ["country", "date"]}


@pytest.mark.loader
def test_a_second_run_corrects_the_last_days_and_stores_no_row_twice(lab: Lab, monkeypatch: pytest.MonkeyPatch) -> None:  # noqa: F811
    import time

    fake = FakeGa4([(days_ago(n), c, 10, 1.0) for n in range(4) for c in ("ID", "US")])
    module = ga4
    monkeypatch.setattr(saas_adapter, "open_session", lambda dial, secrets, **kw: (module, vendors.vendor("ga4"), PARTS, session_over(fake)))

    def run() -> Any:
        built = saas_adapter.build_source(DIAL, {}, [obj()], gate=accept_all)
        assert built is not None
        return load_via_sink(built.source, "ga_traffic", lab.config, f._load_plan("saas", obj()))

    assert run().rows == 8
    assert len(lab.rows("ga_traffic")) == 8
    time.sleep(1.1)
    # Google corrected yesterday's numbers after the first run, and a new day arrived
    fake.rows = [(d, c, 99 if d == days_ago(0) else s, r) for d, c, s, r in fake.rows]
    run()
    rows = lab.rows("ga_traffic")
    assert len(rows) == 8, "merged on country + date: no row twice"
    assert {r["sessions_integer"] for r in rows if r["country"] == "ID" and str(r["date"]).startswith(pendulum.yesterday().to_date_string())} == {99}
