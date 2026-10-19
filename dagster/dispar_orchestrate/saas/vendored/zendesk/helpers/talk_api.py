from enum import Enum
from typing import Any, Dict, Iterator, Optional
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
    ) -> Iterator[TDataItems]:
        """
        Makes a request to a paginated endpoint and returns a generator of data items per page.

        Args:
            endpoint: The url to the endpoint, e.g. /api/v2/calls
            data_point_name: The key which data items are nested under in the response object (e.g. calls)
            params: Optional dict of query params to include in the request
            pagination: Type of pagination type used by endpoint

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

        # make request and keep looping until there is no next page
        get_url: Optional[str] = endpoint
        while get_url:
            response_json = self.session.get_json(get_url, params or None)
            result = response_json[data_point_name]
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

            params = {}
