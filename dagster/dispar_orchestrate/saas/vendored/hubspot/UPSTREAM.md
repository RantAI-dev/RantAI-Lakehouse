# HubSpot source, vendored (SRC-3 AD4, decision 1)

| | |
| --- | --- |
| Upstream | https://github.com/dlt-hub/verified-sources, directory `sources/hubspot/` |
| Commit | `3957506893a7da821dbcc6acd51c7ca4475d1f53` (committed 2026-07-03) |
| Copied | 2026-10-19 |
| Licence | Apache-2.0 (`LICENSE` in this directory, the upstream `LICENSE.txt`) |
| Re-sync | None. A later upstream fix is taken by hand, with a review (decision 1) |

## Local changes

Each is its own commit. The unchanged copy is the first commit of this
directory; `git diff` against it shows exactly what follows.

1. **All HTTP goes through `SaasSession`** (`helpers.py`, `__init__.py`;
   `SRC-3` AD3, AD4). Upstream called `dlt.sources.helpers.requests` with a
   fixed `BASE_URL`, a bearer header built from an `api_key` string, and the
   next-page link taken from the answer and fetched with no check. That would
   go round the link rule (same host only), the address check before any
   credential, the no-redirect rule and the bounded 429 wait. Now the
   functions that took `api_key` take a `session` (renamed in the signatures
   and docstrings) and call `session.get_json(path_or_link, params)`;
   `BASE_URL`, `get_url`, `_get_headers` and the `requests` import are gone.
   Nothing else in the files changed.
2. **Incremental reads through the CRM search API** (`helpers.py`
   `search_data_since`, `settings.py` `LAST_MODIFIED_PROPERTY` and the search
   limits; `SRC-3` decision 6, V-HubSpot.1). Upstream reads every record of
   every object on every run (its issue is dlt-hub/verified-sources#662, the
   open attempt is that pull request; it was read, not copied). A search with
   `last_modified >= cursor`, sorted ascending, pages with `after`. Differences
   from the pull request, each for a documented HubSpot limit
   (https://developers.hubspot.com/docs/api/crm/search): the 10,000-result cap
   restarts the query from the newest time seen and, if 10,000 records share
   one instant, raises `SearchOutOfBoundsException` (the caller reads the
   object in full) instead of silently stopping; properties beyond the
   3,000-character body limit are read with the batch-read endpoint for the ids
   of each page (the pull request would send a body HubSpot refuses once an
   account has many custom properties); contacts use `lastmodifieddate`, every
   other object `hs_lastmodifieddate`. Search does not return archived records,
   so this path reports no deletes. Associations are not read (the search
   response has none; the list path of upstream still can). Tests:
   `test_saas_vendored_hubspot.py`.
