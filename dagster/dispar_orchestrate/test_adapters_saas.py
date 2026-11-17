"""`adapters/saas.py` and its place in `ingest_factory` / `ingest_errors`
(`SRC-3` T1.9), with a FAKE vendor registered for the test: slice 1 ships the
adapter with no vendor behind it. No network: the fake module reads a list.

The loads run `load_via_sink` for real on `test_load_modes_measurement.Lab`
(a local Iceberg catalog), like `test_sink_dedup_modes.py`; CI runs them in
`dagster-loader-tests`.
"""

from __future__ import annotations

import time
from typing import Any

import pytest

from dispar_orchestrate import ingest_errors
from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate import schema_observer
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters.sink import UnsupportedLoadMode, load_via_sink
from dispar_orchestrate.saas import session as saas_session
from dispar_orchestrate.saas import vendors
from dispar_orchestrate.saas.objects import ObjectDecl
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture

DIAL = {"vendor": "hubspot", "account": "123456", "auth": {"type": "integration"}}
SECRETS = {"token": "not-a-real-token"}


class FakeVendor:
    """A vendor whose API is a list. `calls` records what `read` was asked and
    `served` how many records it handed back, per call."""

    def __init__(self) -> None:
        self.records: list[dict[str, Any]] = []
        self.calls: list[Any] = []
        self.served: list[int] = []
        self.fields = [saas_adapter.Field("id", "integer", False), saas_adapter.Field("subject", "text"), saas_adapter.Field("updated_at", "integer")]
        self.cursor: str | None = "updated_at"

    def declaration(self, session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
        if self.cursor is None:
            return ObjectDecl(name=name, primary_key=("id",), full_reason="the fake API has no last-modified filter")
        return ObjectDecl(name=name, primary_key=("id",), cursor=self.cursor)

    def list_objects(self, session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
        return [self.declaration(session, parts, dial, "tickets")]

    def describe(self, session: Any, parts: Any, dial: dict, name: str) -> list[saas_adapter.Field]:
        return self.fields

    def read(self, session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any):
        self.calls.append(since)
        rows = [dict(r) for r in self.records if since is None or r["updated_at"] >= since]
        self.served.append(len(rows))
        yield from rows


@pytest.fixture
def vendor():
    fake = FakeVendor()
    vendors.register("hubspot", fake)
    yield fake
    vendors.unregister("hubspot")


def accept_all(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
    return schema_observer.Decision(action="load", columns=None, changes=[])


def obj(**extra: Any) -> dict:
    return {"name": "tickets", "target": "tickets", "loadMode": "incremental_dedup", "incrementalKey": "updated_at", "primaryKey": ["id"], **extra}


def build(o: dict, gate=accept_all):  # noqa: ANN001
    return saas_adapter.build_source(DIAL, SECRETS, [o], gate=gate)


def run(lab: Lab, o: dict, gate=accept_all):  # noqa: ANN001
    result = build(o, gate)
    assert result is not None
    return load_via_sink(result.source, o["target"], lab.config, f._load_plan("saas", o))


@pytest.mark.loader
def test_an_incremental_second_run_reads_only_newer_rows_and_merges_on_the_key(lab: Lab, vendor: FakeVendor) -> None:
    vendor.records = [{"id": 1, "subject": "a", "updated_at": 10}, {"id": 2, "subject": "b", "updated_at": 20}]
    first = run(lab, obj())
    assert first.rows == 2
    time.sleep(1.1)
    vendor.records += [{"id": 2, "subject": "b2", "updated_at": 30}, {"id": 3, "subject": "c", "updated_at": 40}]
    vendor.records = [r for r in vendor.records if not (r["id"] == 2 and r["updated_at"] == 20)]
    second = run(lab, obj())
    assert vendor.calls[0] is None and vendor.calls[1] == 20, "the second run asks from the cursor's last value"
    assert vendor.served[1] == 2, "only the newer rows came back"
    assert second.rows == 2
    rows = {r["id"]: r["subject"] for r in lab.rows("tickets")}
    assert rows == {1: "a", 2: "b2", 3: "c"}, "merged on the key: no duplicate"
    assert len(lab.rows("tickets")) == 3


@pytest.mark.loader
def test_a_left_out_column_is_absent_from_the_table_and_the_key_is_kept(lab: Lab, vendor: FakeVendor) -> None:
    vendor.records = [{"id": 1, "subject": "a", "updated_at": 10}]
    run(lab, obj(excludedColumns=["subject"]), gate=None)  # a vendor module without a gate: the exclusion applies to each record
    row = lab.rows("tickets")[0]
    assert "subject" not in row and row["id"] == 1 and row["updated_at"] == 10


@pytest.mark.loader
def test_the_apis_column_decision_narrows_the_load(lab: Lab, vendor: FakeVendor) -> None:
    vendor.records = [{"id": 1, "subject": "a", "updated_at": 10}]

    def narrow(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
        assert [c.name for c in reflected.columns] == ["id", "subject", "updated_at"]
        assert reflected.primary_key == ("id",)
        return schema_observer.Decision(action="load", columns=["id", "updated_at"], changes=[])

    run(lab, obj(), gate=narrow)
    assert "subject" not in lab.rows("tickets")[0]


def test_a_describe_change_the_api_holds_back_loads_nothing(vendor: FakeVendor) -> None:
    vendor.records = [{"id": 1, "subject": "a", "updated_at": 10}]
    asked: list[schema_observer.ReflectedTable] = []

    def wait(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
        asked.append(reflected)
        return schema_observer.Decision(action="wait", columns=None, changes=[{"kind": "added"}])

    assert build(obj(), gate=wait) is None
    assert len(asked) == 1
    assert vendor.calls == [], "nothing was read once the API said wait"


def test_a_vendor_that_describes_its_fields_is_observed_before_the_load_only(vendor: FakeVendor) -> None:
    result = build(obj())
    assert result is not None and result.observed_before_load is True
    delattr(FakeVendor, "describe")
    try:
        result = build(obj())
    finally:
        FakeVendor.describe = lambda self, session, parts, dial, name: self.fields  # type: ignore[method-assign]
    assert result is not None and result.observed_before_load is False


def test_an_incremental_load_of_an_object_without_a_declared_cursor_is_refused(vendor: FakeVendor) -> None:
    vendor.cursor = None
    with pytest.raises(UnsupportedLoadMode, match="cannot filter"):
        build(obj())
    vendor.cursor = "updated_at"
    with pytest.raises(UnsupportedLoadMode, match="cannot filter"):
        build(obj(incrementalKey="created_at"))


def test_plain_incremental_is_refused_for_a_business_app(vendor: FakeVendor) -> None:
    """PR #216 final review BLOCKER 18: `incremental` appends, so a changed vendor record would be a
    second row under the same id. The message is fixed and names the only mode that merges."""
    with pytest.raises(UnsupportedLoadMode, match="incremental_dedup"):
        build(obj(loadMode="incremental"))
    assert vendor.calls == [], "nothing is read for a refused mode"


def test_a_key_other_than_the_vendors_is_refused_where_the_mode_uses_it(vendor: FakeVendor) -> None:
    """PR #216 final review BLOCKER 18: a key that differs from the vendor's own folds distinct
    records into one (a GA4 report saved with `primaryKey: ["date"]` collapses every country of a
    day). Checked for every mode that keeps one row per key; `replace` ignores the key."""
    for mode in ("replace_dedup", "incremental_dedup"):
        with pytest.raises(UnsupportedLoadMode, match="vendor identifies one record"):
            build(obj(loadMode=mode, primaryKey=["subject"]))
    assert build(obj(loadMode="replace", primaryKey=["subject"])) is not None


def test_a_full_read_object_loads_without_a_cursor(vendor: FakeVendor) -> None:
    vendor.cursor = None
    vendor.records = [{"id": 1, "subject": "a", "updated_at": 10}]
    result = build({"name": "owners", "target": "owners", "loadMode": "replace_dedup", "primaryKey": ["id"]})
    assert result is not None
    assert list(result.source) == [{"id": 1, "subject": "a", "updated_at": 10}]


def test_the_factory_offers_incremental_modes_to_saas_and_sql_but_not_to_rest() -> None:
    o = obj()
    assert f._load_plan("saas", o).mode == "incremental_dedup"
    with pytest.raises(UnsupportedLoadMode, match="only available for SQL"):
        f._load_plan("rest", o)


def test_a_vendor_without_a_module_is_unsupported_and_not_retried(monkeypatch: pytest.MonkeyPatch) -> None:
    # Slice 6 gave the last of the ten a module; the case is still real for a vendor id added to the table first.
    monkeypatch.delitem(vendors.MODULE_PATHS, "netsuite")
    monkeypatch.delitem(vendors._MODULES, "netsuite", raising=False)
    with pytest.raises(vendors.UnsupportedVendor) as raised:
        saas_adapter.build_source({"vendor": "netsuite", "account": "1234567", "auth": {"type": "integration"}}, SECRETS, [obj()])
    assert ingest_errors.classify(raised.value) == ingest_errors.Classified(ingest_errors.UNSUPPORTED, retried=False)


def test_an_integration_credential_the_vendor_does_not_have_is_a_rejected_setting() -> None:
    vendors.register("salesforce", FakeVendor())
    try:
        with pytest.raises(saas_adapter.SaasSetupRefused) as raised:
            saas_adapter.build_source({"vendor": "salesforce", "account": "acme", "auth": {"type": "integration"}}, {}, [obj()])
    finally:
        vendors.unregister("salesforce")
    assert ingest_errors.classify(raised.value).error_class == ingest_errors.SETTING_REJECTED


def test_each_saas_failure_has_its_own_class_and_only_the_rate_limit_is_retried() -> None:
    sign_in = ingest_errors.classify(saas_session.SaasSignInRefused())
    assert (sign_in.error_class, sign_in.retried) == ("sign_in_refused", False)
    permission = ingest_errors.classify(saas_session.SaasPermissionMissing())
    assert (permission.error_class, permission.retried) == ("permission_missing", False)
    assert ingest_errors.is_permission_refusal(saas_session.SaasPermissionMissing())
    limited = ingest_errors.classify(saas_session.SaasRateLimited())
    assert (limited.error_class, limited.retried) == ("rate_limited", True)
    assert ingest_errors.CLASS_PHRASES["rate_limited"] == "the source limited the rate of requests"
    assert "limited the rate" in str(saas_session.SaasRateLimited())


def test_the_factory_resolves_saas_secrets_through_the_vendor_table(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[str | None] = []
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: seen.append(ref) or f"value-of-{ref}")
    one = f._resolve_object_secrets({"secretRef": "env:A"}, "saas", {"vendor": "hubspot", "account": "1", "auth": {"type": "integration"}})
    assert one == {"token": "value-of-env:A"}
    two = f._resolve_object_secrets(
        {"secretRef": "env:A", "secretRefSecondary": "env:B"},
        "saas",
        {"vendor": "zendesk", "account": "x", "auth": {"type": "oauth2_authorization_code", "clientId": "c"}},
    )
    assert two == {"clientSecret": "value-of-env:A", "refreshToken": "value-of-env:B"}
    assert vendors.auth_type_of({"vendor": "nobody", "auth": {"type": "integration"}}) is None


def test_an_object_the_vendor_cannot_describe_takes_the_after_load_path(vendor: FakeVendor) -> None:
    """HubSpot has no properties API for owners: `describe` answers None (V-HubSpot.2)."""
    vendor.describe = lambda session, parts, dial, name: None  # type: ignore[method-assign]
    asked: list[object] = []

    def never(reflected: schema_observer.ReflectedTable) -> schema_observer.Decision:
        asked.append(reflected)
        return accept_all(reflected)

    result = build(obj(), gate=never)
    assert result is not None and result.observed_before_load is False and asked == []
