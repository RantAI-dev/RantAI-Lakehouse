"""The vendor table and the registry of vendor modules (`SRC-3` AD1, AD2, AD5).

# One table in three places

`_TABLE` is a literal copy of `ops/fixtures/saas_vendors.json`, the file the
Rust table (`lakehouse-store/src/saas_vendors.rs`) and the console are also
compared with. It is a copy and not a read of the file because the
orchestrator's image does not carry `ops/`. `test_saas_vendors.py` compares the
two, so a vendor changed in one place and not the other fails a test.

# Hosts are built, never typed (AD2)

The user supplies only `account`; `validate_account` accepts it in the shape
the vendor's host pattern allows (the same rules as Rust's
`SaasVendor::validate_account`) and `render` builds every address from the
table. A vendor module never receives an address typed by a person.

# The registry

A vendor module (slices 2 to 6) is named in `MODULE_PATHS`; `module_for` imports
it on first use, so a vendor that has no module yet (and a vendor id nobody
registered) raises `UnsupportedVendor`, which the ingest classifies as
`unsupported`: honest, never a guess. `register(vendor_id, module)` is the
other way in, for a test's fake vendor; it takes precedence over the path.
"""

from __future__ import annotations

import importlib
import json
import os
from dataclasses import dataclass
from typing import Any

