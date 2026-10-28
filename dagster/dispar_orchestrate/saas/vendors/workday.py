"""Workday Reports (`SRC-3` slice 6, V-Workday.1 to .3; decisions 2, 3, 5, 6, 7, 10, 13). The
vendor module of the `saas` adapter for `vendor = "workday"`.

# `rest_api` or `SaasSession` (AD5, plan section 8 item 3)

As in slices 3 to 5: the report is read through `SaasSession` directly (address check and
pinning before each request, the credential asked for only after it, no redirects, bounded
`Retry-After`), and the rows go to the same dlt resource every vendor uses. No `rest_api`
source is built and no library is added.

# Objects (decision 5): each custom report the person names

Workday Reports-as-a-Service (RaaS) serves a custom report at
`https://<host>/ccx/service/customreport2/<tenant>/<owner>/<report>?format=json`. There is
no list of reports to ask for, so an object is a report the person names in the connector's
`objects` (the dial), written `<owner>/<report>`: the report owner's Workday user name and the
report's name as it is in the address (Workday writes spaces as underscores). Both parts are
checked to be plain before they go into a path (`check_report`); the host and the tenant come
from the vendor table and the account, never from the name.

# Reading

`?format=json` returns ONE response holding the whole result, the rows under `Report_Entry`.
There is no paging (Workday: "RaaS generates the entire result set in a single response"), so
`read` makes one request. A report that does not answer within `request_timeout_seconds`
fails the run; Workday ends a report that runs longer than 30 minutes by itself, and answers
are read into memory, so a report is best kept to the columns and rows the lakehouse needs.
The shape (`Report_Entry`, an array of rows) was taken from third-party documentation, not
from Workday's, which is behind a login: an answer that does not have it is refused as one the
connector does not read, never loaded as empty.

# Re-read in full, always (decision 6; the spec gap on the feature page)

A report cannot be filtered by when a record changed, only by the prompts its author defined,
so every run replaces the table. The object has no cursor, `full_reason` says so on the
connector page, and no primary key (a report row has no identifier; nothing is merged).

# Deletes (decision 7)

Replaced with the table; nothing to apply.

# Schema changes (decision 9)

Workday is not on decision 9's list of vendors that describe their fields before the load; a
changed column is seen after the load.

# Sign-in (decision 3)

* **Integration user + password** (`integration_headers`): HTTP Basic. The integration system
  user's name is written as Workday asks (`user@tenant`) in the credential; this module sends
  it exactly as saved.
* **Browser sign-in** "where the tenant has registered an API client": the authorisation-code
  grant of the vendor table's row (`https://<host>/<tenant>/authorize`,
  `https://<host>/ccx/oauth2/<tenant>/token`), refreshed with the saved refresh token. The
  client authenticates in the Basic header. Workday's documentation is behind a login and was
  not readable (2026-10-21); the addresses and the Basic client authentication are the
  documented pattern as the developer knows it, and the sign-in is **not verified** against a
  tenant (decision 10: Workday stays unsupported until it is).
"""

from __future__ import annotations

import base64
import re
from collections.abc import Iterator
from typing import Any

import requests

from dispar_orchestrate.adapters.saas import SaasSetupRefused
from dispar_orchestrate.saas.objects import ObjectDecl

# Workday ends a report that runs longer than 30 minutes; a person waits less. A run that has to
# wait this long for one report is a report to make smaller.
request_timeout_seconds = 600

REPORT_PATH = "/ccx/service/customreport2"
_OWNER = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.@-]{0,99}$")
_REPORT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,149}$")
FULL_REASON = "Workday cannot filter a report by when a row changed, so the whole report is read again"


def _refuse() -> SaasSetupRefused:
    return SaasSetupRefused("a Workday report is written as owner/report_name")


def check_report(name: str) -> tuple[str, str]:
    """`(owner, report)` of an object name `owner/report`.

    # Errors

    `SaasSetupRefused` for anything else (the parts go into a request path, so no other
    character is accepted: no slash, dot-dot, query or percent escape).
    """
    owner, slash, report = name.partition("/")
    if not slash or not _OWNER.match(owner) or not _REPORT.match(report) or ".." in name:
        raise _refuse()
    return owner, report


def integration_headers(vendor: Any, secrets: dict[str, str]) -> dict[str, str]:
    """HTTP Basic for the integration system user (`username`, `password`)."""
    pair = f"{secrets['username']}:{secrets['password']}".encode()
    return {"Authorization": "Basic " + base64.b64encode(pair).decode()}


def declaration(session: Any, parts: Any, dial: dict, name: str) -> ObjectDecl:
    """A report: re-read in full, no key. No request is made (a report is run only to be
    read, and running one costs the customer's tenant)."""
    check_report(name)
    return ObjectDecl(name=name, primary_key=(), full_reason=FULL_REASON)


def list_objects(session: Any, parts: Any, dial: dict) -> list[ObjectDecl]:
    """The reports the connector names (`dial.objects`): Workday cannot list them."""
    return [declaration(session, parts, dial, name) for name in dial.get("objects") or [] if isinstance(name, str)]


def read(session: Any, parts: Any, dial: dict, decl: ObjectDecl, *, since: Any) -> Iterator[dict[str, Any]]:
    """The rows of the report, from the one response (`since` is always `None`: no cursor)."""
    owner, report = check_report(decl.name)
    try:
        body = session.get_json(f"{REPORT_PATH}/{parts.tenant}/{owner}/{report}", {"format": "json"})
    except requests.HTTPError as exc:
        # 400 and 404: Workday has no report by that owner and name for this tenant (or the
        # report needs prompts). The vendor's words are not repeated.
        if getattr(exc.response, "status_code", None) in (400, 404):
            raise SaasSetupRefused("Workday has no report by that owner and name") from None
        raise
    except ValueError:
        raise SaasSetupRefused("Workday answered in a form this connector does not read") from None
    entries = body.get("Report_Entry") if isinstance(body, dict) else None
    if not isinstance(entries, list):
        raise SaasSetupRefused("Workday answered in a form this connector does not read")
    for entry in entries:
        if isinstance(entry, dict):
            yield entry
