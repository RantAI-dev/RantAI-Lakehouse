"""Google Analytics 4 (`SRC-3` V-GA4.2; decisions 2, 5, 6, 7, 9). The vendor module of
the `saas` adapter for `vendor = "ga4"`; the code that talks to the GA4 Data API is the
vendored dlt verified source (`saas/vendored/google_analytics/`, AD4), which calls the
API's REST endpoints (`runReport`, `getMetadata`) through the `SaasSession` the adapter
hands in (plan section 8 item 1: the Google client is not used).

# Objects are reports the person defines (decisions 2 and 5)

GA4 has no list of "tables": a report is a choice of dimensions and metrics. The
connector's `objects` (the dial) and a source object's `name` hold one report each, in
this form, which is data this module reads and never an address:

    traffic_by_country = country, date | sessions, activeUsers

The text before `=` is a label (letters, digits, underscore), the names before `|` are
dimensions and after it metrics, written as the Data API names them (`country`,
`sessionDefaultChannelGroup`, `customEvent:plan`). Names are checked against the
property's own `getMetadata` before a load (`describe`, decision 9), so a typo or a
custom dimension the property does not have is a plain message, and the schema of the
report is known before a row is read. Raw events (the BigQuery export Databricks reads)
are not offered (decision 2).

# Incremental by the date dimension (decision 6)

The report's time dimension (`date`, or the first of `dateHour`, `yearMonth`, ... the
definition has; `date` is added when it has none) is the cursor and the dimensions
together are the merge key. A later run starts `LOOKBACK_DAYS` days before the last
value (vendored local change 2: Google may change a day for 24-48 hours; the object's
`lag_seconds`, which dlt's incremental subtracts from the last value and for which it
switches its boundary de-duplication off) and the merge replaces the rows read again. A report always has a cursor, so none is "re-read in full".

# Deletes (decision 7)

GA4 reports have no deleted records; no object says `deletes`.

# Sign-in (decision 3)

Google browser sign-in (refresh token, `client_secret_post`), or a service-account key
(a JWT-bearer token exchange, `integration_auth` below). See the vendor table row.
"""

from __future__ import annotations

import re
from collections.abc import Iterator
from types import SimpleNamespace
from typing import Any

from dispar_orchestrate.adapters.saas import Field, SaasSetupRefused
from dispar_orchestrate.saas.objects import ObjectDecl
from dispar_orchestrate.saas.vendored import google_analytics as ga_source
from dispar_orchestrate.saas.vendored.google_analytics import helpers as ga_helpers
from dispar_orchestrate.saas.vendored.google_analytics.helpers.data_processing import get_metadata
from dispar_orchestrate.saas.vendored.google_analytics.settings import LOOKBACK_DAYS, START_DATE

ROWS_PER_PAGE = 10_000
_LABEL = re.compile(r"^[A-Za-z][A-Za-z0-9_]{0,63}$")
# A Data API name: letters, digits, underscore, with the one `customEvent:name` form.
_API_NAME = re.compile(r"^[A-Za-z][A-Za-z0-9_]*(:[A-Za-z0-9_]+)?$")
_MAX_NAMES = 20


def _refuse() -> SaasSetupRefused:
    return SaasSetupRefused("a Google Analytics report is written as: label = dimension, dimension | metric, metric")


def parse_report(text: str) -> tuple[str, list[str], list[str]]:
    """`(label, dimensions, metrics)` of a report definition, the time dimension
    included (added as `date` when the definition has none).

    # Errors

    `SaasSetupRefused` for anything that is not the form above.
    """
    label, eq, rest = text.partition("=")
    dims_text, bar, metrics_text = rest.partition("|")
    if not eq or not bar or not _LABEL.match(label.strip()):
        raise _refuse()
    dims = [d.strip() for d in dims_text.split(",") if d.strip()]
    metrics = [m.strip() for m in metrics_text.split(",") if m.strip()]
    names = [*dims, *metrics]
    if not metrics or len(names) > _MAX_NAMES or len(set(dims)) != len(dims) or len(set(metrics)) != len(metrics):
        raise _refuse()
    if not all(_API_NAME.match(n) for n in names):
        raise _refuse()
    if not any(d in ga_source.TIME_DIMENSIONS for d in dims):
        dims.append("date")
    return label.strip(), dims, metrics


def time_dimension(dims: list[str]) -> str:
    return next(d for d in dims if d in ga_source.TIME_DIMENSIONS)


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """The report's declaration: the time dimension is the cursor, the dimensions the key."""
    _label, dims, _metrics = parse_report(name)
    return ObjectDecl(name=name, primary_key=tuple(dims), cursor=time_dimension(dims), lag_seconds=LOOKBACK_DAYS * 86_400)


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    """The reports the connector defines (`dial.objects`): GA4 cannot list them."""
    return [declaration(session, parts, dial, name) for name in dial.get("objects") or []]


def describe(session: Any, parts: Any, dial: dict, name: str) -> list[Field] | None:
    """The report's columns as the property's metadata types them, before the load
    (decision 9). A dimension or metric the property does not have is a rejected setting."""
    _label, dims, metrics = parse_report(name)
    meta = get_metadata(session, int(parts.account))
    known_dims = {d["apiName"] for d in meta.get("dimensions") or []}
    known_metrics = {m["apiName"]: m.get("type", "METRIC_TYPE_UNSPECIFIED") for m in meta.get("metrics") or []}
    if not set(dims) <= known_dims or not set(metrics) <= set(known_metrics):
        raise SaasSetupRefused("this property has no dimension or metric by one of those names")
    fields = [Field(d, "timestamp" if d in ga_source.TIME_DIMENSIONS else "text", nullable=False) for d in dims]
    for metric in metrics:
        kind = known_metrics[metric]
        # The columns are named and typed as the verified source names them: `<metric>_<TYPE>`.
        fields.append(Field(f"{metric}_{kind.split('_')[1]}", {"TYPE_INTEGER": "bigint", "METRIC_TYPE_UNSPECIFIED": "text"}.get(kind, "double")))
    return fields


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any) -> Iterator[dict[str, Any]]:
    """The rows of a report from `LOOKBACK_DAYS` before `since`, or from the start of
    Google Analytics' data for a first run (`since` is `None`)."""
    label, dims, metrics = parse_report(decl.name)
    cursor = time_dimension(dims)
    yield from ga_helpers.basic_report(
        session,
        ROWS_PER_PAGE,
        dims,
        metrics,
        int(parts.account),
        label,
        START_DATE,
        SimpleNamespace(last_value=since, cursor_path=cursor),
        # `since` already is the cursor minus the lag (`ObjectDecl.lag_seconds`, dlt's own);
        # subtracting the lookback here too would read it twice.
        lookback_days=0,
    )
