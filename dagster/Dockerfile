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

WORKDIR /opt/dagster/app
COPY pyproject.toml ./
COPY dispar_orchestrate ./dispar_orchestrate
RUN pip install --no-cache-dir .

ENV DAGSTER_HOME=/opt/dagster/dagster_home
RUN mkdir -p "$DAGSTER_HOME"
COPY dagster.yaml "$DAGSTER_HOME"/dagster.yaml
COPY workspace.yaml /opt/dagster/app/workspace.yaml

EXPOSE 3000 4000
