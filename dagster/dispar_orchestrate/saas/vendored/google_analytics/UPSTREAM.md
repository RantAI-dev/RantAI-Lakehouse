# Google Analytics source, vendored (SRC-3 AD4, decision 1)

| | |
| --- | --- |
| Upstream | https://github.com/dlt-hub/verified-sources, directory `sources/google_analytics/` |
| Commit | `3957506893a7da821dbcc6acd51c7ca4475d1f53` (committed 2026-07-03), the same commit as `../hubspot/`, `../zendesk/` and `../salesforce/` |
| Copied | 2026-10-21 |
| Licence | Apache-2.0 (`LICENSE` in this directory, the upstream `LICENSE.txt`) |
| Re-sync | None. A later upstream fix is taken by hand, with a review (decision 1) |

## Local changes

1. **The Data API's REST endpoints through `SaasSession`; no gRPC client, no
   Google libraries** (`helpers/data_processing.py`, `helpers/__init__.py`,
   `__init__.py`; `setup_script_gcp_oauth.py` deleted; `SRC-3` AD3, AD4, plan section 8
   item 1). Upstream built a `BetaAnalyticsDataClient` from dlt's Google credentials
   and read with `run_report` / `get_metadata` over gRPC. Checked in
   `google-analytics-data` 0.23.3 (Apache-2.0): the client does have a REST transport
   (`BetaAnalyticsDataRestTransport`), whose HTTP is `requests` through
   `google.auth.transport.requests.AuthorizedSession`, so it does resolve through
   `getaddrinfo`; but it is Google's own session, not ours: it follows redirects, has
   no address check before the credential, no pinning and no bounded 429 wait, it
   refreshes the token by its own HTTP call, and the package requires `google-api-core`,
   `google-auth`, `grpcio`, `proto-plus` and `protobuf` even for REST. The gRPC transport
   resolves outside `getaddrinfo` altogether. So neither is used and nothing is added to
   `pyproject.toml`: `runReport` is `POST /v1beta/properties/<id>:runReport` and
   `getMetadata` is `GET /v1beta/properties/<id>/metadata`, both through
   `SaasSession.post_json` / `get_json`, and rows are read from the JSON form of the
   answer (`dimensionHeaders`, `metricHeaders` with `type`, `rows`, `rowCount`; int64
   `offset` and `limit` are text in the request). The column names and types are the ones
   upstream gave (`sessions_INTEGER`, a UTC datetime for the date dimensions). The
   interactive `setup_script_gcp_oauth.py` (a local browser flow built on
   `google_auth_oauthlib`) is deleted: the product's own sign-in replaces it.
2. **Reports merge on their dimensions, re-read the last days, and make no empty-range
   request** (`__init__.py`, `helpers/__init__.py` `report_start_date`, `settings.py`
   `LOOKBACK_DAYS`; `SRC-3` decision 6, V-GA4.1). Upstream wrote each report with
   `write_disposition="append"` and no key, starting the day AFTER the last value read.
   Google says "Data processing can take 24-48 hours. During that time, data in your
   reports may change." (https://support.google.com/analytics/answer/11198161), so the
   last days were stored once and never corrected, and a rerun of a range stored its rows
   twice. Now: `merge` with the dimensions as the key; a later run starts `LOOKBACK_DAYS`
   (3) days before the last value, moved back to the first day of its week, month or year
   for `isoYearIsoWeek`, `yearMonth` and `year` (Analytics sums a partial period over the
   days asked for only, which the merge would write over the whole period); a start after
   yesterday (a configured start in the future) asks nothing, where Google would answer
   with an error. The date range stays whole days: the Data API's `dateRanges` are dates,
   in the property's time zone, while the end ("yesterday") is UTC as upstream had it.
