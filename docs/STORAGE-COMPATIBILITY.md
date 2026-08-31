# Storage compatibility — the S3 acceptance bar

This is the bar a customer-provided (or self-hosted) S3-compatible object
store must clear to sit under the RantAI Lakehouse warehouse. It exists
because of a specific risk: **RustFS, the default self-hosted
implementation, has no GA release.** Its newest tag as of this writing is
`1.0.0-rc.4`, and Docker Hub flags it `prerelease`. The mitigation for that
risk is architectural, not a promise about RustFS's roadmap: the warehouse
only ever talks to the plain S3 API (`object_store`'s `aws` backend,
never a vendor SDK or a store's proprietary admin API — see
`lakehouse-iceberg/src/storage.rs`), so a customer can point the warehouse
at any store that clears this bar — AWS S3 itself, SeaweedFS, or another
RustFS release — with **env/config changes only, no code change**. G2
(`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §3) is the proof that swap is
real, not aspirational: the same acceptance test suite passed against
RustFS and SeaweedFS from one Rust code path.

Read this as an ops runbook: each row says what to check, exactly how to
check it, and what this project has actually verified vs. left untested.
An untested row is written as untested — it is not marked green because
"it's S3-compatible" is a marketing claim, not a measurement.

## How to run the reference test yourself

The whole matrix below is measured through one acceptance suite:
`rust/crates/lakehouse-iceberg/tests/g1_lakekeeper.rs`, half (a) — Rust
creates a Bronze Iceberg table through Lakekeeper using **vended STS
credentials**, appends rows, and ClickHouse reads them back through a
`DataLakeCatalog` database. It runs inside the compose network (not on the
host — see that test file's module doc for why), via a profile-gated
compose service:

```bash
# Against RustFS (the default store):
docker compose -p g1check --profile test run --rm g1-test-runner

# Against SeaweedFS (the G2 matrix partner), after bringing its warehouse up:
docker compose -p g1check --profile seaweedfs up -d \
  seaweedfs seaweedfs-bucket-init lakekeeper-warehouse-init-seaweedfs
docker compose -p g1check --profile test --profile seaweedfs run --rm \
  -e LAKEKEEPER_WAREHOUSE=seaweedfs \
  -e CH_RUSTFS_S3_ENDPOINT=http://seaweedfs:8333 \
  g1-test-runner

docker compose -p g1check down -v   # tear down when done
```

Swapping stores here is **only** the `LAKEKEEPER_WAREHOUSE` value and the
`CH_RUSTFS_S3_ENDPOINT` override (the env var name is a historical wart —
it is generic "the S3 endpoint ClickHouse's DDL should use," not
RustFS-specific) — nothing in `rust/crates/lakehouse-iceberg` changed
between the two runs.

## The bar, and how each row is tested

### 1. SigV4 request signing

**What it proves:** the store authenticates every request the same way
AWS S3 does — required because `object_store`'s `aws` backend only speaks
SigV4, never a legacy/custom auth scheme.

**How to test:** run the reference test above. Every PUT (Parquet data
files, Avro/JSON Iceberg metadata) and every GET the object store client
issues is signed with SigV4 under the hood; an unsupported/broken
signature implementation fails the append outright with an auth error
(`403`/`SignatureDoesNotMatch`), not a silent fallback.

**Result:** RustFS — **pass** (measured in P1/G1, reconfirmed here).
SeaweedFS — **pass** (measured directly in this phase; the append and the
subsequent ClickHouse `SELECT count()` both succeeded).

### 2. `ListObjectsV2`

**What it proves:** the store supports the listing call `object_store`
uses for prefix/directory-style enumeration (relevant to orphan-file scans
and any future maintenance job that walks the warehouse by prefix rather
than by manifest).

**How to test:** exercise a `list()` call against the store through
`object_store`'s `aws` backend (or, at the S3 API level directly:
`aws --endpoint-url <store> s3api list-objects-v2 --bucket
lakehouse-warehouse`), and confirm the response is v2-shaped
(`ListBucketResult` with `NextContinuationToken`, not the v1
`Marker`-based shape).

**Result: untested.** The G1 reference path is manifest-driven — Iceberg
records exact file paths in its metadata/manifests, so neither the write
path (create + append) nor half (a)'s ClickHouse read exercises a listing
call. This row needs a dedicated check (a direct `list-objects-v2` call
per store) before it can be marked green for either store.

### 3. Multipart upload

**What it proves:** the store accepts `CreateMultipartUpload` /
`UploadPart` / `CompleteMultipartUpload` for files above
`object_store`'s single-PUT size threshold — relevant once Bronze tables
carry real CDC/dlt volumes rather than this suite's few-KB test batches.

**How to test:** write an object larger than the multipart threshold
(`object_store`'s AWS backend defaults this in the tens-of-MB range) and
confirm it lands intact; at the S3 API level,
`aws s3api create-multipart-upload` / `upload-part` /
`complete-multipart-upload` against the target bucket.

**Result: untested.** Every file this suite writes (Parquet data, Avro
manifests, JSON table metadata) is a few KB — well under any multipart
threshold — so multipart was never triggered on either store. This is a
real gap, not a formality: SeaweedFS's public issue tracker has an
interop report specifically about Lakekeeper-vended sessions failing
multipart writes because Lakekeeper's generated session policy omits the
multipart actions (`CreateMultipartUpload` etc.) — see [seaweedfs/seaweedfs
discussion #8312](https://github.com/seaweedfs/seaweedfs/discussions/8312).
The fix landed upstream in SeaweedFS (treating multipart actions as
implied by `PutObject`, per that discussion's linked PRs) and this
project's SeaweedFS image (`chrislusf/seaweedfs:4.44`) postdates that fix,
but it was **not independently re-verified here** — do not treat this row
as passing until it is.

### 4. Range `GET`

**What it proves:** the store honors `Range:` request headers — Parquet
readers (ClickHouse's `DataLakeCatalog` engine, and any external Iceberg
reader) read footers and specific row groups with range requests rather
than downloading whole files.

**How to test:** issue a `GET` with a `Range: bytes=0-99` header against
an object in the bucket and confirm a `206 Partial Content` response with
the correct byte slice; or trust the `SELECT count()` in the reference
test, which only succeeds if ClickHouse's Parquet reader could actually
read the file's footer/row-groups over HTTP.

**Result:** **likely exercised, not independently confirmed.** ClickHouse
successfully read back the exact row count Rust wrote on both stores,
which is consistent with range reads working (its Parquet reader is known
to use them), but this project did not capture wire-level request logs on
either store to confirm a `Range:` header was actually sent and a `206`
returned. Treat this as a strong positive signal, not a verified pass, for
both RustFS and SeaweedFS.

### 5. Conditional writes (`If-Match` / `If-None-Match`)

**What it proves:** the store supports optimistic-concurrency writes at
the S3 object layer — relevant to catalog implementations that use S3
itself as the source of truth for commit atomicity (e.g.
compare-and-swap on a `metadata.json` pointer object).

**How to test:** `PUT` an object with an `If-None-Match: *` header twice
and confirm the second attempt is rejected with `412 Precondition
Failed`.

**Result: not applicable to this architecture, untested at the S3 layer.**
This warehouse's concurrency control lives in Lakekeeper's REST catalog
(backed by its own Postgres schema), not in S3 conditional headers —
table commits are compare-and-swap at the catalog layer, and the object
store never receives a conditional write from this project's write path.
This row matters if a customer's own tooling (or a future catalog choice)
depends on S3-side conditional writes; it does not block the current
design and was not tested against either store.

### 6. STS-vended or remote-signed credentials

**What it proves:** the store can be the target of Lakekeeper's
short-lived credential vending (`sts-enabled: true` on the warehouse's
storage profile), which is the only credential-delegation mode
`lakehouse-iceberg` implements — see `lakehouse-iceberg::catalog`'s module
doc and ADR 0002. With `sts-enabled: false`, Lakekeeper instead returns
`remote-signing` configuration (a signer URL, no key material), which this
crate deliberately does not implement.

**How to test:** register a Lakekeeper warehouse against the store with
`"sts-enabled": true` and a `sts-role-arn`, then run the reference test —
the append call in half (a) never receives `RUSTFS_ACCESS_KEY`/
`RUSTFS_SECRET_KEY`-shaped static credentials anywhere in the code path
(see the module docs on `g1_lakekeeper.rs`, `catalog.rs`, `storage.rs`),
so a broken or absent STS response surfaces as an S3 auth failure at the
`append` call, not a silent fallback to something static.

**Result:** RustFS — **pass** (RustFS accepts an arbitrary `sts-role-arn`
string without validating it against a real IAM identity — i.e. it does
not enforce real STS semantics, it just goes along with vended-credential
issuance; documented in `docker-compose.yml`'s
`lakekeeper-warehouse-init` comment). SeaweedFS — **pass**, verified
directly in this phase: `chrislusf/seaweedfs:4.44` configured with an
`-s3.iam.config` defining an `sts` block, one role (`arn:aws:iam::
000000000000:role/lakekeeper`) with an allow-all trust policy and an
allow-all attached policy, plus one identity credential Lakekeeper
authenticates as before calling `AssumeRole`. The append succeeded, and
the resulting Parquet/Avro/JSON files were independently confirmed
present in the SeaweedFS bucket via a plain `aws s3 ls --recursive`
against the store — i.e. this was a real STS round-trip producing usable
temporary credentials, not a fallback path.

Two things worth flagging precisely, since the task called for STS
behavior differences to be reported rather than smoothed over:

- SeaweedFS's `-s3.iam.config` format needs a **base64** `sts.signingKey`
  — a plain string fails to load with `illegal base64 data`, and the
  server then silently starts without IAM/STS enforcement rather than
  refusing to boot. This is a store-side config gotcha, not a Rust
  change, and is captured in `ops/seaweedfs/iam.json`'s comment.
- SeaweedFS's STS implementation has had real interop bugs specifically
  against Lakekeeper (the multipart-session-policy issue in row 3 above,
  and a separate POST-body-vs-query-string parameter-encoding bug fixed
  in SeaweedFS's STS handler per the same discussion thread). Both predate
  the `4.44` tag this project pins, and this project's own STS round-trip
  (small-file append, no multipart) passed, but a customer running an
  older SeaweedFS build should expect STS friction that RustFS did not
  exhibit in this project's testing.

Neither finding required a `lakehouse-iceberg` code change — both are
store-side config/version differences, which is exactly what this row is
supposed to tolerate.

### 7. Lifecycle rules

**What it proves:** the store can expire/transition objects on a
schedule — relevant to Bronze retention (ADR 0004) once maintenance jobs
(P4) start relying on it rather than doing everything through Iceberg
`expire_snapshots`/`remove_orphan_files`.

**How to test:** `aws s3api put-bucket-lifecycle-configuration` against
the target bucket with an `Expiration` rule, then confirm
`get-bucket-lifecycle-configuration` echoes it back.

**Result: untested.** Nothing in the current design (P1/P2) sets or reads
bucket lifecycle rules — Bronze retention today is entirely
Iceberg-metadata-driven (ADR 0004), not S3-lifecycle-driven. This row is
listed for completeness against the task brief's acceptance bar and
because a customer's own retention/compliance tooling may depend on it,
not because anything in this project currently needs it.

## Summary matrix

| # | Requirement | RustFS `1.0.0-rc.4` | SeaweedFS `4.44` |
| - | --- | --- | --- |
| 1 | SigV4 signing | Pass (measured) | Pass (measured) |
| 2 | `ListObjectsV2` | Untested | Untested |
| 3 | Multipart upload | Untested | Untested (known past interop bug, fix unverified here) |
| 4 | Range `GET` | Likely pass (not independently confirmed) | Likely pass (not independently confirmed) |
| 5 | Conditional writes | Not applicable to this architecture / untested | Not applicable to this architecture / untested |
| 6 | STS-vended credentials | Pass (measured) | Pass (measured) |
| 7 | Lifecycle rules | Untested | Untested |

## What this means for a customer bringing their own S3

Rows 1 and 6 are the ones this project's write path actually depends on
today, and both are measured green on two different stores using the same
Rust code — that is the load-bearing evidence for "S3 is the boundary."
Rows 2, 3, 5, and 7 are not exercised by anything this project currently
does; a customer whose own tooling depends on them (bucket lifecycle
policies, S3-side conditional writes, high-volume CDC triggering
multipart) should verify those independently against their chosen store
before relying on this document as a green light. Row 4 sits in between:
behavior is consistent with correct support, but this project has not
instrumented the wire traffic to prove it.
