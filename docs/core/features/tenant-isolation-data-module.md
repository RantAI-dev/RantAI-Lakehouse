# One organisation never sees another's connectors or upload names

| | |
| --- | --- |
| Module | Sources (connectors, file upload), Data Explorer (annotations) |
| Backlog | `SEC-16` |
| Spec | `docs/core/specs/sec-16.md` |
| Status | Draft. Decisions not signed; the defaults below are being built for part A. Part B is not designed |
| Plan | `docs/superpowers/plans/2026-10-08-sec-16-cross-tenant.md` |

## Problem

The spec lists four leaks. Checked against the code on 2026-10-08
(`main` at `cd8e3df`):

| Spec row | What the code does today |
| --- | --- |
| `GET /api/connectors/ingestible` lists every tenant's connectors | True. The handler takes no caller and applies no tenant filter. Anyone holding `ingest:read` gets every tenant's hosts, usernames and secret names |
| All uploads share one Bronze namespace | True |
| A message names another tenant's table | Not as worded: no message carries a table name. What leaks is narrower: the refusal says *why* a name is taken ("a connector loads that table" for a connector of any tenant), so a person can learn that another organisation has a connector writing a table of that name |
| Annotation GET and PUT skip the tenant gate | PUT was fixed on `main` (the review of pull request 79). GET still skips the gate. Neither checks that the asset exists, so an annotation can be written for a table that is not there |

## What the user can do when this is done

Part A (being built):

1. A person holding `ingest:read` sees, on the ingestible list, only the
   connectors of the tenant they are acting as; a person in no tenant sees
   an empty list.
2. The orchestrator's own service identity, and an unrestricted
   administrator, still see every connector, so scheduled loads keep
   running.
3. A table name that cannot be used for an upload is refused with one
   sentence, whatever the reason, unless the reason is the person's own
   tenant's connector: "That table name cannot be used. Choose another
   name."
4. Reading an asset's annotation passes the same tenant gate as writing it.
5. Reading or writing an annotation for an asset that does not exist is
   refused with "Asset not found."

Part B (not designed, not built):

6. Each tenant's uploads live in a namespace of their own, so two tenants
   can both have a table called `orders` and neither can learn the other's
   names at all.

## Not included

- Part B. It changes where the load job writes, how a raw table is
  registered in the catalog, which database the query engine reads it
  from, and what happens to the tables already loaded. The catalog is one
  per installation and is shown to one tenant (`CATALOG_TENANT_ID`); a
  namespace per tenant has to be decided together with that rule and with
  the team that owns the pipelines. It needs its own decision record first.
- The lineage route's use of the unfiltered connector list (it only looks
  up connectors it already holds the names of; the file belongs to another
  team).
- Routes outside the Data module.

## Asking the assistant

No new assistant feature. The assistant reads annotations through its data
map, not through these routes; nothing it can ask changes.

## Decisions

Until signed, the default is used.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| D1 | Who sees every tenant's connectors on the ingestible list | A service identity, and an unrestricted (`*:*`) principal. Nobody else | |
| D2 | A tenant-less person on that list | Empty list (200), as other tenant-scoped lists answer | |
| D3 | The refusal for a taken table name | One sentence for every reason that is not the caller's own tenant's connector | |
| D4 | Annotation read refused by the tenant gate | 403 with the gate's fixed reason, as the write answers | |
| D5 | Annotation for an asset that is not in the catalog | 404, for read and write; 503 when the catalog cannot be asked | |
| D6 | Part B (namespace per tenant) | Not built until a decision record exists | |

## Limits

- After part A, a refused table name still tells a person that the name is
  taken by someone. Only part B removes that.
- Annotations already stored for assets that do not exist stay in the
  database; they are no longer readable or writable through these routes.

## Acceptance checklist

Run by the product owner on a running console with two tenants. A step not
performed is never a pass.

- [ ] A person of tenant A holding `ingest:read` calls the ingestible list and sees no connector of tenant B
- [ ] A scheduled connector load of each tenant still runs
- [ ] An upload into a table name that tenant B's connector loads is refused with the one sentence, and the sentence does not mention a connector
- [ ] An upload into a table name that the person's own tenant's connector loads still says a connector loads it
- [ ] A person outside the catalog's tenant reads an annotation and gets 403
- [ ] Reading and writing an annotation for a made-up asset id answers 404
- [ ] A user without the permission is refused on each route, as before
