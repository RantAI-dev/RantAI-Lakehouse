# ADR 0016: outside tools read published tables through the API

Status: Accepted (2026-10-31). Backlog `DATA-5`. The product owner chose option B of
`docs/core/specs/data-5.md`. This record is how B is built.

## Context

- **Published tables exist for outside tools.** A Gold mart can be published as an Iceberg table
  in the catalog's `gold` namespace (ADR 0010, `DATA-1`).
- **Nobody can count who reads them.** `GET /api/gold/export/{mart}/consumers` is an honest
  stub.
- **Outside tools cannot log in to the catalog properly.** Lakekeeper trusts exactly one
  OpenID issuer, and here that is `ops/oidc-mock` (ADR 0011). It is a development stand-in: it
  mints a fixed list of principals at start, its tokens last 30 days, and its token endpoint
  does not check secrets. It cannot issue or revoke a login per outside tool.
- **Outside tools cannot read published tables today either.** The warehouse's storage profile
  vends `http://rustfs:9000`, a name that resolves only inside the deployment.
- **Lakekeeper's audit records stay in its own log.** It writes one per authorization check,
  but nothing collects them.

## Decision

The API is the only door for outside readers.

1. **Identity.** An outside tool is a service identity with the scope `iceberg:read`. Its key
   is the existing service credential: SHA-256 at rest, shown once at rotate, revocable. No new
   identity system is added, and the catalog's issuer is not touched.
2. **The door.** The API serves the read side of the Iceberg REST catalog under `/api/iceberg/v1`:
   - `GET config`;
   - list namespaces, and load one namespace;
   - list tables, load a table, check whether a table exists;
   - load a table's credentials.

   Rules:
   - Only the `gold` namespace of the catalog tenant's warehouse is served.
   - Every other method or path is refused with the Iceberg error shape (403).
   - Only a service principal may use the door. A browser session is refused, because the
     count is per tool.
3. **Forwarding.** The API forwards each allowed call to Lakekeeper with the token of a new
   catalog principal, `outside-readers`.
   - That principal holds `describe` on the warehouse and `select` on the `gold` namespace only.
     That is the first namespace-scoped grant in the stack, so the catalog refuses anything
     outside `gold` as well, even if the API had a bug.
   - The tool's own key never reaches Lakekeeper.
4. **The storage address.** In table-load and credential responses, the API replaces the
   vended `s3.endpoint` with `OUTSIDE_S3_ENDPOINT`. That is the address an operator publishes
   for storage, for example the RustFS host port behind TLS.
   - The vended keys are still the catalog's short-lived, table-scoped ones.
   - Without `OUTSIDE_S3_ENDPOINT`, or without the `outside-readers` token, the door answers
     503 and says which setting is missing. The consumers route then reports "Not measured"
     with the same reason.
5. **Counting.** A successful table load through the door adds one read to a daily counter for
   that tool and table.
   - The counter stores the tool's name at the time of the read, so a deleted tool's past
     reads stay readable.
   - If the counter cannot be written, the load is refused (503). An uncounted read would
     make the count a lie (principle 5).
   - Rows older than 90 days are deleted.
   - Internal readers (the API's own reader, ClickHouse, Trino) use the catalog directly, so
     they are never counted.

## Consequences

- Outside reading works for the first time. It also needs an operator step, publishing
  storage under `OUTSIDE_S3_ENDPOINT`, which `docs/OPERATIONS.md` describes.
- Catalog metadata traffic goes through the API. Data files do not: they go straight to
  storage with the vended keys.
- Reads that skip the door cannot be counted: a tool holding the internal storage keys, or one
  talking to Lakekeeper's published port with a token. Those keys and tokens are for internal
  services only, and the feature page says so.
- Replacing `oidc-mock` with a production issuer is still open (ADR 0011). This design does not
  depend on it.

## Rejected

- **Collecting Lakekeeper's audit log.** It needs a log shipper with access to the container
  runtime, the count arrives late, and it still needs per-tool catalog logins, which the issuer
  cannot give.
- **Per-tool principals in `oidc-mock`.** That would build on a development stand-in.
- **A storage audit webhook (option C).** It counts files, not reads.
