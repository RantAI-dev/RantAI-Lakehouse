# P3 Dagster code location image — see
# docs/adr/0005-dagster-code-location-ownership-and-packaging.md for why
# this is a built image (not a bind-mounted stock dagster/dagster image)
# and why it lives here, sibling to rust/, not inside it.
#
# One image, three roles (webserver / daemon / code-location grpc server),
# selected by `command:` in docker-compose.yml — Dagster's own standard
# multi-container topology, not a shape invented for this repo.
FROM python:3.12-slim

# WS4 (grand plan §6): every @op's metadata carries the commit the running
# image was built from, so a "source not available for this build" 409 on
# GET /api/pipelines/{id}/source?op= can name a REAL mismatch instead of
# comparing against an empty string. Defaults to "unknown" so a local
# `docker build .` without --build-arg still succeeds (no secret, no
# required input — compose passes it from `git rev-parse HEAD` at build
# time; see docker-compose.yml's dagster-code-location build block).
ARG GIT_SHA=unknown
ENV GIT_SHA=${GIT_SHA}

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential libpq-dev curl \
    && rm -rf /var/lib/apt/lists/*

# WS5 item G2: `backup_job.py`'s `pg_dump` client. `docker-compose.yml`'s
# `postgres` service is pinned `image: postgres:16` (confirmed at every
# occurrence of that pin) -- a client major must match the server major, a
# newer `pg_dump`'s custom-format output is not guaranteed to `pg_restore`
# into an older server -- but this base image's Debian release (verified
# by running `cat /etc/debian_version` inside `python:3.12-slim`: `13.6`,
# i.e. trixie) packages PostgreSQL 17 by default, not 16, so a bare
# `apt-get install postgresql-client-16` fails with "Unable to locate
# package" against trixie's default sources. The PostgreSQL project's own
# apt repository carries every still-supported major for every supported
# Debian release, so it is added first, specifically for the `-16`
# package. Requires network access to apt.postgresql.org at BUILD TIME
# (fetching the signing key and the repository index) -- a fully
# air-gapped build must pre-stage these packages some other way; this
# does not solve that case, only names it.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gnupg \
    && install -d /usr/share/postgresql-common/pgdg \
    && curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc \
         -o /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc \
    && echo "deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] https://apt.postgresql.org/pub/repos/apt trixie-pgdg main" \
         > /etc/apt/sources.list.d/pgdg.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends postgresql-client-16 \
    && rm -rf /var/lib/apt/lists/*

# adapters/sql.py's mssql driver (Microsoft ODBC Driver 18, pyodbc) needs
# BOTH the pyodbc Python package (pyproject.toml) AND this system driver
# -- pyodbc's manylinux wheel dynamically links libodbc.so.2 at runtime
# (verified: `ldd` on the wheel's .so shows `libodbc.so.2 => not found`
# until unixodbc is installed), and msodbcsql18 is the ODBC driver
# `adapters/sql.py::_mssql_connection_string`'s connection string names
# via `DRIVER={ODBC Driver 18 for SQL Server}` -- that literal string
# must match a driver `pyodbc.drivers()` actually reports, which is what
# installing msodbcsql18 under its own package-provided odbcinst.ini
# entry guarantees.
#
# Same apt-repo pattern as the PGDG block above, for the same reason:
# this base image's Debian release (trixie, `/etc/debian_version` ==
# 13.6) has no msodbcsql18 in its own default sources, but Microsoft's
# own apt repository publishes a trixie ("13") line
# (https://packages.microsoft.com/config/debian/13/prod.list, verified
# 2026-09-23) carrying it. ACCEPT_EULA=Y is required non-interactively by
# msodbcsql18's postinst script -- the package refuses to configure
# without it, by design (Microsoft's own EULA acceptance gate), not a
# workaround. Requires network access to packages.microsoft.com at BUILD
# TIME, same air-gapped-build caveat as the PGDG block above.
# Two things verified by actually running this block, not assumed from
# Microsoft's docs:
# 1. The key must be DEARMORED (gpg --dearmor) into a binary keyring, not
#    stored as the ASCII-armored .asc curl downloads directly --
#    `https://packages.microsoft.com/config/debian/13/prod.list` itself
#    names `signed-by=/usr/share/keyrings/microsoft-prod.gpg` (a binary
#    keyring path, by extension).
# 2. `https://packages.microsoft.com/keys/microsoft.asc` is NOT the key
#    that signs this repo's `InRelease` today -- apt/sqv refused it with
#    "Missing key EE4D7792F748182B, which is needed to verify signature"
#    even dearmored correctly. `EE4D7792F748182B` is
#    `https://packages.microsoft.com/keys/microsoft-2025.asc`'s own key id
#    (verified with `gpg --show-keys`), listed alongside `microsoft.asc`
#    at `https://packages.microsoft.com/keys/` -- Microsoft rotated the
#    signing key this repository uses without rotating the name every
#    published install guide still points at.
RUN apt-get update \
    && apt-get install -y --no-install-recommends unixodbc \
    && curl -fsSL https://packages.microsoft.com/keys/microsoft-2025.asc \
         | gpg --dearmor -o /usr/share/keyrings/microsoft-prod.gpg \
    && echo "deb [arch=amd64,arm64,armhf signed-by=/usr/share/keyrings/microsoft-prod.gpg] https://packages.microsoft.com/debian/13/prod trixie main" \
         > /etc/apt/sources.list.d/mssql-release.list \
    && apt-get update \
    && ACCEPT_EULA=Y apt-get install -y --no-install-recommends msodbcsql18 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/dagster/app
COPY pyproject.toml ./
COPY dispar_orchestrate ./dispar_orchestrate
RUN pip install --no-cache-dir .

ENV DAGSTER_HOME=/opt/dagster/dagster_home
RUN mkdir -p "$DAGSTER_HOME"
COPY dagster.yaml "$DAGSTER_HOME"/dagster.yaml
COPY workspace.yaml /opt/dagster/app/workspace.yaml

EXPOSE 3000 4000
