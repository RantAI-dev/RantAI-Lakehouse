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
2. **All HTTP goes through `SaasSession`** (`helpers/talk_api.py`,
   `__init__.py`, `helpers/credentials.py` deleted; `SRC-3` AD3, AD4). Upstream's
   `ZendeskAPIClient` built the host from a subdomain, took one of three credential
   classes (OAuth token, API token, email and password) and fetched with
   `dlt.sources.helpers.requests`, following the `next_page` / `links.next` link of
   every answer unchecked. That would go round the link rule (same host only), the
   address check before any credential, the no-redirect rule and the bounded 429
   wait. `ZendeskAPIClient(session)` now calls `session.get_json`; `zendesk_support`
   takes the session in place of `credentials`; the credential classes are gone
   (the connector's sign-in builds the headers, see `saas/vendors/zendesk.py`).
   Nothing else in `get_pages` changed: the same four pagination types.
3. **The duplicates of the time-based incremental export**
   (`helpers/talk_api.py`: `INCREMENTAL_EXPORTS`, `CURSOR_EXPORT`, `distinct_by`,
   `get_incremental`; `__init__.py`: `ticket_table` and `ticket_events` use it;
   `SRC-3` decision 6; upstream issue dlt-hub/verified-sources#659).
   *Cause.* `ticket_table` read `/api/v2/incremental/tickets` page by page
   (`PaginationType.STREAM`). A time-based export ends each page at the
   `generated_timestamp` of its last item and starts the next page AT that time, so
   the items that share it are sent again; Zendesk documents this ("Because of
   limitations with time-based pagination, the exported data may contain duplicate
   items"). Nothing in the source removed them, so a ticket at a page boundary was
   yielded twice in one run (`ticket_events` is an `append` resource with a primary
   key, so there the repeat is a second stored row). *Fix.* Tickets (and users) use
   the cursor-based export `/incremental/tickets/cursor.json`, which Zendesk
   recommends and which repeats nothing within an export: the first request takes
   `start_time`, each next page the `after_url` of the page before. Ticket events
   and organizations have only the time-based export, so there `get_pages` drops an
   item whose `id` + `created_at` (events) or `id` + `updated_at` (organizations)
   was already yielded in the call, which is Zendesk's own documented rule
   (https://developer.zendesk.com/documentation/api-basics/working-with-data/using-the-incremental-export-api/,
   "Excluding duplicate items"). The next export asks from the last `updated_at`
   and so gets the item at that exact time again; the merge key removes it.
   Tests: `test_saas_vendored_zendesk.py` (the duplicate is shown on the vendor's
   time-based endpoint, none on `get_incremental`).
