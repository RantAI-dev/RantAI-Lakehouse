"""The OAuth 2.0 client-credentials grant of a `saas` vendor's integration credential
(`SRC-3` decision 3, second way): ServiceNow, Dynamics 365 and SharePoint.

# Why one function for three vendors (CODE-STANDARD: one helper, not three copies)

ServiceNow (slice 4) wrote this exchange; the two Microsoft vendors (slice 5) need the same
grant with one more field, `scope` (Entra: `https://graph.microsoft.com/.default`, "of all the direct
application permissions you have configured ... the ones associated with the resource",
https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow). The vendor table
gives the token address (for Entra it holds the customer's tenant, a validated part of the account, AD2)
and, in `integration.scope`, the scope; ServiceNow's row has none and sends none, exactly as before.

The token is asked for after the address check and pinning that `rest._token_request` applies, kept until
shortly before it expires, and forgotten after a 401. Entra sends no refresh token for this grant, so there
is nothing to rotate or report.
"""

from __future__ import annotations

import time
from collections.abc import Callable
from typing import Any

from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.saas import vendors

_EXPIRY_MARGIN_SECONDS = 60


def integration_auth(
    vendor: Any,
    secrets: dict[str, str],
    *,
    resolve_checked: Callable[..., Any],
    parts: Any,
    dial: dict | None = None,
    clock: Callable[[], float] | None = None,
) -> tuple[Callable[[], dict], Callable[[], None]]:
    """`(headers, refresh)` for the client-credentials grant: a bearer token fetched when
    first needed, again once it has expired, and again on demand after a 401.

    The token is asked for at the vendor table's address for this account (`rebase_for_gate`
    only moves it to the gate's mock under the gate-only switch).

    # Errors

    `OAuthSignInRefused` (at the first request) when the vendor refuses the exchange.
    """
    clock = clock or (lambda: time.time())  # looked up at the call: a test moves the time
    token_url = vendors.rebase_for_gate(vendor.id, vendor.token_url(parts))
    form = {"grant_type": "client_credentials", "client_id": secrets["clientId"], "client_secret": secrets["clientSecret"]}
    scope = vendor.integration_scope(parts)
    if scope:
        form["scope"] = scope
    state: dict[str, Any] = {"headers": None, "expires_at": 0.0}

    def headers() -> dict:
        if state["headers"] is None or clock() >= state["expires_at"]:
            body = rest_adapter._token_request(token_url, form, resolve_checked=resolve_checked)
            lifetime = body.get("expires_in")
            valid = isinstance(lifetime, (int, float)) and not isinstance(lifetime, bool) and lifetime > 0
            # No stated lifetime: replaced only after a 401 (as `rest.AuthSession` does).
            state["expires_at"] = clock() + max(lifetime - _EXPIRY_MARGIN_SECONDS, 0) if valid else float("inf")
            state["headers"] = {"Authorization": f"Bearer {body['access_token']}"}
        return state["headers"]

    def refresh() -> None:
        state["headers"] = None

    return headers, refresh
