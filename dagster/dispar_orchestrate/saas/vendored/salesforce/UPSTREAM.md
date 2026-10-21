# Salesforce source, vendored (SRC-3 AD4, decision 1)

| | |
| --- | --- |
| Upstream | https://github.com/dlt-hub/verified-sources, directory `sources/salesforce/` |
| Commit | `3957506893a7da821dbcc6acd51c7ca4475d1f53` (committed 2026-07-03), the same commit as `../hubspot/` and `../zendesk/` |
| Copied | 2026-10-21 |
| Licence | Apache-2.0 (`LICENSE` in this directory, the upstream `LICENSE.txt`) |
| Re-sync | None. A later upstream fix is taken by hand, with a review (decision 1) |

## Local changes

1. **All HTTP goes through `SaasSession`; no `simple_salesforce`; no password
   sign-in** (`helpers/records.py`, `helpers/client.py` deleted, `__init__.py`,
   `settings.py`; `SRC-3` AD3, AD4, decision 4). Upstream built a
   `simple_salesforce.Salesforce` client from one of six credential classes
   (username + password + security token, organisation id, connected-app password
   flow, JWT, client credentials, a session id) and read through the Bulk API 1.0,
   falling back to REST. The library accepts a `requests.Session` for every call it
   makes, Bulk API included (read in `simple_salesforce` 1.12.10: all HTTP is
   `session.request` in `call_salesforce` and the bulk handlers), but that session
   would be a plain `requests` one: it dials the host the library built, follows
   redirects and has no address check, no pinning and no bounded 429 wait, and the
   Bulk API's XML and CSV job traffic does not fit `SaasSession`'s JSON calls. So the
   library is not used and not added to `pyproject.toml`: describe is
   `GET /services/data/v<API_VERSION>/sobjects/<name>/describe` and a read is
   `GET .../query?q=<SOQL>`, paged by `nextRecordsUrl` (a path on the same host,
   which the session follows under its same-origin rule). The six credential classes
   are gone (the connector's browser sign-in builds the header; the SOAP
   username + password + security-token sign-in is retired by Salesforce in Summer
   '27 and is not offered, decision 4). Also here: an sObject or field name is checked
   to be a plain API name before it is spliced into the query; the `last_state`
   value is parsed as a time and written back as a SOQL dateTime literal; a
   `base64` (file body) field is not selected (Salesforce returns it as an address,
   one record per batch); a query longer than 12,000 characters (a `GET` address
   Salesforce would refuse beyond 16,384 bytes) is a fixed failure. `requirements.txt`
   is upstream's and is not read by anything here.
