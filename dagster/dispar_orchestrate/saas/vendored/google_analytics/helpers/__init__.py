"""Google analytics source helpers"""

from typing import Any, Iterator, List
from pendulum.datetime import DateTime

import dlt
from dlt.common import logger, pendulum
from dlt.common.typing import TDataItem

from ..settings import LOOKBACK_DAYS, START_DATE
from .data_processing import get_report


def report_start_date(
    time_dimension: str, last_value: Any, start_date: str, lookback_days: int = LOOKBACK_DAYS
) -> str:
    """
    The first day (`YYYY-MM-DD`) a report is asked for.

    First run (no `last_value`): `start_date`. Later runs: `LOOKBACK_DAYS` days before the
    last value read, moved back to the first day of its week, month or year when the
    report's time dimension is that coarse, because Analytics sums a partial period over
    only the days asked for and the merge would then replace a whole month with a part of it.
    (SRC-3 V-GA4.1 local change 2. Upstream started the day AFTER the last value, so
    the last days, which Google may still change for 24 to 48 hours, were never read again.)
    """
    if not last_value:
        return start_date or START_DATE
    floor = pendulum.parse(last_value) if isinstance(last_value, str) else pendulum.instance(last_value)
    floor = floor.subtract(days=lookback_days)
    unit = {"isoYearIsoWeek": "week", "yearMonth": "month", "year": "year"}.get(time_dimension)
    if unit:
        floor = floor.start_of(unit)
    return floor.to_date_string()


def basic_report(
    session: Any,
    rows_per_page: int,
    dimensions: List[str],
    metrics: List[str],
    property_id: int,
    resource_name: str,
    start_date: str,
    last_date: dlt.sources.incremental[DateTime],
    lookback_days: int = LOOKBACK_DAYS,
) -> Iterator[TDataItem]:
    """
    Retrieves the data for a report given dimensions, metrics, and filters required for the report.

    Args:
        session: The `SaasSession` of the connector, used to make requests.
        dimensions: Dimensions for the report. See metadata for the full list of dimensions.
        metrics: Metrics for the report. See metadata for the full list of metrics.
        property_id: A reference to the Google Analytics project.
            More info: https://developers.google.com/analytics/devguides/reporting/data/v1/property-id
        rows_per_page: Controls how many rows are retrieved per page in the reports.
            Default is 10000, maximum possible is 100000.
        resource_name: The resource name used to save incremental into dlt state.
        start_date: Incremental load start_date.
            Default is taken from dlt state if it exists.
        last_date: Incremental load end date.
            Default is taken from dlt state if it exists.
        lookback_days: Days re-read before the last value (local change 2). Zero when the
            caller's own incremental already carries the lag (the connector's `ObjectDecl.lag_seconds`).

    Returns:
        Generator of all rows of data in the report.
    """

    # grab the start time from last dlt load if not filled, if that is also empty then use the first day of the millennium as the start time instead
    if last_date.last_value:
        if start_date != START_DATE:
            logger.warning(
                f"Using the starting date: {last_date.last_value} for incremental report: {resource_name} and ignoring start date passed as argument {start_date}"
            )
    start_date = report_start_date(
        last_date.cursor_path, last_date.last_value, start_date, lookback_days
    )
    # SHOULD-FIX 11 (slice 3 review): the end is the relative term `yesterday`, which the Data
    # API infers in the PROPERTY's reporting time zone ("the date is inferred based on the
    # property's reporting time zone", DateRange reference), where upstream computed the date
    # in UTC. The dates in the rows are in that zone too, so the cursor and the end agree.
    end_date = "yesterday"
    if start_date > pendulum.now("UTC").add(days=1).to_date_string():
        # Later than "yesterday" in every time zone (a configured start in the future):
        # Google answers such a range with an error, so no request is made.
        return

    processed_response = get_report(
        session=session,
        property_id=property_id,
        dimension_list=list(dimensions),
        metric_list=list(metrics),
        limit=rows_per_page,
        start_date=start_date,
        # configure end_date to yesterday as a date string
        end_date=end_date,
    )
    yield from processed_response
