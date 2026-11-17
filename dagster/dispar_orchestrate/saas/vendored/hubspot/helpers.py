"""Hubspot source helpers"""

from typing import Union

from typing import Any, Dict, Iterator, List, Optional

from dlt.common.schema.typing import TColumnSchema

from datetime import datetime, timezone

from .settings import (
    CRM_BATCH_READ_ENDPOINT,
    CRM_SEARCH_ENDPOINT,
    HS_TO_DLT_TYPE,
    OBJECT_TYPE_PLURAL,
    SEARCH_MAX_PROPS_LENGTH,
    SEARCH_MAX_RESULTS,
    SEARCH_PAGE_LIMIT,
)
from .utils import chunk_properties

# SRC-3 AD3/AD4 (local change 1): upstream called `dlt.sources.helpers.requests`
# with `BASE_URL`, a bearer header built from an API key, and the next-page link
# taken from the answer. All of that is now `saas.session.SaasSession`: it
# builds the host from the vendor table, asks the connector's sign-in for the
# headers only after the address check, follows a link only on the same host,
# and waits out a 429. The `session` argument below is that object (upstream
# passed an `api_key` string); endpoints stay paths on the session's origin.


def pagination(_data: Dict[str, Any], session: Any) -> Optional[Dict[str, Any]]:
    _next = _data.get("paging", {}).get("next", None)
    # _next = False
    if _next:
        next_url = _next["link"]
        # Get the next page response (the session refuses a link to another host)
        return session.get_json(next_url)  # type: ignore
    else:
        return None


def extract_association_data(
    _obj: Dict[str, Any],
    data: Dict[str, Any],
    association: str,
    session: Any,
) -> List[Dict[str, Any]]:
    values = []

    while data is not None:
        for r in data["results"]:
            values.append(
                {
                    "value": _obj["hs_object_id"],
                    f"{association}_id": r["id"],
                }
            )
        data = pagination(data, session)
    return values


def extract_property_history(objects: List[Dict[str, Any]]) -> Iterator[Dict[str, Any]]:
    for item in objects:
        history = item.get("propertiesWithHistory")
        if not history:
            return
        # Yield a flat list of property history entries
        for key, changes in history.items():
            if not changes:
                continue
            for entry in changes:
                yield {"object_id": item["id"], "property_name": key, **entry}


def fetch_property_history(
    endpoint: str,
    session: Any,
    props: str,
    params: Optional[Dict[str, Any]] = None,
) -> Iterator[List[Dict[str, Any]]]:
    """Fetch property history from the given CRM endpoint.

    Args:
        endpoint: The endpoint to fetch data from, as a string.
        session: The `SaasSession` to read through.
        props: A comma separated list of properties to retrieve the history for
        params: Optional dict of query params to include in the request

    Yields:
         List of property history entries (dicts)
    """
    params = dict(params or {})
    params["propertiesWithHistory"] = props
    params["limit"] = 50
    # Make the API request and parse the response JSON data
    _data = session.get_json(endpoint, params)
    while _data is not None:
        if "results" in _data:
            yield list(extract_property_history(_data["results"]))

        # Follow pagination links if they exist
        _next = _data.get("paging", {}).get("next", None)
        if _next:
            next_url = _next["link"]
            # Get the next page response
            _data = session.get_json(next_url)
        else:
            _data = None


def fetch_data(
    endpoint: str,
    session: Any,
    params: Optional[Dict[str, Any]] = None,
    context: Optional[Dict[str, Any]] = None,
) -> Iterator[List[Dict[str, Any]]]:
    """
    Fetch data from HUBSPOT endpoint using a specified API key and yield the properties of each result.
    For paginated endpoint this function yields item from all pages.

    Args:
        endpoint (str): The endpoint to fetch data from, as a string.
        session (SaasSession): The session to read through.
        params: Optional dict of query params to include in the request.
        context (Optional[Dict[str, Any]]): Additional data which need to be added in the resulting page.

    Yields:
        A List of CRM object dicts

    Raises:
        The session's failures; see Notes.

    Notes:
        The request is `session.get_json(endpoint, params)`: the session raises its own failures
        (sign-in refused, permission missing, rate limited) or `requests.HTTPError` for another
        error status. The `endpoint` argument is a path on the session's origin.
    """
    # Make the API request and parse the response JSON data
    _data = session.get_json(endpoint, params)
    # Yield the properties of each result in the API response
    while _data is not None:
        if "results" in _data:
            _objects: List[Dict[str, Any]] = []
            for _result in _data["results"]:
                _obj = _result.get("properties", _result)
                if "id" not in _obj and "id" in _result:
                    # Move id from properties to top level
                    _obj["id"] = _result["id"]
                if "associations" in _result:
                    for association in _result["associations"]:
                        __data = _result["associations"][association]

                        __values = extract_association_data(
                            _obj, __data, association, session
                        )

                        # remove duplicates from list of dicts
                        __values = [
                            dict(t) for t in {tuple(d.items()) for d in __values}
                        ]

                        _obj[association] = __values
                if context:
                    _obj.update(context)
                _objects.append(_obj)
            yield _objects

        # Follow pagination links if they exist
        _data = pagination(_data, session)


class SearchOutOfBoundsException(Exception):
    """More than `SEARCH_MAX_RESULTS` records share one modification time, so
    the search cannot move past them. The caller reads the object in full."""


def _to_ms(value: Any) -> int:
    """A HubSpot date value (ISO 8601 text, or milliseconds as text or number) as
    UTC epoch milliseconds."""
    if isinstance(value, (int, float)) or (isinstance(value, str) and value.isdigit()):
        return int(value)
    return int(datetime.fromisoformat(str(value).replace("Z", "+00:00")).astimezone(timezone.utc).timestamp() * 1000)


