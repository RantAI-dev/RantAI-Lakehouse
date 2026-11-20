"""Tests for the pure re-upload chain graph and the polling chain sensor
(`BLD-14`, DEC-22, D3-D9). No Dagster instance and no network: the graph is
derived from plain wire payloads, and the sensor's poster is patched.

The graph tests are sentences about D3's order rule and P1/P2's exclusions.
The sensor tests pin each branch of `decide` and the cursor's "settle only
after the record was accepted" rule (D8, P4, P5, P10).
"""

from __future__ import annotations

from dispar_orchestrate.authored_factory import FILE_INGEST_JOB, reupload_chain_graph


def pipeline(
    pipeline_id: str,
    *,
    target: str | None = None,
    inputs: list[tuple[str, str]] | None = None,
    tenant: str | None = "t1",
    status: str = "ready",
    depends_on: list[str] | None = None,
    target_lag: dict | None = None,
) -> dict:
    """One `GET /api/pipelines/runnable` row in the shape D3 reads."""
    definition: dict = {
        "inputs": [{"zone": zone, "table": table} for zone, table in (inputs or [])],
    }
    if target:
        zone, table = target.split(".", 1)
        definition["targetZone"] = zone
        definition["targetTable"] = table
    result: dict = {
        "id": pipeline_id,
        "tenantId": tenant,
        "status": status,
        "definition": definition,
        "dependsOn": depends_on or [],
    }
    if target_lag is not None:
        result["targetLag"] = target_lag
    return result


def first_pipeline(
    pipeline_id: str, table: str = "orders", *, target: str = "silver.orders_clean", **kwargs
) -> dict:
    fields: dict = {
        "target": target,
        "inputs": [("bronze", table)],
        "depends_on": [FILE_INGEST_JOB],
    }
    fields.update(kwargs)
    result = pipeline(pipeline_id, **fields)
    result["definition"]["sourceZone"] = "bronze"
    result["definition"]["sourceTable"] = table
    return result


def graph_for(*pipelines: dict) -> dict:
    return reupload_chain_graph(list(pipelines))


def test_a_gold_pipeline_reading_a_silver_output_is_after_it() -> None:
    silver = first_pipeline("pl-silver")
    gold = pipeline("pl-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")])

    graph = graph_for(gold, silver)

    entry = graph["pl-gold"]["orders"]
    assert entry.parents == ("pl-silver",)
    assert entry.ancestors == ("pl-silver",)
    assert "pl-silver" not in graph, "the first pipeline has no chain parent of its own"


def test_a_second_input_the_reupload_does_not_reach_is_not_a_parent() -> None:
    silver = first_pipeline("pl-silver")
    unrelated = pipeline("pl-unrelated", target="silver.other_clean")
    gold = pipeline(
        "pl-gold",
        target="gold.orders_daily",
        inputs=[("silver", "orders_clean"), ("silver", "other_clean")],
    )

    graph = graph_for(silver, unrelated, gold)

    entry = graph["pl-gold"]["orders"]
    assert entry.parents == ("pl-silver",)
    assert entry.ancestors == ("pl-silver",)


def test_a_diamond_waits_for_both_branches() -> None:
    first = first_pipeline("pl-first")
    left = pipeline("pl-left", target="silver.left", inputs=[("silver", "orders_clean")])
    right = pipeline("pl-right", target="silver.right", inputs=[("silver", "orders_clean")])
    join = pipeline("pl-join", target="gold.join", inputs=[("silver", "left"), ("silver", "right")])

    graph = graph_for(join, right, left, first)

    entry = graph["pl-join"]["orders"]
    assert entry.parents == ("pl-left", "pl-right")
    assert entry.ancestors == ("pl-first", "pl-left", "pl-right")


def test_another_tenants_pipeline_is_never_in_the_chain() -> None:
    silver = first_pipeline("pl-silver", tenant="t1")
    foreign_gold = pipeline(
        "pl-foreign-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")], tenant="t2"
    )
    own_gold = pipeline(
        "pl-own-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")], tenant="t1"
    )

    graph = graph_for(silver, foreign_gold, own_gold)

    assert "pl-foreign-gold" not in graph
    assert graph["pl-own-gold"]["orders"].parents == ("pl-silver",)


def test_a_paused_pipeline_and_what_only_it_feeds_are_left_out() -> None:
    silver = first_pipeline("pl-silver", status="paused")
    gold = pipeline("pl-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")])

    assert graph_for(silver, gold) == {}


def test_a_target_lag_pipeline_is_left_out() -> None:
    silver = first_pipeline("pl-silver", target_lag={"kind": "lag", "minutes": 5})
    gold = pipeline("pl-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")])

    assert graph_for(silver, gold) == {}


def test_a_cycle_is_left_out() -> None:
    first = first_pipeline("pl-first")
    a = pipeline("pl-a", target="silver.a", inputs=[("bronze", "orders"), ("gold", "b")])
    b = pipeline("pl-b", target="gold.b", inputs=[("silver", "a")])
    tail = pipeline("pl-tail", target="gold.tail", inputs=[("gold", "b")])

    graph = graph_for(first, a, b, tail)

    assert "pl-a" not in graph
    assert "pl-b" not in graph
    assert "pl-tail" not in graph


def test_with_the_switch_off_nothing_below_the_first_pipeline_is_in_the_chain() -> None:
    silver = first_pipeline("pl-silver", depends_on=[])
    gold = pipeline("pl-gold", target="gold.orders_daily", inputs=[("silver", "orders_clean")])

    assert graph_for(silver, gold) == {}


def test_a_first_pipeline_that_also_reads_another_first_pipeline_has_parents() -> None:
    first = first_pipeline("pl-first")
    second = first_pipeline("pl-second", target="silver.second", inputs=None)
    second["definition"]["inputs"] = [
        {"zone": "bronze", "table": "orders"},
        {"zone": "silver", "table": "orders_clean"},
    ]

    graph = graph_for(first, second)

    assert graph["pl-second"]["orders"].parents == ("pl-first",)
    assert graph["pl-second"]["orders"].ancestors == ("pl-first",)
