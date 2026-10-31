# Outside tools read published tables, and the console counts them

| | |
| --- | --- |
| Module | Build |
| Backlog | `DATA-5` |
| Kind | Task |
| Spec | `docs/core/specs/data-5.md` (every target signed by the product owner, 2026-10-31) |
| Status | Planned |
| Priority | P3 |
| Owner | In the base. |
| Acceptor | In the base. |
| Plan | `docs/superpowers/plans/2026-10-31-data-5-outside-readers.md` |
| Started | 2026-10-31 |
| Shipped | |
| Evidence | |

## Problem

- A published Gold table exists for tools outside the console, but:
  - no outside tool can read one, because the storage address handed out is internal;
  - there is no way to give a tool its own key;
  - the "Consumers (7d)" column always reads "Not measured".

## What the user can do when this is done

1. An admin creates a service identity with the scope `iceberg:read` for an outside tool, and
   gets its key, shown once, with **Rotate**.
2. The tool connects to `<console address>/api/iceberg` with that key, as a bearer token, and
   reads published Gold tables.
3. The same key cannot write, and cannot read Silver or Bronze.
4. A published table shows "Readers (7 days)": each tool, its reads and its last read.
5. A table nobody read says "No reads in 7 days". Where outside reading is not set up, it says
   "Not measured" with the reason.
6. Revoking or deleting the key stops the tool, and its past reads stay listed.

## Decisions

- Option B of the spec (product owner, 2026-10-31), built as ADR 0016: the API is the door,
  and a read is counted when the API answers it.
- History: 90 days of daily counts; the page shows 7 days.

## Limits

- Only the catalog tenant's warehouse is served. Tenant warehouses are not.
- Tools pass the key as a bearer token. The OAuth2 client-credentials exchange is not offered.
- A tool that skips the door (internal storage keys, or Lakekeeper's own port) is not counted.
  Those credentials are for internal services only.
- An operator must publish storage at `OUTSIDE_S3_ENDPOINT` (`docs/OPERATIONS.md`).

## Acceptance checklist

The rows are in the spec: `DATA-5-AC1` to `DATA-5-AC5`.

## Not included

Counting reads that skip the door, rows or bytes per read, and Silver tables.

## Asking the assistant

No. This is an admin and integration feature, not a dashboard one.