def _flatten(result: Dict[str, Any], context: Optional[Dict[str, Any]]) -> Dict[str, Any]:
    """The same shape `fetch_data` gives a record: the properties, plus `id`."""
    _obj = dict(result.get("properties", result))
    if "id" not in _obj and "id" in result:
        _obj["id"] = result["id"]
    if context:
        _obj.update(context)
    return _obj


def search_data_since(
    endpoint: str,
    session: Any,
    last_modified_prop: str,
    since_ms: int,
    props: List[str],
    context: Optional[Dict[str, Any]] = None,
) -> Iterator[List[Dict[str, Any]]]:
    """Records of a CRM object modified at or after `since_ms`, through the CRM
    search API (SRC-3 local change 2: upstream read every record on every run,
    upstream PR #662 is the open attempt at this).

    A search is a POST of a filter (`last_modified_prop >= since`), sorted by that
    property ascending, 100 results a page. Three documented limits are handled,
    not assumed away:

    * a query returns at most `SEARCH_MAX_RESULTS`; when the pages run out at
      the limit, the search restarts from the newest modification time seen
      (the records at that instant come again; the merge key removes them);
    * if the limit is reached and no newer time was seen, 10,000 records share
      one instant and the search cannot move: `SearchOutOfBoundsException`;
    * the request body may hold 3,000 characters, so the properties that do not
      fit in the first request are read with the batch-read endpoint for the
      ids of each page, then merged into the record.

    Archived records do not appear in search results (HubSpot's search
    documentation), so this path reports no deletes.

    Raises:
        SearchOutOfBoundsException: see above.
    """
    first, *later = _search_property_chunks(props, last_modified_prop)
    url = CRM_SEARCH_ENDPOINT.format(crm_endpoint=endpoint)
    while True:
        body: Dict[str, Any] = {
            "properties": first,
            "limit": SEARCH_PAGE_LIMIT,
            "filterGroups": [
                {"filters": [{"propertyName": last_modified_prop, "operator": "GTE", "value": str(since_ms)}]}
            ],
            "sorts": [{"propertyName": last_modified_prop, "direction": "ASCENDING"}],
        }
        newest = since_ms
        capped = False
        while True:
            data = session.post_json(url, body)
            results = data.get("results", [])
            if results:
                objects = [_flatten(r, context) for r in results]
                for obj in objects:
                    if obj.get(last_modified_prop):
                        newest = max(newest, _to_ms(obj[last_modified_prop]))
                for chunk in later:
                    extra = _batch_read(endpoint, session, [str(o["id"]) for o in objects], chunk)
                    for obj in objects:
                        obj.update(extra.get(str(obj["id"]), {}))
                yield objects
            after = data.get("paging", {}).get("next", {}).get("after")
            if not after:
                return
            if int(after) >= SEARCH_MAX_RESULTS:
                capped = True
                break
            body["after"] = after
        if capped:
            if newest <= since_ms:
                raise SearchOutOfBoundsException()
            since_ms = newest


def _search_property_chunks(props: List[str], last_modified_prop: str) -> List[List[str]]:
    """The properties as request-sized chunks; the first holds the cursor
    property, which every record must carry."""
    rest = sorted(p for p in set(props) if p != last_modified_prop)
    chunks = list(chunk_properties([last_modified_prop] + rest, SEARCH_MAX_PROPS_LENGTH))
    return chunks or [[last_modified_prop]]


def _batch_read(endpoint: str, session: Any, ids: List[str], props: List[str]) -> Dict[str, Dict[str, Any]]:
    """`props` of the records `ids` (at most 100), by id."""
    data = session.post_json(
        CRM_BATCH_READ_ENDPOINT.format(crm_endpoint=endpoint),
        {"properties": props, "inputs": [{"id": i} for i in ids]},
    )
    return {str(r["id"]): dict(r.get("properties", {})) for r in data.get("results", [])}


def _get_property_names_types(
    session: Any, object_type: str
) -> Dict[str, Union[str, None]]:
    """
    Retrieve property names and their types if present for a given entity from the HubSpot API.

    Args:
        entity: The entity name for which to retrieve property names.

    Returns:
        A dict of propery names and their types if present.

    Raises:
        Exception: If an error occurs during the API request.
    """
    props_to_type: Dict[str, str] = {}
    endpoint = f"/crm/v3/properties/{OBJECT_TYPE_PLURAL[object_type]}"

    for page in fetch_data(endpoint, session):
        for prop in page:
            props_to_type[prop["name"]] = prop.get("type", None)

    return props_to_type


def get_properties_labels(
    session: Any, object_type: str, property_name: str
) -> Iterator[Dict[str, Any]]:
    endpoint = f"/crm/v3/properties/{object_type}/{property_name}"
    _data: Optional[Dict[str, Any]] = session.get_json(endpoint)
    while _data is not None:
        yield _data
        _data = pagination(_data, session)


def _to_dlt_columns_schema(col: Dict[str, str]) -> TColumnSchema:
    """Converts hubspot column to dlt column schema that will be
    used as a column hint."""
    col_name, col_type = next(iter(col.items()))
    # NOTE: if col_type is not in HS_TO_DLT_TYPE, we return an empty dict.
    # Downstream, this means no column hints are provided for this property.
    return (
        {"name": col_name, "data_type": HS_TO_DLT_TYPE[col_type]}
        if col_type in HS_TO_DLT_TYPE
        else {}
    )
