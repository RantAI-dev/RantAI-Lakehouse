"""dagster/dispar_orchestrate/adapters/files.py -- S3-compatible object
reading for a `files`-adapter connector (`dial.protocol == "s3"`; gcs/
azure/sftp remain `connector_type` rows seeded `supported=false`, Task
D1, until implemented).

SSRF (Z1): `dial.endpoint`, when set, is caller-supplied -- same threat
model as sql.py. `build_source` resolves and checks it BEFORE any S3
call, returning the resolved address for the caller to pin. `s3fs`
(Z8 -- `boto3` is NOT installed in this Dagster environment, matching
the WS2 review's own finding) wraps `aiobotocore`, which runs over
`aiohttp`; with `aiodns` absent (verified) aiohttp's threaded resolver
calls `socket.getaddrinfo` -- FULL pinning applies (unlike sql.py's
postgresql/mssql drivers), proven with no real network by
`test_pinned_resolution_is_consulted_on_the_s3fs_path`.

Docstring correction (this module previously claimed an absent
`dial.endpoint` dials "the deployment's own fixed RustFS warehouse" --
false, and never implemented): `_default_get_object` passes
`client_kwargs={}` when `endpoint` is falsy, and `s3fs.S3FileSystem`'s
own default, with no `endpoint_url` override, is `boto3`'s standard
region-based AWS S3 endpoint -- there is no RustFS fallback anywhere in
this module or its caller. `rust/crates/lakehouse-store/src/ingest_spec.rs`'s
`FilesDial::endpoint` doc agrees: it names the field "an optional
endpoint override (e.g. a RustFS/MinIO endpoint URL rather than public
AWS S3)" -- an OVERRIDE of a public-AWS default, not a switch between two
already-implemented targets. So: an absent `endpoint` means this
connector dials the provider's own fixed, well-known public S3 endpoint
-- not a caller-chosen host -- which is exactly why `resolved` is `None`
and no SSRF check runs in that branch: the check exists for a
CALLER-CHOSEN host (Z1's threat model), and a fixed public provider
endpoint the caller cannot redirect is not one. That SSRF reasoning still
holds under the corrected meaning; only the "RustFS" claim was false. A
deployment that wants this connector to reach its own RustFS warehouse
must set `dial.endpoint` explicitly, going through the same SSRF check as
any other caller-chosen host.

Tier 1 reads `format: "csv"` only. `parquet`/`jsonl`/`xlsx` are shapes
this adapter does not yet read: `build_source` raises `ValueError` naming
the unimplemented format rather than silently skipping the object --
AGENTS.md principle 2 ("never fabricate"): a skipped file is data the
user believes was ingested.
"""

from __future__ import annotations

import csv
import io
from dataclasses import dataclass
from typing import Any, Callable
from urllib.parse import urlparse

from dispar_orchestrate import ssrf_guard

# WS3 plan review Z8: an object-size cap -- this adapter reads a whole
# object into memory (dlt's csv reader wants a seekable buffer, and
# Tier 1 does not implement chunked/streaming CSV parsing), so an
# unbounded read is a real memory-exhaustion risk from a
# connector:manage-chosen object. 256 MiB is generous for a Tier-1 CSV
# fixture/demo object; a deployment ingesting larger files needs a
# follow-up streaming implementation, not a raised cap.
_MAX_OBJECT_BYTES = 256 * 1024 * 1024


@dataclass(frozen=True)
class AdapterBuildResult:
    source: Any
    resolved: ssrf_guard.ResolvedAddress | None


def _default_get_object(
    *, endpoint, bucket, key, access_key, secret_key, filesystem_factory=None
) -> bytes:
    import s3fs

    make_fs = filesystem_factory or s3fs.S3FileSystem
    fs = make_fs(
        key=access_key,
        secret=secret_key,
        client_kwargs={"endpoint_url": endpoint} if endpoint else {},
    )
    path = f"{bucket}/{key}"
    size = fs.info(path)["size"]
    if size > _MAX_OBJECT_BYTES:
        raise ValueError(
            f"object {path!r} is {size} bytes, which exceeds this adapter's "
            f"{_MAX_OBJECT_BYTES}-byte cap (Tier 1 reads a whole object into memory)"
        )
    return fs.cat_file(path)


def build_source(
    spec: dict,
    secrets: dict[str, str],
    source_objects: list[dict],
    *,
    resolve_checked=ssrf_guard.resolve_checked,
    get_object: Callable = _default_get_object,
) -> AdapterBuildResult:
    """Build the row iterator for a `files`-adapter connector.

    # Errors

    Raises `ValueError` for a `spec["format"]` this adapter does not yet
    read (only `"csv"` is implemented in Tier 1) -- naming the format, so
    the caller can distinguish "unsupported, honestly" from a crash.
    Raises `ssrf_guard.SsrfBlocked` if `spec["endpoint"]` is set and
    resolves to a blocked address.
    """
    if spec["format"] != "csv":
        raise ValueError(f"files adapter does not yet read format {spec['format']!r}")

    endpoint = spec.get("endpoint")
    resolved = None
    if endpoint:
        parsed = urlparse(endpoint)
        port = parsed.port or (443 if parsed.scheme == "https" else 80)
        resolved = resolve_checked(parsed.hostname, port)

    def _rows():
        for obj in source_objects:
            body = get_object(
                endpoint=endpoint,
                bucket=spec["bucket"],
                key=obj["name"],
                access_key=secrets.get("accessKey"),
                secret_key=secrets.get("secretKey"),
            )
            yield from csv.DictReader(io.StringIO(body.decode("utf-8")))

    return AdapterBuildResult(source=_rows(), resolved=resolved)
