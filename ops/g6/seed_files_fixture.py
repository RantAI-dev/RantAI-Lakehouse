"""Creates the ONE real files-adapter fixture the G6 gate needs -- a
landing/orders.csv object on the warehouse bucket -- so
rust/migrations/0033_connector_ingest_spec.sql's decision to seed
conn-s3-warehouse with an EMPTY source_objects (no tracked file created
a landing/ prefix before this) has something real to point a TEST
connector at.

s3fs, not boto3 (WS3 plan review Z8): boto3 is not installed anywhere
in this Dagster image (verified, same finding as adapters/files.py's own
module docstring) -- this script uses the SAME s3fs.S3FileSystem shape,
in reverse (`pipe_file` instead of `cat_file`).
"""
from __future__ import annotations

import s3fs

_CSV_BODY = b"id,customer,amount\n1,acme,100.00\n2,globex,250.50\n"


def seed(*, endpoint: str, bucket: str, access_key: str, secret_key: str) -> None:
    fs = s3fs.S3FileSystem(key=access_key, secret=secret_key, client_kwargs={"endpoint_url": endpoint})
    fs.pipe_file(f"{bucket}/landing/orders.csv", _CSV_BODY)
