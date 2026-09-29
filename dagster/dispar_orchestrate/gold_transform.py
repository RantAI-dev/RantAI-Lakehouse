"""Silver -> Gold: the business-facing marts, in ClickHouse `serving`.

Gold is not a concept in this product, it is a location: `routes::dashboard`
lists chart sources from `system.tables WHERE database='serving'`, and
`lakehouse_bi::builder` hardcodes `FROM serving.<mart>`. A table that is not
in `serving` cannot back a dashboard tile, however clean it is. The copilot
narrows that further — `routes::agent::schema_context` injects only
`serving.mart_*` into the model's system prompt — so a mart that wants to be
answerable in chat needs the `mart_` prefix too.

Models are declared, not coded: see `ch_models.Model`. Adding a mart is one
entry in [`MODELS`], not a new module.

Source is `silver.g3a_orders`, which `silver_transform_job` rebuilds from
Bronze. Both reads carry `FINAL`: Silver is a
`ReplacingMergeTree(_ingested_at)` whose duplicate versions are only
collapsed at an unscheduled merge, and an aggregate that omits `FINAL`
silently sums rows that a merge has not removed yet — the exact
double-counting Silver exists to fix (Bronze holds 4 copies of every row
after 4 ingest runs; Silver holds 1).
"""

from __future__ import annotations

from dagster import job, op

from dispar_orchestrate.ch_models import Model, run_models

SILVER_ORDERS = "silver.`g3a_orders` FINAL"

MODELS = [
    Model(
        name="gold_orders_daily",
        target="serving.mart_orders_daily",
        engine="MergeTree ORDER BY order_date",
        select=(
            "SELECT toDate(created_at) AS order_date,"
            "       count() AS orders,"
            "       uniqExact(customer) AS customers,"
            "       round(sum(amount), 2) AS revenue,"
            "       round(avg(amount), 2) AS avg_order_value "
            f"FROM {SILVER_ORDERS} "
            "GROUP BY order_date"
        ),
    ),
    Model(
        name="gold_orders_by_customer",
        target="serving.mart_orders_by_customer",
        engine="MergeTree ORDER BY customer",
        select=(
            "SELECT customer,"
            "       count() AS orders,"
            "       round(sum(amount), 2) AS revenue,"
            "       round(avg(amount), 2) AS avg_order_value,"
            "       max(created_at) AS last_order_at "
            f"FROM {SILVER_ORDERS} "
            "GROUP BY customer"
        ),
    ),
]


@op
def build_gold_marts(context) -> dict:
    counts = run_models(MODELS, log=context.log)
    context.add_output_metadata({f"{name}_rows": rows for name, rows in counts.items()})
    return counts


@job
def gold_transform_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `gold_transform_job`. Distinct
    from `gold_export_job`, which is the opposite direction: that one
    EXPORTS an existing Gold mart out to Iceberg (ADR 0010), this one
    BUILDS the mart from Silver in the first place."""
    build_gold_marts()