# ---- the table: a copy of ops/fixtures/saas_vendors.json ------------------------
_TABLE: list[dict[str, Any]] = [{'id': 'salesforce',
  'label': 'Salesforce',
  'connectorType': 'Salesforce',
  'account': {'kind': 'label'},
  'accountLabel': 'My Domain name (the part before .my.salesforce.com)',
  'apiBase': 'https://{account}.my.salesforce.com',
  'oauth': {'authorizeUrl': 'https://{account}.my.salesforce.com/services/oauth2/authorize',
            'tokenUrl': 'https://{account}.my.salesforce.com/services/oauth2/token',
            'revocationUrl': 'https://{account}.my.salesforce.com/services/oauth2/revoke',
            'scopes': ['api', 'refresh_token'],
            'clientAuth': 'post'},
  'integration': None,
  'whoami': {'method': 'GET', 'path': '/services/data/v62.0/limits'}},
 {'id': 'servicenow',
  'label': 'ServiceNow',
  'connectorType': 'ServiceNow',
  'account': {'kind': 'label'},
  'accountLabel': 'Instance name (the part before .service-now.com)',
  'apiBase': 'https://{account}.service-now.com',
  'oauth': {'authorizeUrl': 'https://{account}.service-now.com/oauth_auth.do',
            'tokenUrl': 'https://{account}.service-now.com/oauth_token.do',
            'revocationUrl': 'https://{account}.service-now.com/oauth_revoke_token.do',
            'scopes': [],
            'clientAuth': 'post'},
  'integration': {'secretKey': 'integration:servicenow',
                  'secretFields': ['clientId', 'clientSecret']},
  'whoami': {'method': 'GET',
             'path': '/api/now/table/sys_user?sysparm_limit=1&sysparm_fields=sys_id'}},
 {'id': 'workday',
  'label': 'Workday Reports',
  'connectorType': 'Workday Reports',
  'account': {'kind': 'host_tenant', 'suffixes': ['.workday.com', '.myworkday.com']},
  'accountLabel': 'Workday host and tenant, as host/tenant',
  'apiBase': 'https://{host}',
  'oauth': {'authorizeUrl': 'https://{host}/{tenant}/authorize',
            'tokenUrl': 'https://{host}/ccx/oauth2/{tenant}/token',
            'revocationUrl': None,
            'scopes': [],
            'clientAuth': 'post'},
  'integration': {'secretKey': 'integration:workday', 'secretFields': ['username', 'password']},
  'whoami': None},
 {'id': 'ga4',
  'label': 'Google Analytics 4',
  'connectorType': 'Google Analytics 4',
  'account': {'kind': 'digits'},
  'accountLabel': 'GA4 property id (digits)',
  'apiBase': 'https://analyticsdata.googleapis.com',
  'oauth': {'authorizeUrl': 'https://accounts.google.com/o/oauth2/v2/auth',
            'tokenUrl': 'https://oauth2.googleapis.com/token',
            'revocationUrl': 'https://oauth2.googleapis.com/revoke',
            'scopes': ['https://www.googleapis.com/auth/analytics.readonly'],
            'clientAuth': 'post'},
  'integration': {'secretKey': 'integration:ga4', 'secretFields': ['serviceAccountJson']},
  'whoami': {'method': 'GET', 'path': '/v1beta/properties/{account}/metadata'}},
 {'id': 'hubspot',
  'label': 'HubSpot',
  'connectorType': 'HubSpot',
  'account': {'kind': 'digits'},
  'accountLabel': 'HubSpot account id (digits)',
  'apiBase': 'https://api.hubapi.com',
  'oauth': {'authorizeUrl': 'https://app.hubspot.com/oauth/authorize',
            'tokenUrl': 'https://api.hubapi.com/oauth/v1/token',
            'revocationUrl': None,
            'scopes': ['crm.objects.companies.read',
                       'crm.objects.contacts.read',
                       'crm.objects.deals.read',
                       'crm.objects.owners.read',
                       'crm.objects.quotes.read',
                       'crm.schemas.companies.read',
                       'crm.schemas.contacts.read',
                       'crm.schemas.deals.read',
                       'crm.schemas.quotes.read',
                       'tickets',
                       'e-commerce',
                       'oauth'],
            'clientAuth': 'post'},
  'integration': {'secretKey': 'integration:hubspot', 'secretFields': ['token']},
  'whoami': {'method': 'GET', 'path': '/account-info/v3/details'}},
 {'id': 'zendesk',
  'label': 'Zendesk',
  'connectorType': 'Zendesk',
  'account': {'kind': 'label'},
  'accountLabel': 'Subdomain (the part before .zendesk.com)',
  'apiBase': 'https://{account}.zendesk.com',
  'oauth': {'authorizeUrl': 'https://{account}.zendesk.com/oauth/authorizations/new',
            'tokenUrl': 'https://{account}.zendesk.com/oauth/tokens',
            'revocationUrl': None,
            'scopes': ['read'],
            'clientAuth': 'post_json'},
  'integration': {'secretKey': 'integration:zendesk', 'secretFields': ['email', 'apiToken']},
  'whoami': {'method': 'GET', 'path': '/api/v2/users/me.json'}},
 {'id': 'confluence',
  'label': 'Confluence',
  'connectorType': 'Confluence',
  'account': {'kind': 'label'},
  'accountLabel': 'Site name (the part before .atlassian.net)',
  'apiBase': 'https://{account}.atlassian.net',
  'oauth': {'authorizeUrl': 'https://auth.atlassian.com/authorize',
            'tokenUrl': 'https://auth.atlassian.com/oauth/token',
            'revocationUrl': None,
            'scopes': ['read:page:confluence',
                       'read:space:confluence',
                       'read:comment:confluence',
                       'read:blogpost:confluence',
                       'offline_access'],
            'clientAuth': 'post'},
  'integration': {'secretKey': 'integration:confluence', 'secretFields': ['email', 'apiToken']},
  'whoami': {'method': 'GET', 'path': '/wiki/api/v2/spaces?limit=1'}},
 {'id': 'dynamics365',
  'label': 'Dynamics 365',
  'connectorType': 'Dynamics 365',
  'account': {'kind': 'dataverse_host'},
  'accountLabel': 'Environment address, as org.crm4.dynamics.com',
  'apiBase': 'https://{account}',
  'oauth': {'authorizeUrl': 'https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize',
            'tokenUrl': 'https://login.microsoftonline.com/organizations/oauth2/v2.0/token',
            'revocationUrl': None,
            'scopes': ['https://{account}/user_impersonation', 'offline_access'],
            'clientAuth': 'post'},
  'integration': {'secretKey': 'integration:dynamics365',
                  'secretFields': ['clientId', 'clientSecret']},
  'whoami': {'method': 'GET', 'path': '/api/data/v9.2/WhoAmI'}},
 {'id': 'netsuite',
  'label': 'NetSuite',
  'connectorType': 'NetSuite',
  'account': {'kind': 'account_id'},
  'accountLabel': 'Account id (for example 1234567 or 1234567_SB1)',
  'apiBase': 'https://{account}.suitetalk.api.netsuite.com',
  'oauth': {'authorizeUrl': 'https://{account}.app.netsuite.com/app/login/oauth2/authorize.nl',
            'tokenUrl': 'https://{account}.suitetalk.api.netsuite.com/services/rest/auth/oauth2/v1/token',
            'revocationUrl': 'https://{account}.suitetalk.api.netsuite.com/services/rest/auth/oauth2/v1/revoke',
            'scopes': ['rest_webservices'],
            'clientAuth': 'basic'},
  'integration': {'secretKey': 'integration:netsuite', 'secretFields': ['clientId', 'privateKey']},
  'whoami': {'method': 'GET', 'path': '/services/rest/record/v1/employee?limit=1'}},
 {'id': 'sharepoint',
  'label': 'SharePoint',
  'connectorType': 'SharePoint',
  'account': {'kind': 'label'},
  'accountLabel': 'Tenant name (the part before .sharepoint.com)',
  'apiBase': 'https://graph.microsoft.com',
  'oauth': {'authorizeUrl': 'https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize',
            'tokenUrl': 'https://login.microsoftonline.com/organizations/oauth2/v2.0/token',
            'revocationUrl': None,
            'scopes': ['https://graph.microsoft.com/Sites.Read.All', 'offline_access'],
            'clientAuth': 'post'},
  'integration': {'secretKey': 'integration:sharepoint',
                  'secretFields': ['clientId', 'clientSecret']},
  'whoami': {'method': 'GET', 'path': '/v1.0/sites/{account}.sharepoint.com'}}]
