# The G7 gate's runner image (`g7-test-runner` in `docker-compose.yml`).
#
# BLD-2 review R1 BLOCKER 8: the gate needs a Postgres client to insert the
# source rows it times, and the script used to `pip install` one at run
# time — unpinned, so two runs of the same commit could get different
# drivers, and a gate that changes what it depends on between runs is not a
# gate. Both dependencies are pinned HERE instead, the same way
# `ops/oidc-mock/Dockerfile` pins its own, so the image is built once by
# `docker compose build g7-test-runner` and the gate's `command` runs the
# script against a fixed pair of versions.
FROM python:3.12-slim

RUN pip install --no-cache-dir "requests==2.32.3" "psycopg2-binary==2.9.10"

# The script is bind-mounted read-only at `/work` (`docker-compose.yml`);
# this image carries no gate code of its own, so a stale copy can never be
# what CI ran.
WORKDIR /work
CMD ["python", "g7_pipeline_test.py"]