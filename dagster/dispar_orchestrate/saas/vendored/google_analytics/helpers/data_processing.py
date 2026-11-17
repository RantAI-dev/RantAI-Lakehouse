"""
This module contains helpers that process data and make it ready for loading into the database
"""

from typing import Any, Iterator, List, Union
from dlt.common.pendulum import pendulum
from dlt.common.typing import DictStrAny, TDataItem, TDataItems


def get_metadata(session: Any, property_id: int) -> DictStrAny:
    """
    The dimensions and metrics a property accepts (`GET /v1beta/properties/<id>/metadata`).

    SRC-3 V-GA4.1 (local change 1): the Data API's REST endpoint read through the
    connector's `SaasSession`, in place of upstream's `BetaAnalyticsDataClient.get_metadata`.
    """
    return session.get_json(f"/v1beta/properties/{int(property_id)}/metadata")


def get_report(
    session: Any,
    property_id: int,
    dimension_list: List[str],
    metric_list: List[str],
    limit: int,
    start_date: str,
    end_date: str,
) -> Iterator[TDataItem]:
    """
    Gets all the possible pages of reports with the given query parameters.
    Processes every page and yields a dictionary for every row of the report.

    Args:
        session: The `SaasSession` of the connector. Upstream took a gRPC
            `BetaAnalyticsDataClient`, whose HTTP/2 channel resolves and dials the host
            itself, outside the address and rate-limit rules of `SaasSession` (SRC-3 AD3, AD4).
        property_id: A reference to the Google Analytics project.
            More info: https://developers.google.com/analytics/devguides/reporting/data/v1/property-id
        dimension_list: The names of all the dimensions requested in the query.
        metric_list: The names of all the metrics requested in the query.
        limit: Describes how many rows there should be per page.
        start_date: The starting date of the query.
        end_date: The ending date of the query.

    Yields:
        Generator of all rows of data in the report.
    """

    # loop through all the pages
    # the total number of rows is received after the first request, for the first request to be sent through, initializing the row_count to 1 would suffice
    offset = 0
    row_count = 1
    limit = limit
    while offset < row_count:
        # make request to get the particular page. `offset` and `limit` are int64, which
        # the JSON mapping writes as text (runReport reference).
        body = {
            "dimensions": [{"name": name} for name in dimension_list],
            "metrics": [{"name": name} for name in metric_list],
            "dateRanges": [{"startDate": start_date, "endDate": end_date}],
            "offset": str(offset),
            "limit": str(limit),
        }
        # process request
        response = session.post_json(f"/v1beta/properties/{int(property_id)}:runReport", body)
        processed_response_generator = process_report(response=response)
        yield from processed_response_generator
        # update
        row_count = int(response.get("rowCount") or 0)
        offset += limit


def process_report(response: DictStrAny) -> Iterator[TDataItems]:
    """
    Receives a single page for a report response, processes it, and returns a generator for every row of data in the report page.

    Args:
        response: The API response for a single page of the report (the REST JSON form).

    Yields:
        Generator of dictionaries for every row of the report page.
    """

    metric_headers = response.get("metricHeaders") or []
    metrics_headers = [header["name"] for header in metric_headers]
    dimensions_headers = [header["name"] for header in response.get("dimensionHeaders") or []]
    for row in response.get("rows") or []:
        response_dict: DictStrAny = {
            dimension_header: _resolve_dimension_value(
                dimension_header, dimension_value["value"]
            )
            for dimension_header, dimension_value in zip(
                dimensions_headers, row.get("dimensionValues") or []
            )
        }
        for i in range(len(metrics_headers)):
            # get metric type and process the value depending on type. Save metric name including type as well for the columns
            metric_type = metric_headers[i].get("type", "METRIC_TYPE_UNSPECIFIED")
            metric_value = process_metric_value(
                metric_type=metric_type, value=row["metricValues"][i]["value"]
            )
            response_dict[
                f"{metrics_headers[i]}_{metric_type.split('_')[1]}"
            ] = metric_value
        yield response_dict


def process_metric_value(metric_type: str, value: str) -> Union[str, int, float]:
    """
    Processes the metric type, converts it from string to the correct type, and returns it.

    Args:
        metric_type: The type of the metric, as the REST API names it (`TYPE_INTEGER`, ...).
        value: The value of the metric as a string.

    Returns:
        The given value converted to the correct data type.
    """

    # So far according to GA4 documentation these are the correct types: https://developers.google.com/analytics/devguides/reporting/data/v1/rest/v1beta/MetricType
    # METRIC_TYPE_UNSPECIFIED is a string, TYPE_INTEGER an int, all the others floating points.
    if metric_type == "METRIC_TYPE_UNSPECIFIED":
        return value
    elif metric_type == "TYPE_INTEGER":
        return int(value)
    else:
        return float(value)


def _resolve_dimension_value(dimension_name: str, dimension_value: str) -> Any:
    """
    Helper function that receives a dimension's name and value and converts it to a datetime object if needed.

    Args:
        dimension_name: Name of the dimension.
        dimension_value: Value of the dimension.

    Returns:
        The value of the dimension with the correct data type.
    """

    if dimension_name == "date":
        return pendulum.from_format(dimension_value, "YYYYMMDD", tz="UTC")
    elif dimension_name == "dateHour":
        return pendulum.from_format(dimension_value, "YYYYMMDDHH", tz="UTC")
    elif dimension_name == "dateHourMinute":
        return pendulum.from_format(dimension_value, "YYYYMMDDHHmm", tz="UTC")
    elif dimension_name == "yearMonth":
        return pendulum.from_format(dimension_value, "YYYYMM", tz="UTC")
    elif dimension_name == "isoYearIsoWeek":
        # GA4 returns isoYearIsoWeek in format "202401"
        year = int(dimension_value[:4])
        week = int(dimension_value[4:])
        return pendulum.parse(f"{year}-W{week:02d}", strict=False, tz="UTC")
    elif dimension_name == "year":
        return pendulum.from_format(dimension_value, "YYYY", tz="UTC")
    else:
        return dimension_value
