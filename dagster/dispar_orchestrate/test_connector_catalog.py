"""Tests for `connector_catalog.py`: what a connector-loaded Bronze table is
registered in the catalog with. No ClickHouse: `ch_exec` and the catalog
database helper are injected."""

from __future__ import annotations

import pytest

from dispar_orchestrate.connector_catalog import CATALOG_DB, register_connector_table

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