# ---------------------------------------------------------------------------------

GATE_ORIGINS_ENV = "SAAS_GATE_ORIGINS"


def rebase_for_gate(vendor_id: str, url: str) -> str:
    """`url` with its scheme, host and port replaced by the gate's mock vendor
    (`SAAS_GATE_ORIGINS`, a JSON object of vendor id to `http://host:port`).

    This goes round the rule that a vendor's hosts come only from the table (AD2),
    so it is closed twice: it does nothing unless the gate-only switch
    `CONNECTOR_OAUTH_ALLOW_HTTP=true` is on (the one `rest.py` and the API read for
    their own mock providers; a real installation never sets it) AND the variable
    names this vendor. The gate's mock vendor server answers on plain `http` at an
    internal address, so the gate also sets `INGEST_ALLOW_INTERNAL_HOSTS`; nothing in
    the product reads `SAAS_GATE_ORIGINS` for any other purpose. A value that is not
    a bare `http(s)://host[:port]` is ignored (fail closed to the real host).
    """
    if os.environ.get("CONNECTOR_OAUTH_ALLOW_HTTP", "").strip() != "true":
        return url
    try:
        table = json.loads(os.environ.get(GATE_ORIGINS_ENV, "") or "{}")
    except ValueError:
        return url
    origin = table.get(vendor_id) if isinstance(table, dict) else None
    if not isinstance(origin, str):
        return url
    from urllib.parse import urlparse

    mock, real = urlparse(origin), urlparse(url)
    if mock.scheme not in ("http", "https") or not mock.hostname or mock.path not in ("", "/") or mock.username or mock.query:
        return url
    return f"{mock.scheme}://{mock.netloc}{real.path}" + (f"?{real.query}" if real.query else "")


OAUTH_AUTH_TYPE = "oauth2_authorization_code"
OAUTH_SECRET_FIELDS = ("clientSecret", "refreshToken")


class UnsupportedVendor(Exception):
    """No module is registered for the dial's vendor (or the vendor is not in
    the table). Not retried; the message is fixed."""

    def __init__(self) -> None:
        super().__init__("this vendor has no connector in this build yet")


class InvalidAccount(ValueError):
    """`account` is not in the shape the vendor's host pattern allows. The
    message is fixed and never echoes the input."""

    def __init__(self) -> None:
        super().__init__("account is not in the form this vendor uses")


@dataclass(frozen=True)
class AccountParts:
    """What the address placeholders `{account}`, `{host}`, `{tenant}` stand for."""

    account: str
    host: str = ""
    tenant: str = ""


@dataclass(frozen=True)
class Vendor:
    id: str
    label: str
    connector_type: str
    account_rule: dict[str, Any]
    api_base: str
    oauth: dict[str, Any]
    integration: dict[str, Any] | None
    whoami: dict[str, Any] | None

    def validate_account(self, account: str) -> AccountParts:
        """`account` split into placeholder parts.

        # Errors

        `InvalidAccount` for a value outside the vendor's pattern. The text is
        taken exactly as given: whitespace, a scheme, userinfo, a port or a
        path all fail the character checks.
        """
        return _validate_account(self.account_rule, account)

    def render(self, template: str, parts: AccountParts) -> str:
        return template.replace("{account}", parts.account).replace("{host}", parts.host).replace("{tenant}", parts.tenant)

    def api_url(self, parts: AccountParts, path: str = "") -> str:
        return self.render(self.api_base, parts) + path

    def token_url(self, parts: AccountParts) -> str:
        return self.render(self.oauth["tokenUrl"], parts)

    def secret_key(self, *, oauth: bool) -> str | None:
        """The `auth_type` key of `secret_map` for this vendor and sign-in."""
        if oauth:
            return OAUTH_AUTH_TYPE
        return self.integration["secretKey"] if self.integration else None


def _is_label(text: str) -> bool:
    return (
        bool(text)
        and len(text) <= 63
        and not text.startswith("-")
        and not text.endswith("-")
        and all(c.isascii() and (c.isalnum() or c == "-") for c in text)
    )


def _is_tenant(text: str) -> bool:
    return bool(text) and len(text) <= 64 and all(c.isascii() and (c.isalnum() or c in "-_") for c in text)


