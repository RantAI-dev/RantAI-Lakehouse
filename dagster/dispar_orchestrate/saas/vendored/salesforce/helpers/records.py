"""Salesforce source helpers"""

import re

import pendulum

from typing import Optional, Iterable, Dict, Set, Any, cast

from dlt.common.typing import TDataItem


from ..settings import API_VERSION, IS_PRODUCTION

# SRC-3 V-Salesforce.1 (local change 1): an sObject or field name is spliced into
# the SOQL text, so it must be a plain API name (letters, digits, underscores).
_API_NAME = re.compile(r"^[A-Za-z][A-Za-z0-9_]*$")
# A GET carries the whole query in its address; Salesforce refuses an address of
# more than 16,384 bytes. Stay well under it (the host and the headers count too).
_MAX_QUERY_CHARS = 12_000


class QueryTooLongError(Exception):
    """An sObject with so many fields that its query does not fit one request."""

    def __init__(self) -> None:
        super().__init__("this Salesforce object has too many fields to read in one query")


def _process_record(
    record: Dict[str, Any], date_fields: Set[str], api_type: str = "standard"
) -> Dict[str, Any]:
    """
    Process a single Salesforce record by removing attributes and converting date fields.

    Args:
        record: The record to process
        date_fields: Set of field names that contain date/datetime values
        api_type: Whether this record came from Bulk API (timestamps), or standard API (ISO strings)

    Returns:
        The processed record
    """
    # Strip out the attributes field
    record.pop("attributes", None)

    for field in date_fields:
        if record.get(field):
            if api_type == "bulk":
                # Bulk API returns timestamps, convert to ISO 8601
                record[field] = pendulum.from_timestamp(
                    record[field] / 1000,
                ).strftime("%Y-%m-%dT%H:%M:%S.%fZ")
            else:
                # Standard API returns ISO strings, ensure consistent format
                if record[field]:
                    # pendulum.parse can also return Date, Time or Duration
                    # Salesforce date/datetime fields are always DateTime
                    dt = cast(pendulum.DateTime, pendulum.parse(record[field]))
                    record[field] = dt.strftime("%Y-%m-%dT%H:%M:%S.%fZ")

    return record


def describe_sobject(session: Any, sobject: str) -> Dict[str, Any]:
    """The sObject's describe result (`GET .../sobjects/<name>/describe`)."""
    if not _API_NAME.match(sobject):
        raise ValueError("not a Salesforce object name")
    return session.get_json(f"/services/data/v{API_VERSION}/sobjects/{sobject}/describe")


def queried_fields(desc: Dict[str, Any]) -> list:
    """The fields of a describe result that a read selects (the vendor module's
    `describe` answers the same list, so the schema shown before a load is the one loaded).

    Salesforce returns compound fields as separate fields, so the compound parent is
    left out (but `Name`). If no fields are specified, every other field is used,
    except a `base64` one (a file's body): Salesforce returns it as an address of its
    own, one record per batch, never inline.
    """
    compound_fields = {
        f["compoundFieldName"]
        for f in desc["fields"]
        if f["compoundFieldName"] is not None
    } - {"Name"}
    return [
        f
        for f in desc["fields"]
        if f["name"] not in compound_fields and f["type"] != "base64"
    ]


def get_records(
    session: Any,
    sobject: str,
    last_state: Optional[str] = None,
    replication_key: Optional[str] = None,
    include_deleted: bool = False,
) -> Iterable[TDataItem]:
    """
    Retrieves records from Salesforce for a specified sObject.

    Args:
        session: The `SaasSession` of the connector. Upstream took a `simple_salesforce`
            client; that library (and its Bulk API) dials the host itself, outside the
            link, address and rate-limit rules of `SaasSession` (SRC-3 AD3, AD4).
        sobject (str): The name of the sObject to retrieve records from.
        last_state (str, optional): The last known state for incremental loading. Defaults to None.
        replication_key (str, optional): The replication key for incremental loading. Defaults to None.
        include_deleted (bool): Read through `queryAll`, which also returns the records in the
            recycle bin with `IsDeleted = true` (SRC-3 decision 7, local change 3).

    Yields:
        Dict[TDataItem]: A dictionary representing a record from the Salesforce sObject.
    """
    # Get all fields for the sobject
    desc = describe_sobject(session, sobject)
    fields = [f["name"] for f in queried_fields(desc)]

    # Fields of type "datetime" (Standard API returns ISO strings).
    # We normalize them later.
    date_fields = {
        f["name"] for f in desc["fields"] if f["type"] in ("datetime",) and f["name"]
    }

    # Generate a predicate to filter records by the replication key
    predicate, order_by, n_records = "", "", 0
    if replication_key:
        if not _API_NAME.match(replication_key):
            raise ValueError("not a Salesforce field name")
        if last_state:
            # `>=`, not upstream's `>`: Salesforce stamps a millisecond, so another
            # record may carry the very time of the last one read; the merge on `Id`
            # removes the repeat of the cursor's own record (local change 3).
            # The value is parsed and written back in Salesforce's own literal form:
            # nothing the caller passed is spliced into the query as it came.
            literal = cast(pendulum.DateTime, pendulum.parse(str(last_state))).in_timezone("UTC")
            predicate = f"WHERE {replication_key} >= {literal.strftime('%Y-%m-%dT%H:%M:%S.%f')[:-3]}Z"
        order_by = f"ORDER BY {replication_key} ASC"
    query = f"SELECT {', '.join(fields)} FROM {sobject} {predicate} {order_by}"
    if not IS_PRODUCTION:
        query += " LIMIT 100"
    if len(query) > _MAX_QUERY_CHARS:
        raise QueryTooLongError()

    # Standard REST query. The next page is `nextRecordsUrl`, a path on the same
    # host, which the session follows under its same-origin rule.
    endpoint = "queryAll" if include_deleted else "query"
    result = session.get_json(f"/services/data/v{API_VERSION}/{endpoint}", {"q": query})
    while True:
        for record in result["records"]:
            processed_record = _process_record(record, date_fields, api_type="standard")
            yield processed_record
            n_records += 1

        # Check if there are more records to fetch
        if result["done"]:
            break
        result = session.get_json(result["nextRecordsUrl"])
