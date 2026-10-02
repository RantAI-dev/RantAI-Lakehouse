"""Tests for `connector_catalog.py`: what a loaded Bronze table, a connector's
or an upload's, is registered in the catalog with. No ClickHouse: `ch_exec`
and the catalog database helper are injected."""

from __future__ import annotations

import pytest

from dispar_orchestrate.connector_catalog import (
    CATALOG_DB,
    is_plain_table_name,
    register_connector_table,
    register_loaded_table,
)

DESCRIBE_TSV = "\n".join(
    [
        "order_id\tInt16\t\t\t\t\t",
        "customer_id\tNullable(String)\t\t\t\t\t",
        "_ingested_at\tDateTime64(6, 'UTC')\t\t\t\t\t",
        "_dlt_load_id\tString\t\t\t\t\t",
        "_dlt_id\tString\t\t\t\t\t",
    ]
)


def _fake_exec(statements):
    def _exec(ch, statement):
        statements.append(statement)
        return "1672" if statement.startswith("SELECT count()") else DESCRIBE_TSV

    return _exec


def test_registers_the_tables_total_and_columns_without_dlt_bookkeeping():
    statements, databases, registered = [], [], []
    total = register_connector_table(
        "conn-northwind",
        {"name": "public.orders", "target": "northwind_orders"},
        ch=object(),
        ch_exec=_fake_exec(statements),
        ensure_catalog_database=lambda ch, db: databases.append(db),
        register=lambda **kw: registered.append(kw),
    )

    assert total == 1672
    assert databases == [CATALOG_DB]
    # R11: the total is counted under a `WHERE`, never as a bare row count.
    assert statements[0] == f"SELECT count() FROM {CATALOG_DB}.`bronze.northwind_orders` WHERE 1"
    [entry] = registered
    assert entry["slug"] == "northwind-orders"
    assert entry["title"] == "Northwind Orders"
    assert entry["bronze_table_name"] == "northwind_orders"
    # The table's total across every load, not one run's rows.
    assert entry["row_count"] == 1672
    assert entry["author"] == "connector conn-northwind"
    assert "public.orders" in entry["description"]
    assert entry["columns"] == [
        ("order_id", "Int16", ""),
        ("customer_id", "Nullable(String)", ""),
        ("_ingested_at", "DateTime64(6, 'UTC')", ""),
    ]


@pytest.mark.parametrize("target", ["Orders", "orders`; DROP TABLE x", "1orders", ""])
def test_refuses_a_target_that_is_not_a_plain_identifier(target):
    with pytest.raises(ValueError, match="not a plain lower-case identifier"):
        register_connector_table(
            "conn-x",
            {"name": "public.orders", "target": target},
            ch=object(),
            ch_exec=lambda *a: pytest.fail("no SQL may run for a refused target"),
            ensure_catalog_database=lambda *a: None,
            register=lambda **kw: pytest.fail("nothing may be registered"),
        )


def test_an_upload_registers_through_the_same_helper_with_its_own_description_and_author():
    statements, databases, registered = [], [], []
    total = register_loaded_table(
        "g9_orders",
        description="Bronze Iceberg table loaded from an uploaded file (upload u-1).",
        author="upload",
        ch=object(),
        ch_exec=_fake_exec(statements),
        ensure_catalog_database=lambda ch, db: databases.append(db),
        register=lambda **kw: registered.append(kw),
    )

    assert total == 1672
    assert databases == [CATALOG_DB]
    # R11 again: the one place that counts a loaded table counts it under a `WHERE`.
    assert statements[0] == f"SELECT count() FROM {CATALOG_DB}.`bronze.g9_orders` WHERE 1"
    [entry] = registered
    assert entry["slug"] == "g9-orders"
    assert entry["title"] == "G9 Orders"
    assert entry["bronze_table_name"] == "g9_orders"
    assert entry["row_count"] == 1672
    assert entry["author"] == "upload"
    assert entry["description"] == "Bronze Iceberg table loaded from an uploaded file (upload u-1)."
    assert [name for name, _, _ in entry["columns"]] == ["order_id", "customer_id", "_ingested_at"]


def test_the_connector_wrapper_and_the_shared_helper_register_the_same_table_the_same_way():
    wrapped, shared = [], []
    register_connector_table(
        "conn-x",
        {"name": "public.orders", "target": "orders"},
        ch=object(),
        ch_exec=_fake_exec([]),
        ensure_catalog_database=lambda *a: None,
        register=lambda **kw: wrapped.append(kw),
    )
    register_loaded_table(
        "orders",
        description=wrapped[0]["description"],
        author=wrapped[0]["author"],
        ch=object(),
        ch_exec=_fake_exec([]),
        ensure_catalog_database=lambda *a: None,
        register=lambda **kw: shared.append(kw),
    )
    assert wrapped == shared


@pytest.mark.parametrize("table", ["orders\n", "Orders", "1orders", "", "a-b", "a b", "orders`"])
def test_a_table_name_that_is_not_a_plain_identifier_is_refused_even_with_a_trailing_newline(table):
    # `$` in a pattern also matches in front of a trailing newline, so a
    # `match` against `^...$` accepted "orders\n"; `fullmatch` does not.
    assert not is_plain_table_name(table)
    with pytest.raises(ValueError, match="not a plain lower-case identifier"):
        register_loaded_table(
            table,
            description="d",
            author="a",
            ch=object(),
            ch_exec=lambda *a: pytest.fail("no SQL may run for a refused table name"),
            ensure_catalog_database=lambda *a: None,
            register=lambda **kw: pytest.fail("nothing may be registered"),
        )


@pytest.mark.parametrize("table", ["orders", "_x", "a__b", "g9_upload_0a1b2c3d", "t1"])
def test_a_plain_lower_case_identifier_is_a_table_name(table):
    assert is_plain_table_name(table)
