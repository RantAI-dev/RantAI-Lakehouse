# Zendesk source, vendored (SRC-3 AD4, decision 1)

| | |
| --- | --- |
| Upstream | https://github.com/dlt-hub/verified-sources, directory `sources/zendesk/` |
| Commit | `3957506893a7da821dbcc6acd51c7ca4475d1f53` (committed 2026-07-03), the same commit as `../hubspot/` |
| Copied | 2026-10-19 |
| Licence | Apache-2.0 (`LICENSE` in this directory, the upstream `LICENSE.txt`) |
| Re-sync | None. A later upstream fix is taken by hand, with a review (decision 1) |

## Local changes

Each is its own commit. The unchanged copy is the first commit of this
directory; `git diff` against it shows exactly what follows.

1. **Zendesk Talk and Zendesk Chat removed** (`__init__.py`: `zendesk_talk`,
   `talk_resource`, `talk_incremental_resource`, `zendesk_chat`,
   `chats_table_resource`; `settings.py`: `TALK_ENDPOINTS`,
   `INCREMENTAL_TALK_ENDPOINTS`; `SRC-3` decision 5). Chat and Talk are not
   offered, and Chat is served from `www.zopim.com`, a host the connector never
   dials (hosts come only from the vendor table, AD2), so the code could not run
   through the session of the next change anyway. `zendesk_support` is
   untouched by this commit.
