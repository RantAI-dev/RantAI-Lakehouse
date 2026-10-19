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
