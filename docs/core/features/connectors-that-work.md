# Every listed connector can be created, tested and edited

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-6` |
| Spec | `docs/core/specs/src-6.md` (no *(proposed)* numbers; the decisions below are about behaviour) |
| Status | Decisions signed 2026-10-08. Built, not verified by CI, not accepted |
| Plan | `docs/superpowers/plans/2026-10-08-src-6-broken-connectors.md` |

## Problem

Three connector types in the picker cannot be used as shown. An object-storage
connector made in the wizard always fails its test, so its keys can never be
changed. An Oracle connector's test always reads as failed, so its password
change is refused. Google Sheets is listed as supported and moves no data.
The Sources page header promises SaaS and federation, which do not exist.
Found by reading the code on 2026-10-08; evidence is in the plan, section 2.

## What the user can do when this is done

1. Create an S3-compatible object-storage connector in the wizard, with an
   endpoint, and see its test pass.
2. Change that connector's access key and secret key; a wrong pair is refused
   and the old pair keeps working.
3. Change an Oracle connector's password. It is saved, and the console says it
   was saved without being tested.
4. Test an Oracle, MongoDB, Kafka or SFTP connector and read "cannot be tested
   from the console", not "failed" and not a list of other products.
5. See Google Sheets in the picker as not available, with the reason, and not
   be able to pick it.
6. Read a Sources page header that names only what exists.

## Not included

- A live connection test for Oracle, MongoDB, Kafka and SFTP. These connect
  from the orchestrator, not from the API; a test that runs there is new work
  (follow-up backlog item, see Decisions 1).
- Testing public AWS S3 with no endpoint set (Decision 3).
- Making Google Sheets work.
- Table discovery for these types: `SRC-13`. Gate tests for them: `SRC-9`.
- New connector types: `SRC-3`.

## Asking the assistant

Not part of the dashboards assistant. The assistant's "test connector" tool
calls the same test, so it gives the same answers. The AI team reviews that the
tool schema snapshot is unchanged.

## Decisions

All seven defaults signed by the product owner on 2026-10-08 ("run everything as proposed").

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Oracle test. The spec says "the test reports the real result". The API has no Oracle client and no pure-Rust one exists. | The test answers "cannot be tested from the console"; a password change is saved and marked not tested. A live test from the orchestrator becomes a new backlog item. | 2026-10-08 |
| 2 | MongoDB, Kafka and SFTP have the same test gap and are not in the spec's Target table. | Same answer as Oracle, in this work. The spec gains a row. | 2026-10-08 |
| 3 | Object storage with no endpoint (public AWS S3). | Not tested; the console says to set an endpoint to test. Keys are saved and marked not tested. | 2026-10-08 |
| 4 | The object-storage form offers "SFTP" as a protocol, which never loads data (SFTP is its own connector type). | Remove the option from the form. | 2026-10-08 |
| 5 | Google Sheets in the picker. | Dimmed tile that cannot be picked, with the reason on it. Sheets connectors that already exist stay, and their test keeps answering "not supported". | 2026-10-08 |
| 6 | Sources page header text. | "Databases, change capture, object storage, files, REST APIs and message topics. Data enters the platform here before processing." | 2026-10-08 |
| 7 | Order against the security fixes. `SEC-15` (PR #82) and `SEC-14` (PR #85) rewrite the same two Rust files. | The console and Sheets part is built now. The test and credential part is built after #82 and #85 merge. | 2026-10-08 |

## Limits to tell a customer

- Oracle, MongoDB, Kafka and SFTP connectors cannot be tested from the
  console. A wrong credential shows on the first load, not on save.
- An object-storage connector is tested only when it has an endpoint.
- Google Sheets is not available.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Create an object-storage connector in the wizard: endpoint, bucket, access key, secret key | Created; the test shows connected | |
| 2 | Edit it; enter a wrong secret key; save | Refused with a plain message; the test still passes with the old keys | |
| 3 | Edit it; enter the right keys again; save | Saved and tested | |
| 4 | Create an object-storage connector with no endpoint | Created; the test says it cannot be tested without an endpoint | |
| 5 | Edit an Oracle connector; change the password; save | Saved; the page says it was not tested | |
| 6 | Press Test on an Oracle, a MongoDB, a Kafka and an SFTP connector | Each says it cannot be tested from the console; health does not turn red | |
| 7 | Open the connector picker | Google Sheets is dimmed, shows its reason, cannot be picked | |
| 8 | Open Sources | The header does not mention SaaS or federation | |
| 9 | As a user without `connector:manage`, try steps 1 and 5 | Refused | |
| 10 | Point an object-storage connector at an endpoint that is down; test | A plain failure message with no address or server text in it | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
