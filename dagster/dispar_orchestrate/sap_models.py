"""SAP material master: Bronze -> Silver -> Gold, as declared models.

The source is a SAP "Dynamic List Display" export of the material master
(`raw_sap.material_master` in Postgres, loaded from the customer's file),
ingested to Bronze by `bronze_ingest_job`. Every Bronze column is a
`String`, because the export is text and Bronze keeps the source's own
shape — typing it is Silver's job, which is exactly what this module does.

# What Silver fixes here, concretely

- **Dates.** SAP writes `DD.MM.YYYY`. Left as text, `created` sorts
  alphabetically ("01.12.2020" before "07.02.2025" before "17.05.2023"),
  so every "newest first" answer is wrong. `parseDateTimeBestEffortOrNull`
  with `dateDMY` turns them into real `Date`s.
- **Numbers.** `gross_weight`/`net_weight`/`volume` arrive as `"0.000"`.
  As text, `sum()` is impossible and `>` compares character by character.
- **Empty vs missing.** SAP fills unknown values with an empty string, not
  NULL. `nullIf(trim(x), '')` makes "no value" a single, countable thing.
- **Duplicates.** `ReplacingMergeTree` keyed on `(plnt, material)` keeps
  one row per material per plant, so a re-ingest cannot inflate counts.

Only the columns worth typing are promoted; the export's remaining fields
stay in Bronze, reachable through the catalog when someone needs them.
That is the point of keeping Bronze: Silver is a curated view, not a
lossy migration.
"""

from __future__ import annotations

from dagster import job, op

from dispar_orchestrate.ch_models import Model, ch_target, ensure_catalog_database, run_models

CATALOG_DB = "icecat_sap"
BRONZE = f"{CATALOG_DB}.`bronze.sap_material_master`"
SILVER = "silver.`sap_material_master`"


def _date(col: str) -> str:
    """SAP `DD.MM.YYYY` -> `Nullable(Date)`; unparseable stays NULL rather
    than defaulting to 1970, which would quietly land in every chart.

    Explicit rearrangement into `YYYY-MM-DD` rather than
    `parseDateTimeBestEffort`: "best effort" reads `07.02.2025` as
    ambiguous and resolves day/month by a SETTING
    (`date_time_input_format`), not by an argument — a silent
    month/day swap for every day <= 12 if that setting is ever not what
    this code assumed. `parseDateTimeOrNull(x, '%d.%m.%Y')` also works but
    returns a `DateTime`, adding a midnight time component the source
    never had.
    """
    return (
        f"toDateOrNull(concat("
        f"substring(nullIf(trim({col}), ''), 7, 4), '-', "
        f"substring(nullIf(trim({col}), ''), 4, 2), '-', "
        f"substring(nullIf(trim({col}), ''), 1, 2)))"
    )


def _num(col: str) -> str:
    return f"toFloat64OrNull(nullIf(trim({col}), ''))"


def _str(col: str) -> str:
    return f"nullIf(trim({col}), '')"


MODELS = [
    Model(
        name="silver_sap_material_master",
        target="silver.sap_material_master",
        engine="ReplacingMergeTree(_ingested_at) ORDER BY (plnt, material)",
        select=(
            "SELECT "
            # `ifNull(..., '')`, not `trim(...)` alone: every Bronze column
            # is `Nullable(String)`, and a `MergeTree` sorting key rejects
            # nullable columns unless `allow_nullable_key` is on (it is
            # off, by default and deliberately — a NULL in a sort key makes
            # range pruning ambiguous). The two key columns are therefore
            # non-nullable by construction; a material with no id is
            # filtered out by the `WHERE` below rather than sorted as ''.
            f"  ifNull(trim(plnt), '') AS plnt,"
            f"  ifNull(trim(material), '') AS material,"
            f"  {_str('material_description')} AS material_description,"
            f"  {_str('mtyp')} AS material_type,"
            f"  {_str('matl_group')} AS material_group,"
            f"  {_str('ext_material_grp')} AS ext_material_group,"
            f"  {_str('bun')} AS base_unit,"
            f"  {_num('gross_weight')} AS gross_weight,"
            f"  {_num('net_weight')} AS net_weight,"
            f"  {_str('wun')} AS weight_unit,"
            f"  {_num('volume')} AS volume,"
            f"  {_str('vun')} AS volume_unit,"
            f"  {_str('pgr')} AS purchasing_group,"
            f"  {_str('proctype')} AS procurement_type,"
            f"  {_date('created')} AS created_at,"
            f"  {_str('created_by')} AS created_by,"
            f"  {_date('last_chg')} AS changed_at,"
            f"  {_str('changed_by')} AS changed_by,"
            # Same nullability rule as the sort key, for a different
            # reason: `ReplacingMergeTree`'s version column must not be
            # nullable, because "which version wins" has no answer when the
            # version is NULL. dlt stamps `_ingested_at` on every row, so
            # the fallback is unreachable in practice; epoch is the right
            # value for it anyway — a row with no load stamp should always
            # lose to one that has it.
            "  coalesce(toDateTime64(_ingested_at, 3, 'UTC'), toDateTime64(0, 3, 'UTC')) AS _ingested_at "
            f"FROM {BRONZE} "
            "WHERE trim(material) != ''"
        ),
    ),
    Model(
        name="gold_material_by_type",
        target="serving.mart_material_by_type",
        engine="MergeTree ORDER BY material_type",
        select=(
            "SELECT "
            "  coalesce(material_type, '(kosong)') AS material_type,"
            "  count() AS materials,"
            "  uniqExact(material_group) AS material_groups,"
            "  round(avg(net_weight), 3) AS avg_net_weight,"
            "  countIf(net_weight = 0 OR net_weight IS NULL) AS without_weight "
            f"FROM {SILVER} FINAL "
            "GROUP BY material_type"
        ),
    ),
    Model(
        name="gold_material_created_monthly",
        target="serving.mart_material_created_monthly",
        engine="MergeTree ORDER BY month",
        select=(
            "SELECT "
            # `assumeNotNull` is safe ONLY because of the `WHERE created_at
            # IS NOT NULL` below, and is needed because `month` is this
            # table's sorting key: `toStartOfMonth(Nullable)` is itself
            # nullable, which `MergeTree` refuses as a key.
            "  toStartOfMonth(assumeNotNull(created_at)) AS month,"
            "  count() AS materials_created,"
            "  uniqExact(created_by) AS creators "
            f"FROM {SILVER} FINAL "
            "WHERE created_at IS NOT NULL "
            "GROUP BY month"
        ),
    ),
    Model(
        name="gold_material_by_group",
        target="serving.mart_material_by_group",
        engine="MergeTree ORDER BY material_group",
        select=(
            "SELECT "
            "  coalesce(material_group, '(kosong)') AS material_group,"
            "  count() AS materials,"
            "  uniqExact(base_unit) AS base_units,"
            "  max(changed_at) AS last_changed_at "
            f"FROM {SILVER} FINAL "
            "GROUP BY material_group"
        ),
    ),
]


@op
def build_sap_models(context) -> dict:
    ensure_catalog_database(ch_target(), CATALOG_DB)
    counts = run_models(MODELS, log=context.log)
    context.add_output_metadata({f"{name}_rows": rows for name, rows in counts.items()})
    return counts


@job
def sap_transform_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `sap_transform_job`. One job for
    the whole SAP chain above Bronze: Silver first, then the three Gold
    marts that read it — order is the list's own (see `run_models`)."""
    build_sap_models()
