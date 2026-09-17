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

WORKDIR /opt/dagster/app
COPY pyproject.toml ./
COPY dispar_orchestrate ./dispar_orchestrate
RUN pip install --no-cache-dir .

ENV DAGSTER_HOME=/opt/dagster/dagster_home
RUN mkdir -p "$DAGSTER_HOME"
COPY dagster.yaml "$DAGSTER_HOME"/dagster.yaml
COPY workspace.yaml /opt/dagster/app/workspace.yaml

EXPOSE 3000 4000