def _validate_account(rule: dict[str, Any], account: str) -> AccountParts:
    kind = rule["kind"]
    if kind == "label" and _is_label(account):
        return AccountParts(account.lower())
    if kind == "digits" and account and len(account) <= 20 and account.isascii() and account.isdigit():
        return AccountParts(account)
    if kind == "account_id":
        ok = (
            0 < len(account) <= 40
            and account[0].isascii()
            and account[0].isalnum()
            and account[-1].isascii()
            and account[-1].isalnum()
            and all(c.isascii() and (c.isalnum() or c in "-_") for c in account)
        )
        if ok:
            return AccountParts(account.lower().replace("_", "-"))
    if kind == "dataverse_host":
        lowered = account.lower()
        if lowered.endswith(".dynamics.com"):
            org, dot, region = lowered[: -len(".dynamics.com")].rpartition(".")
            digits = region[3:]
            if dot and region.startswith("crm") and len(digits) <= 2 and (digits == "" or digits.isdigit()) and digits.isascii() and _is_label(org):
                return AccountParts(lowered)
    if kind == "host_tenant":
        host, slash, tenant = account.partition("/")
        lowered = host.lower()
        if slash and any(lowered.endswith(s) and _is_label(lowered[: -len(s)]) for s in rule["suffixes"]) and _is_tenant(tenant):
            return AccountParts(f"{lowered}/{tenant}", lowered, tenant)
    raise InvalidAccount()


VENDORS: dict[str, Vendor] = {
    entry["id"]: Vendor(
        id=entry["id"],
        label=entry["label"],
        connector_type=entry["connectorType"],
        account_rule=entry["account"],
        api_base=entry["apiBase"],
        oauth=entry["oauth"],
        integration=entry["integration"],
        whoami=entry["whoami"],
    )
    for entry in _TABLE
}


def vendor(vendor_id: str) -> Vendor:
    """The vendor with this id.

    # Errors

    `UnsupportedVendor` when the table has none.
    """
    found = VENDORS.get(vendor_id)
    if found is None:
        raise UnsupportedVendor()
    return found


def auth_type_of(dial: dict) -> str | None:
    """The `secret_map` key of a `saas` dial: `oauth2_authorization_code`, or
    `integration:<vendor id>`; `None` when the dial has no sign-in of that kind
    (or the vendor has no integration credential)."""
    kind = (dial.get("auth") or {}).get("type")
    if kind == OAUTH_AUTH_TYPE:
        return kind
    if kind == "integration":
        found = VENDORS.get(str(dial.get("vendor", "")))
        return found.secret_key(oauth=False) if found else None
    return None


def secret_fields_for(auth_type: str) -> tuple[str, ...] | None:
    """The secret fields behind a `saas` `auth_type` key (the sign-in's, or a
    vendor's integration credential's); `None` for a key no vendor has.
    Mirrors Rust's `saas_vendors::secret_fields_for`."""
    if auth_type == OAUTH_AUTH_TYPE:
        return OAUTH_SECRET_FIELDS
    for entry in VENDORS.values():
        if entry.integration and entry.integration["secretKey"] == auth_type:
            return tuple(entry.integration["secretFields"])
    return None


# ---- the registry of vendor modules (AD5) ------------------------------------------

_MODULES: dict[str, Any] = {}

# The vendors that have a module, by import path. A vendor's slice adds its line
# (SRC-3 plan section 5); nothing else imports a vendor module, so the
# orchestrator loads only what a connector uses.
MODULE_PATHS: dict[str, str] = {
    "hubspot": "dispar_orchestrate.saas.vendors.hubspot",
    "zendesk": "dispar_orchestrate.saas.vendors.zendesk",
    "salesforce": "dispar_orchestrate.saas.vendors.salesforce",
    "ga4": "dispar_orchestrate.saas.vendors.ga4",
}


def register(vendor_id: str, module: Any) -> None:
    """Make `module` the implementation of vendor `vendor_id`.

    # Errors

    `UnsupportedVendor` when the table has no such vendor (a module cannot add
    a vendor; only the table, and with it the Rust side, can).
    """
    vendor(vendor_id)
    _MODULES[vendor_id] = module


def unregister(vendor_id: str) -> None:
    """Forget a vendor's module (tests)."""
    _MODULES.pop(vendor_id, None)


def module_for(vendor_id: str) -> Any:
    """The registered module of a vendor.

    # Errors

    `UnsupportedVendor` when none is registered.
    """
    module = _MODULES.get(vendor_id)
    if module is None and vendor_id in MODULE_PATHS:
        module = importlib.import_module(MODULE_PATHS[vendor_id])
        _MODULES[vendor_id] = module
    if module is None:
        raise UnsupportedVendor()
    return module
