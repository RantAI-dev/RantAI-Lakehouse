from enum import Enum
from typing import Any, Dict, Iterator, Optional, Tuple
from dlt.common.typing import TDataItems
from .. import settings

# SRC-3 AD3/AD4 (local change 2): upstream built the host from a subdomain, took
# credentials (OAuth token, API token or email and password) and fetched with
# `dlt.sources.helpers.requests`, following the `next_page` link of every
# answer unchecked. All of that is now `saas.session.SaasSession`: the host comes
# from the vendor table, the sign-in's headers are asked for only after the
# address check, a link is followed only on the same host, no redirect is
# followed and a 429 is waited out within a bound. The client holds the session.


class PaginationType(Enum):
    OFFSET = 0
    CURSOR = 1
    STREAM = 2
    START_TIME = 3
    # SRC-3 local change 3: the cursor-based incremental export (tickets, users).
    # The first request takes `start_time`; every later page and the next export
    # take the `after_url` of the page before.
    CURSOR_EXPORT = 4


# SRC-3 local change 3 (dlt-hub/verified-sources#659). The incremental exports a
# connector reads, each as (endpoint, key of the items in the answer, how it pages,
# the properties that identify one export row). A TIME-BASED export (`STREAM`) starts
# every page AT the end time of the page before, so items that share that time come
# twice; Zendesk's own rule for removing them is "filter out any items that share
# `id` + `updated_at`" (`id` + `created_at` for ticket events), see
# https://developer.zendesk.com/documentation/api-basics/working-with-data/using-the-incremental-export-api/
# ("Excluding duplicate items"). Tickets and users have the cursor-based export,
# which Zendesk recommends and which repeats nothing within an export.
INCREMENTAL_EXPORTS = {
    "tickets": ("/api/v2/incremental/tickets/cursor.json", "tickets", PaginationType.CURSOR_EXPORT, None),
    "users": ("/api/v2/incremental/users/cursor.json", "users", PaginationType.CURSOR_EXPORT, None),
    "ticket_events": ("/api/v2/incremental/ticket_events.json", "ticket_events", PaginationType.STREAM, ("id", "created_at")),
    "organizations": ("/api/v2/incremental/organizations.json", "organizations", PaginationType.STREAM, ("id", "updated_at")),
}


class ZendeskAPIClient:
    """
    API client used to make requests to Zendesk talk, support and chat API
    """

    session: Any

    def __init__(self, session: Any) -> None:
        """
        Args:
            session: the `SaasSession` of the connector. Endpoints are paths on its origin.
        """
        self.session = session

    def get_pages(
        self,
        endpoint: str,
        data_point_name: str,
        pagination: PaginationType,
        params: Optional[Dict[str, Any]] = None,
        distinct_by: Optional[Tuple[str, ...]] = None,
    ) -> Iterator[TDataItems]:
        """
        Makes a request to a paginated endpoint and returns a generator of data items per page.

        Args:
            endpoint: The url to the endpoint, e.g. /api/v2/calls
            data_point_name: The key which data items are nested under in the response object (e.g. calls)
            params: Optional dict of query params to include in the request
            pagination: Type of pagination type used by endpoint
            distinct_by: SRC-3 local change 3: properties that identify an item; an item
                whose values were already yielded in this call is left out of its page.

        Returns:
            Generator of pages, each page is a list of dict data items
        """
        # update the page size to enable cursor pagination
        params = params or {}
        if pagination == PaginationType.CURSOR:
            params["page[size]"] = settings.PAGE_SIZE
        elif pagination == PaginationType.STREAM:
            params["per_page"] = settings.INCREMENTAL_PAGE_SIZE
        elif pagination == PaginationType.START_TIME:
            params["limit"] = settings.INCREMENTAL_PAGE_SIZE
        elif pagination == PaginationType.CURSOR_EXPORT:
            params["per_page"] = settings.INCREMENTAL_PAGE_SIZE

        seen: set = set()
        # make request and keep looping until there is no next page
        get_url: Optional[str] = endpoint
        while get_url:
            response_json = self.session.get_json(get_url, params or None)
            result = response_json[data_point_name]
            if distinct_by:
                fresh = []
                for item in result:
                    key = tuple(item.get(name) for name in distinct_by)
                    if key not in seen:
                        seen.add(key)
                        fresh.append(item)
                result = fresh
            yield result

            get_url = None
            if pagination == PaginationType.CURSOR:
                if response_json["meta"]["has_more"]:
                    get_url = response_json["links"]["next"]
            elif pagination == PaginationType.OFFSET:
                get_url = response_json.get("next_page", None)
            elif pagination == PaginationType.STREAM:
                # See https://developer.zendesk.com/api-reference/ticketing/ticket-management/incremental_exports/#json-format
                if not response_json["end_of_stream"]:
                    get_url = response_json["next_page"]
            elif pagination == PaginationType.START_TIME:
                if response_json["count"] > 0:
                    get_url = response_json["next_page"]
            elif pagination == PaginationType.CURSOR_EXPORT:
                if not response_json["end_of_stream"]:
                    get_url = response_json.get("after_url")

            params = {}

    def get_incremental(self, resource: str, start_time: int) -> Iterator[TDataItems]:
        """Pages of `resource` (a key of `INCREMENTAL_EXPORTS`) created or changed at or after
        `start_time` (UNIX seconds), with no item twice in one call (SRC-3 local change 3).

        Items at exactly `start_time` are exported again by the next call: the export is
        inclusive and a caller that remembers its last `updated_at` asks from it.
        """
        endpoint, key, pagination, distinct_by = INCREMENTAL_EXPORTS[resource]
        return self.get_pages(endpoint, key, pagination, params={"start_time": start_time}, distinct_by=distinct_by)
