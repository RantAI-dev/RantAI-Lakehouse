#!/usr/bin/env python3
"""Tests for `.github/workflows/release.yml` (REL-2 S2 review BLOCKER 2).

The release pipeline is the one workflow in this repository that holds the
signing key, so two of its properties are checked here, on every commit, and
not only when a release is cut:

  - no `secrets.COSIGN_*` sits in a job-level `env:`, where EVERY step of
    the job could read it — a third-party action included;
  - every third-party action is pinned to a commit SHA, because a tag can
    move under the pipeline's feet and a commit cannot.

Two more are checked since REL-2 S2 D3a, for the same reason — a property
of the pipeline that only a release would otherwise prove:

  - the manifest's signature is a Sigstore bundle
    (`release.json.sigstore.json`), not a detached signature;
  - nothing is uploaded to the public transparency log (Rekor), in either
    job: the dry run would publish throwaway entries, and the manifest's
    signature must verify on an air-gapped host.

And two more since REL-2 S2 review round 4, from the PR's own dry run:

  - the SBOM folder is created before the SBOM action writes into it;
  - the signing key is not written to `$GITHUB_ENV`, which would put it in
    the environment of every later step of the job.

And one more since REL-2 S2 review round 5, from the PR's second dry run:

  - every SBOM is written as CycloneDX **JSON**, because `cosign attest
    --type cyclonedx` reads a JSON predicate and the action's plain
    `cyclonedx` format is XML.

And one more since the PR #313 dry run, from the build that failed:

  - every `rustup target add` of the musl target runs from inside `rust/`,
    where the toolchain pin is, so the target lands on the PINNED
    toolchain; and the pin itself does not ask for the musl target, so no
    developer downloads it.

And one more since the PR #313 install-proof, from the install that failed:

  - every file the proof job reads out of the dry run's artifact is on
    that artifact's upload list, under the SAME relative path — the
    artifact keeps every file at its path relative to the repository
    root, so a file the dry run wrote into `dist/` is read back from
    `dry-run/dist/`, not from `dry-run/`.

And three more since REL-2 S3 D13, from the review round 3 BLOCKER 1
(after `docker load`, nothing resolved a digest-pinned reference):

  - every job that builds or saves a bundle turns Docker's CONTAINERD
    image store on before its first image pull or build, and asserts
    that the store is on — on the classic store no `docker load`
    registers a repo digest, so a bundle built there cannot be
    installed on an air-gapped host;
  - the proof job turns the containerd store on before it installs;
  - the proof job cuts the runner off from the registries BEFORE it
    installs, and proves the cut — a runner that can still pull would
    hide the bug the proof exists for, because Compose pulls silently
    when it may.

And one more since PR #335's `update-proof` job, from the marker check that
failed after the REAL update to bundle B:

  - every call to the API in the `update-proof` job carries a session: the
    authenticated helper (`ops/release/proof_api.sh`) signs in and keeps the
    session in a cookie jar, and a call with no session answers 401.

And two more since REL-2 S3 review round 4 BLOCKER 1, from the dry run
that lost its registry:

  - no job that restarts Docker holds a SERVICE container: a service
    container is started before the job's first step, and the restart
    stops it — so the dry run starts its throwaway registry itself,
    after the store switch;
  - the dry run's registry is pinned by digest and is waited for.

And two more since WEB-2, from the product owner's decision of
2026-11-02 that an installed Lakehouse does not serve the landing
page:

  - the console image's build context (`.dockerignore`) keeps the
    landing page's source out (since WEB-5 the Lakehouse site, `site/`),
    so no console image carries it; and
  - the install proof asks for `/landing`'s status and fails the job
    unless it is 404.

And four more since REL-7, from the 0.2.0 release that could not attach its
bundle (GitHub refuses a release asset of 2 GiB or more):

  - the release attaches the bundle IN PARTS, split at 1900 MiB with
    `-d -a 2` numbering, `SHA256SUMS` lists every part, and the whole
    file's `.sha256` is an asset so the joined file can be checked;
  - a draft release that already exists for the tag is written into with
    `gh release upload --clobber`, so the job re-runs on the same tag —
    and a PUBLISHED release is refused;
  - the dry run splits the bundle it built, checks the parts, joins them
    in name order and checks the joined file against the bundle's own
    `.sha256`, so a split that does not round-trip fails before a tag.

The workflow is parsed with PyYAML (`pyyaml==6.0.3`, installed by the jobs
that run these tests). The checks are run against synthetic workflows too,
so a check that cannot fail is itself a failure. No network, no repository:
the tests read two files and write nothing.
"""

from __future__ import annotations

import fnmatch
import re
import sys
import unittest
from pathlib import Path
from typing import Any

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "release.yml"

# The rules of the update proof live in a module of their own, so they
# can be tested with no Docker and no network (REL-2 S4 decision D21).
RELEASE_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(RELEASE_DIR))

import update_proof  # noqa: E402  (the path is set above)

# The toolchain pin: `rustup` reads it from inside `rust/`.
TOOLCHAIN_PATH = REPO_ROOT / "rust" / "rust-toolchain.toml"

# A third-party action, pinned: `owner/repo` and a 40-hex commit SHA.
# A SUB-ACTION (`owner/repo/path@<sha>`) is pinned the same way: it is
# the same repository, at the same commit, and the pipeline uses it to
# install the SBOM action's own syft (REL-2 S2 review round 6 BLOCKER 1).
PINNED_ACTION = re.compile(
    r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)*@[0-9a-f]{40}$"
)

# The SBOM action, and the one step that is NOT it: the action's own
# `download-syft` sub-action installs syft, it does not write an SBOM.
SBOM_ACTION = re.compile(r"^anchore/sbom-action@")

# The three steps that need the signing key, and the two secrets of the
# `release` environment.
SIGNING_STEPS = (
    "Sign each image",
    "Attest each SBOM to its image",
    "Sign the manifest and verify the signature",
)
SECRET_NAMES = ("COSIGN_PRIVATE_KEY", "COSIGN_PASSWORD")

# The manifest's signature, as REL-2 S2 D3a decided it: a Sigstore bundle.
SIGSTORE_BUNDLE = "release.json.sigstore.json"

# The SBOM format every SBOM step must ask for: CycloneDX **JSON**.
# `anchore/sbom-action` takes syft's format names, and syft writes the
# plain `cyclonedx` name as XML — `cosign attest --type cyclonedx` reads a
# JSON predicate, so the XML answer is "invalid character '<' looking for
# beginning of value" (REL-2 S2 review round 5 BLOCKER 1).
SBOM_FORMAT = "cyclonedx-json"

# cosign v3's way to sign with a key and NO transparency log. Both flags
# are needed: v3 refuses `--tlog-upload=false` while the signing config
# is in effect, and that config names the public log service.
NO_TLOG_FLAGS = ("--use-signing-config=false", "--tlog-upload=false")

# The musl target the static installer is built for.
MUSL_TARGET = "x86_64-unknown-linux-musl"

# The dry run's artifact, and the folder the proof job unpacks it into.
DRY_RUN_ARTIFACT = "release-dry-run"
DRY_RUN_FOLDER = "dry-run"

# The step of the dry run that uploads the artifact, and the step of the
# proof job that downloads it.
UPLOAD_STEP = "Upload the bundle and the manifest"
DOWNLOAD_STEP = "Download the dry run's bundle"

# The step that turns Docker's containerd image store on (REL-2 S3 D13).
CONTAINERD_STEP = "Turn on the containerd image store"

# The step of the dry run that starts its throwaway registry
# (REL-2 S3 review round 4 BLOCKER 1).
REGISTRY_STEP = "Start the throwaway registry"

# The registry the dry run pushes to, and the URL it answers on.
DRY_RUN_REGISTRY = "localhost:5000"
REGISTRY_API = "http://localhost:5000/v2/"

# The step that cuts the proof runner off from the registries.
AIR_GAP_STEP = "Cut the runner off from the registries"

# The Dagster services a release install runs, and the steps that prove
# it (REL-2 S4b review round 3 BLOCKER 3, decision D22).
#
# The Dagster services carry the `dagster` profile, and a stack that
# does not run it runs no pipelines at all: no upload loads, no
# connector runs, no maintenance jobs. The proof asserts that the
# daemon and the code location run — after the install, after the
# update, and after the rollback.
DAGSTER_SERVICES = ("dagster-daemon", "dagster-code-location")
DAGSTER_STEP = "The Dagster services run"
DAGSTER_STEPS = (
    "The Dagster services run after the install",
    "The Dagster services run after the update",
    "The Dagster services run after the rollback",
)

# The install's own `.env`: the file that names the profiles the
# install runs, and the file the proof's compose call reads.
INSTALL_ENV = "--env-file install/.env"

# A release asset may not reach 2 GiB, and the bundle holds every image
# the release runs, so it is attached IN PARTS (REL-7 D1). The parts are
# made at 1900 MiB — the margin under the limit — numbered and padded by
# `-d -a 2`, so the operator's name-order join is the byte order (D2).
BUNDLE_PART_SIZE = "1900M"
BUNDLE_PARTS_GLOB = "./*.tar.zst.part-*"

# The steps of REL-7: the release job's asset list (the split and the
# checksums, D1/D5), and the dry run's round trip (D4).
ASSETS_STEP = "The release's assets"
DRAFT_STEP = "Create the draft release"
ROUND_TRIP_STEP = "The bundle splits into parts and joins back to the same bytes"


# The API the proof talks to, and the part of it that is authenticated.
#
# The proof's third marker is a stored connector credential, and it is
# read back through the API: `GET /api/connectors/{id}` needs the
# `connector:manage` permission, and a call with NO session answers
# 401. The marker check that ran after the real update to bundle B
# failed exactly there — "curl: (22) The requested URL returned error:
# 401" — because it called the API with no session at all (PR #335
# update-proof, marker check 401).
API_BASE_URL = "http://127.0.0.1:8080"
API_PATH = "/api/"

# The helper that signs in and makes the call: every API call of the
# proof goes through it, so that no step can forget the sign-in.
PROOF_API_HELPER = "ops/release/proof_api.sh"

# A call that carries the session ITSELF: the cookie jar of the
# sign-in, or a bearer token.
SESSION_CARRYING = re.compile(
    r"(?:^|\s)(?:-b\b|--cookie\b)|authorization:\s*bearer",
    re.IGNORECASE,
)

# The two steps that read the three markers back.
MARKER_STEPS = (
    "The markers are still there after the update to bundle B",
    "The stack runs bundle B again, and the markers are still there",
)

# The connector route the proof's third marker is read through, and the
# tenant every call of the proof acts as.
#
# The API's connector routes are TENANT-SCOPED (`SEC-50`, `rust/crates/
# lakehouse-api/src/tenant_scope.rs`): a connector belongs to a tenant,
# and `GET /api/connectors/{id}` answers 404 for a caller that does not
# belong to that tenant. The read of the marker connector answered 404
# after the real update to bundle B, because the connector had NO tenant
# at all: the bootstrap administrator belongs to no tenant, so the create
# fell back to the typed name and the row's `tenant_id` stayed NULL — a
# row no tenant-scoped read can see (PR #335 update-proof, connector
# 404).
#
# The API reads the tenant from the `X-Tenant` header, and the console
# sends the active tenant on EVERY request (`src/services/http.ts`
# attaches it), so the proof's calls carry it too — every one of them
# naming the SAME tenant.
CONNECTOR_PATH = "/api/connectors"

# The call that names the tenant, in the helper's own environment
# variable. The value is the tenant's id; the API reads a UUID there and
# answers 404 for anything else.
TENANT_NAMED = re.compile(r'PROOF_TENANT="(?P<tenant>[^"]+)"')

# The file the seeding step writes the tenant's id to, and every step
# that calls the API reads it back from there: the steps of a job are
# separate shells, so a variable of one step is not in the next.
TENANT_FILE = "tenant-id.txt"

# The digest reference the proof's own steps pull, and the commands the
# check below reads.
#
# Docker's CONTAINERD image store does NOT register the TAG when the
# pull is by digest: the digest reference and the tag are two names for
# one image, and only the digest one is registered (REL-2 S3 D13). A
# `docker save` of the tag then fails with "No such image", and the
# bundle's archive is never written — PR #335's `update-proof` job,
# bundle B.
DIGEST_REFERENCE = "$reference"
PULL = re.compile(r'docker pull\s+"(?P<argument>[^"]*)"')
SAVE = re.compile(r"docker save\b(?P<argument>.*)$")
TAG = re.compile(r'docker tag\s+"(?P<argument>[^"]*)"')

# The registries the proof cuts the runner off from: Docker Hub's
# registry, its authentication and its index, GHCR, Quay, and the CDN
# the Docker Hub layers come from.
REGISTRIES = (
    "registry-1.docker.io",
    "auth.docker.io",
    "index.docker.io",
    "ghcr.io",
    "quay.io",
    "production.cloudflare.docker.com",
)

# The console image's build context: `.dockerignore` decides what the
# console build sees, because the console is built from the repository
# ROOT (WEB-2).
DOCKERIGNORE_PATH = REPO_ROOT / ".dockerignore"

# The landing page's source. The product owner decided on 2026-11-02 that
# the page is a separate product page and that an installed Lakehouse must
# not serve it, so no console image carries it (WEB-2). WEB-5 moved it out
# of the console into its own package, the Lakehouse site in `site/` (D11).
LANDING_PATHS = ("site",)

# The step of the proof job that asks the console what it answers.
CONSOLE_STEP = "The stack is ready and the console answers"

# The path the landing page is served at, and the status it must answer
# on an install: the console's page-not-found.
LANDING_PATH = "/landing"
LANDING_NOT_FOUND = "404"

# A workflow with a signing secret in a job-level `env:`.
SYNTHETIC_JOB_SECRET = """
name: Synthetic
on: workflow_dispatch
jobs:
  release:
    runs-on: ubuntu-latest
    env:
      COSIGN_PASSWORD: ${{ secrets.COSIGN_PASSWORD }}
    steps:
      - name: Sign each image
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        run: cosign sign --key "$COSIGN_KEY" "$ref"
"""

# A workflow with the same secret in a STEP's `env:`, which is where it
# belongs: a step's environment is that step's alone.
SYNTHETIC_STEP_SECRET = """
name: Synthetic
on: workflow_dispatch
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - name: Sign each image
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        env:
          COSIGN_PRIVATE_KEY: ${{ secrets.COSIGN_PRIVATE_KEY }}
        run: cosign sign --key "$COSIGN_KEY" "$ref"
"""

# A workflow with an action pinned to a tag that can move.
SYNTHETIC_MOVING_TAG = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: The SBOM of the API image
        uses: anchore/sbom-action@v0
        run: syft packages
"""

# A workflow whose SBOM action writes into a folder nobody creates.
SYNTHETIC_NO_SBOM_FOLDER = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: The SBOM of the API image
        uses: anchore/sbom-action@e22c389904149dbc22b58101806040fa8d37a610 # v0.24.0
        with:
          output-file: sbom/rantai-lakehouse-api.cdx.json
"""

# A workflow that puts the signing key into the environment of every
# later step of the job.
SYNTHETIC_KEY_IN_THE_GITHUB_ENV = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: Generate a throwaway signing key
        run: |
          {
            echo 'COSIGN_PRIVATE_KEY<<COSIGN_PRIVATE_KEY_EOF'
            cat cosign.key
            echo 'COSIGN_PRIVATE_KEY_EOF'
          } >> "$GITHUB_ENV"
"""

# A workflow whose SBOM is CycloneDX XML, which `cosign attest` cannot
# read as a predicate (REL-2 S2 review round 5 BLOCKER 1).
SYNTHETIC_XML_SBOM = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: The SBOM of the API image
        uses: anchore/sbom-action@e22c389904149dbc22b58101806040fa8d37a610 # v0.24.0
        with:
          format: cyclonedx
"""

# A workflow that never pulls the third-party images, so its bundle
# would hold only the four images this repository publishes
# (REL-2 S2 review round 6 BLOCKER 1).
SYNTHETIC_NO_THIRD_PARTY_PULL = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: Build the offline bundle
        run: ops/release/make_bundle.sh --manifest release.json
"""

# A workflow that adds the musl target at the repository root, so the
# target goes to the runner's default toolchain and not to the pin the
# build runs on (PR #313 dry run).
SYNTHETIC_MUSL_TARGET_OUTSIDE_RUST = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: Build lakehousectl (static, musl)
        run: |
          set -euo pipefail
          rustup target add x86_64-unknown-linux-musl
          cd rust
          cargo build --release --target x86_64-unknown-linux-musl -p lakehousectl
"""

# A workflow whose proof job reads the bundle from the artifact's ROOT,
# although the dry run uploaded it from `dist/` — so the file is not
# there (PR #313 install-proof).
SYNTHETIC_PROOF_READS_FROM_THE_ROOT = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: Upload the bundle and the manifest
        uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
        with:
          name: release-dry-run
          path: |
            dist/*.tar.zst
            cosign.pub
  install-proof:
    runs-on: ubuntu-latest
    steps:
      - name: Download the dry run's bundle
        uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2
        with:
          name: release-dry-run
          path: dry-run
      - name: Install the bundle
        env:
          LAKEHOUSECTL_PUBLIC_KEY_FILE: ${{ github.workspace }}/dry-run/cosign.pub
        run: |
          tar --zstd -xf dry-run/rantai-lakehouse-*.tar.zst -C bundle
"""


# A workflow that pulls an image before the containerd image store is
# turned on: the image lands in the classic store, and the bundle built
# from it cannot be installed on an air-gapped host (REL-2 S3 D13).
SYNTHETIC_PULL_BEFORE_THE_STORE = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: The published images, by digest
        run: docker pull "$ref"
"""

# A workflow whose proof installs BEFORE the runner is cut off from the
# registries, so the install is not air-gapped in fact.
SYNTHETIC_INSTALL_BEFORE_THE_CUT = """
name: Synthetic
on: workflow_dispatch
jobs:
  install-proof:
    runs-on: ubuntu-latest
    steps:
      - name: Install the bundle
        run: ./lakehousectl install --bundle bundle
      - name: Cut the runner off from the registries
        run: |
          echo "0.0.0.0 ghcr.io" | sudo tee -a /etc/hosts
"""

# A workflow whose proof does not assert that the Dagster services run:
# an install that runs no profile runs no pipeline at all (REL-2 S4b
# review round 3 BLOCKER 3, decision D22).
SYNTHETIC_NO_DAGSTER_ASSERTION = """
name: Synthetic
on: workflow_dispatch
jobs:
  install-proof:
    runs-on: ubuntu-latest
    steps:
      - name: The Dagster services run
        run: |
          docker compose -p rantai-lakehouse ps
"""

# A workflow whose proof asserts the Dagster services, but reads the
# stack without the install's own `.env`: the file that names the
# profiles the install runs.
SYNTHETIC_DAGSTER_WITHOUT_THE_INSTALL_ENV = """
name: Synthetic
on: workflow_dispatch
jobs:
  install-proof:
    runs-on: ubuntu-latest
    steps:
      - name: The Dagster services run
        run: |
          docker compose -p rantai-lakehouse ps --format json
"""

# A workflow with a SERVICE container in a job that restarts Docker: the
# service is started before the job's first step, so the restart stops it
# and the store switch hides the image it ran from (REL-2 S3 review round
# 4 BLOCKER 1).
SYNTHETIC_SERVICE_ACROSS_THE_RESTART = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    services:
      registry:
        image: registry:2
        ports:
          - "5000:5000"
    steps:
      - name: Turn on the containerd image store
        run: sudo systemctl restart docker
"""

# A workflow whose service container is started by a step AFTER the store
# switch: it is not lost, so it is not reported.
SYNTHETIC_SERVICE_STARTED_AFTER_THE_SWITCH = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    services:
      registry:
        image: registry:2
    steps:
      - name: Turn on the containerd image store
        run: sudo systemctl restart docker
      - name: Start the registry
        run: docker run -d --name registry -p 5000:5000 registry:2
"""

# A workflow whose install proof asks the console what it answers, but
# never asks about the landing page (WEB-2).
SYNTHETIC_PROOF_WITHOUT_THE_LANDING_PAGE = """
name: Synthetic
on: workflow_dispatch
jobs:
  install-proof:
    runs-on: ubuntu-latest
    steps:
      - name: The stack is ready and the console answers
        run: |
          set -euo pipefail
          curl -fsS -o /dev/null -w '%{http_code}\\n' http://127.0.0.1:3000/
"""

# A workflow whose pull-request trigger does not run for a change to the
# console image's build context (WEB-2).
SYNTHETIC_WITHOUT_THE_BUILD_CONTEXT_IN_THE_PATHS = """
name: Synthetic
on:
  pull_request:
    paths:
      - ".github/workflows/release.yml"
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: Free runner disk
        run: docker image prune -af
"""

# A workflow whose proof pulls an image by its digest and then saves it
# BY ITS TAG, with no `docker tag` in between: on Docker's containerd
# image store the pull does not create the tag, so the save fails with
# "No such image" (REL-2 S3 D13; PR #335 update-proof, bundle B).
SYNTHETIC_PROOF_SAVES_A_TAG_WITHOUT_TAGGING_IT = """
name: Synthetic
on: workflow_dispatch
jobs:
  update-proof:
    runs-on: ubuntu-latest
    steps:
      - name: The bundle B is assembled and signed
        run: |
          set -euo pipefail
          digest="sha256:0123"
          reference="localhost:5000/rantai-lakehouse-api:0.1.0-update-proof.1@$digest"
          docker pull "$reference"
          docker save -o bundle-b/images/rantai-lakehouse-api.tar \\
            "localhost:5000/rantai-lakehouse-api:0.1.0-update-proof.1"
"""

# The same workflow, with the tag made before the save: the way
# `ops/release/make_bundle.sh` does it (REL-2 S3 D13).
SYNTHETIC_PROOF_TAGS_BEFORE_IT_SAVES = """
name: Synthetic
on: workflow_dispatch
jobs:
  update-proof:
    runs-on: ubuntu-latest
    steps:
      - name: The bundle B is assembled and signed
        run: |
          set -euo pipefail
          digest="sha256:0123"
          reference="localhost:5000/rantai-lakehouse-api:0.1.0-update-proof.1@$digest"
          docker pull "$reference"
          docker tag "$reference" \\
            "localhost:5000/rantai-lakehouse-api:0.1.0-update-proof.1"
          docker save -o bundle-b/images/rantai-lakehouse-api.tar \\
            "localhost:5000/rantai-lakehouse-api:0.1.0-update-proof.1"
"""


# A workflow whose proof reads the stored connector credential with a
# bare `curl`: the call carries no session, and the API answers 401
# (PR #335 update-proof, marker check 401).
SYNTHETIC_PROOF_CALLS_THE_API_WITHOUT_A_SESSION = """
name: Synthetic
on: workflow_dispatch
jobs:
  update-proof:
    runs-on: ubuntu-latest
    steps:
      - name: The markers are still there after the update to bundle B
        run: |
          detail="$(curl -fsS "http://127.0.0.1:8080/api/connectors/$connector")"
"""

# The same call with the sign-in's cookie jar: the API answers it.
SYNTHETIC_PROOF_CALLS_THE_API_WITH_THE_COOKIE_JAR = """
name: Synthetic
on: workflow_dispatch
jobs:
  update-proof:
    runs-on: ubuntu-latest
    steps:
      - name: The markers are still there after the update to bundle B
        run: |
          detail="$(curl -fsS -b cookies.txt "http://127.0.0.1:8080/api/connectors/$connector")"
"""

# A helper that signs in and then makes the call with NO session: the
# call it makes is the one the API refuses with 401.
SYNTHETIC_HELPER_WITHOUT_A_SESSION = """
#!/usr/bin/env bash
set -euo pipefail
curl -fsS -c "$COOKIE_JAR" -X POST "$API_URL/api/auth/login" -d "$payload" > /dev/null
curl -fsS "$API_URL$path"
"""

# A proof whose create and whose read act as two DIFFERENT tenants: the
# read lands in a tenant the connector is not in, and the API answers 404
# (PR #335 update-proof, connector 404).
SYNTHETIC_PROOF_CALLS_THE_CONNECTORS_IN_TWO_TENANTS = """
name: Synthetic
on: workflow_dispatch
jobs:
  update-proof:
    runs-on: ubuntu-latest
    steps:
      - name: Seed the markers
        run: |
          connector="$(PROOF_TENANT="$tenant" ops/release/proof_api.sh post /api/connectors '{}')"
      - name: The markers are still there after the update to bundle B
        run: |
          detail="$(PROOF_TENANT="$other" ops/release/proof_api.sh get "/api/connectors/$connector")"
"""

# A proof whose connector call names no tenant at all: the call then
# acts as the caller's FIRST tenant, which is not the connector's.
SYNTHETIC_PROOF_CALLS_THE_CONNECTOR_WITHOUT_A_TENANT = """
name: Synthetic
on: workflow_dispatch
jobs:
  update-proof:
    runs-on: ubuntu-latest
    steps:
      - name: The markers are still there after the update to bundle B
        run: |
          detail="$(ops/release/proof_api.sh get "/api/connectors/$connector")"
"""

# A helper that signs in and makes the call, but never names the tenant
# it acts as: the API resolves the caller's first tenant, and the
# connector of another tenant is a 404.
SYNTHETIC_HELPER_WITHOUT_A_TENANT = """
#!/usr/bin/env bash
set -euo pipefail
curl -fsS -c "$COOKIE_JAR" -X POST "$API_URL/api/auth/login" -d "$payload" > /dev/null
curl -fsS -b "$COOKIE_JAR" "$API_URL$path"
"""


# A workflow that attaches the WHOLE bundle as a release asset: GitHub
# refuses an asset of 2 GiB or more, and the bundle is bigger (REL-7 D1).
SYNTHETIC_WHOLE_BUNDLE_AS_AN_ASSET = """
name: Synthetic
on: workflow_dispatch
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - name: The release's assets
        run: |
          set -euo pipefail
          bundle="dist/rantai-lakehouse-$version-linux-x86_64.tar.zst"
          mkdir -p release-assets
          cp "$bundle" release-assets/
          (
            cd release-assets
            sha256sum ./*.tar.zst > SHA256SUMS
          )
"""

# A dry run whose round trip splits and joins, but never checks the
# joined file: a split that does not round-trip would pass it (REL-7 D4).
SYNTHETIC_ROUND_TRIP_WITHOUT_THE_CHECK = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: The bundle splits into parts and joins back to the same bytes
        run: |
          set -euo pipefail
          split -b 1900M -d -a 2 "$bundle" "$parts_dir/$name.part-"
          (
            cd "$parts_dir"
            cat ./"$name".part-* > "$name"
          )
"""

# A draft step that only creates the release: a re-run on a tag whose
# draft already exists fails (REL-7 D5).
SYNTHETIC_CREATE_ONLY = """
name: Synthetic
on: workflow_dispatch
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - name: Create the draft release
        run: |
          set -euo pipefail
          gh release create "v$version" --draft release-assets/*
"""


def load_workflow(path: Path = WORKFLOW_PATH) -> dict[str, Any]:
    """The workflow, parsed."""
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def job_environments(document: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """Every job's OWN `env:` mapping, and the workflow's.

    A step's `env:` is not in here: a step's environment is that step's
    alone, and the signing key may sit in one.
    """
    found: dict[str, dict[str, Any]] = {}
    workflow_env = document.get("env")
    if isinstance(workflow_env, dict):
        found["(workflow)"] = workflow_env
    for name, job in (document.get("jobs") or {}).items():
        env = job.get("env")
        if isinstance(env, dict):
            found[name] = env
    return found


def cosign_secrets_in_a_job_environment(document: dict[str, Any]) -> list[str]:
    """Every job-level environment that holds a signing secret."""
    found: list[str] = []
    for where, env in job_environments(document).items():
        for key, value in env.items():
            if "secrets.COSIGN_" in str(value):
                found.append(f"{where}.{key}")
    return found


def is_pinned(value: str) -> bool:
    """Whether an action is pinned to a commit SHA.

    A local action (`./path`) is code in this repository: it has no tag to
    move, so it needs no SHA.
    """
    return value.startswith("./") or bool(PINNED_ACTION.match(value))


def actions(document: dict[str, Any]) -> list[str]:
    """Every `uses:` of every step of every job."""
    found: list[str] = []
    for job in (document.get("jobs") or {}).values():
        for step in job.get("steps") or []:
            if "uses" in step:
                found.append(str(step["uses"]))
    return found


def unpinned_actions(document: dict[str, Any]) -> list[str]:
    """Every action that is NOT pinned to a commit SHA."""
    return [value for value in actions(document) if not is_pinned(value)]


def actions_without_a_version_comment(text: str, values: list[str]) -> list[str]:
    """Every action whose line does not name its release in a comment.

    The SHA is what runs; the comment says which release it is, so a
    reader — and the next bump — knows what was pinned.
    """
    missing: list[str] = []
    for value in values:
        for line in text.splitlines():
            if f"uses: {value}" not in line:
                continue
            if not re.search(r"#\s*v\d", line):
                missing.append(value)
            break
    return missing


def steps_of(document: dict[str, Any], job_name: str) -> list[dict[str, Any]]:
    """The steps of one job."""
    job = (document.get("jobs") or {}).get(job_name) or {}
    return list(job.get("steps") or [])


def step_named(document: dict[str, Any], job_name: str, name: str) -> dict[str, Any]:
    """One step, by its name."""
    for step in steps_of(document, job_name):
        if step.get("name") == name:
            return step
    raise AssertionError(f"no step named {name!r} in the job {job_name!r}")


def step_index(document: dict[str, Any], job_name: str, name: str) -> int:
    """Where a step sits in its job."""
    for index, step in enumerate(steps_of(document, job_name)):
        if step.get("name") == name:
            return index
    raise AssertionError(f"no step named {name!r} in the job {job_name!r}")


def third_party_pull_steps(document: dict[str, Any]) -> list[str]:
    """Every job that pulls the third-party images, by digest."""
    found: list[str] = []
    for job_name, job in (document.get("jobs") or {}).items():
        for step in job.get("steps") or []:
            run = str(step.get("run") or "")
            if "compose_images.py" in run and "docker pull" in run:
                found.append(job_name)
                break
    return found


def is_sbom_step(step: dict[str, Any]) -> bool:
    """Whether a step writes an SBOM with the SBOM action.

    The action's `download-syft` SUB-action is not one: it installs
    syft, and the loop below runs it.
    """
    return SBOM_ACTION.match(str(step.get("uses") or "")) is not None


def creates_the_sbom_folder(document: dict[str, Any], job_name: str) -> bool:
    """Whether the job creates `sbom/` before its first SBOM step.

    The SBOM action writes into `sbom/` and does not create the folder,
    so a job that does not create it first fails with ENOENT.
    """
    steps = steps_of(document, job_name)
    first = next(
        (
            index
            for index, step in enumerate(steps)
            if is_sbom_step(step)
        ),
        None,
    )
    if first is None:
        return False
    return any("mkdir -p sbom" in str(step.get("run") or "") for step in steps[:first])


def shell_commands(run: str) -> str:
    """A `run:` block without its comment lines.

    A comment is not a command: the workflow explains itself in them,
    and the checks below are about what the steps DO.
    """
    return "\n".join(
        line for line in run.splitlines() if not line.strip().startswith("#")
    )


def logical_lines(run: str) -> list[str]:
    """The run block's logical lines.

    A line that ends with a backslash continues on the next one, so a
    command split over two lines is ONE command: a `docker save` whose
    image is named on the next line still saves that image.
    """
    lines: list[str] = []
    buffer = ""
    for raw in shell_commands(run).splitlines():
        line = raw.strip()
        buffer = f"{buffer} {line}" if buffer else line
        if buffer.endswith("\\"):
            buffer = buffer[:-1].rstrip()
            continue
        lines.append(buffer)
        buffer = ""
    if buffer:
        lines.append(buffer)
    return lines


def digest_pulls(lines: list[str]) -> list[int]:
    """Where the step pulls an image BY ITS DIGEST.

    A pull of a plain tag (`alpine:3.20`) is not one: the tag is the
    name that pull creates.
    """
    found: list[int] = []
    for index, line in enumerate(lines):
        match = PULL.search(line)
        if match is None:
            continue
        argument = match.group("argument")
        if "@" in argument or argument == DIGEST_REFERENCE:
            found.append(index)
    return found


def tag_saves(lines: list[str]) -> list[int]:
    """Where the step saves an image BY ITS TAG.

    A save of a digest reference is not one: the archive it writes
    loads as a dangling image, and the host that loads it cannot
    resolve the digest-pinned reference the manifest names (REL-2 S3
    D13).
    """
    found: list[int] = []
    for index, line in enumerate(lines):
        match = SAVE.search(line)
        if match is None:
            continue
        argument = match.group("argument")
        if "@" in argument or DIGEST_REFERENCE in argument:
            continue
        found.append(index)
    return found


def tags_the_digest_reference(lines: list[str], start: int, end: int) -> bool:
    """Whether the lines between two of them tag the digest reference."""
    for line in lines[start:end]:
        match = TAG.search(line)
        if match is None:
            continue
        argument = match.group("argument")
        if argument == DIGEST_REFERENCE or "@" in argument:
            return True
    return False


def untagged_saves(document: dict[str, Any], job_name: str) -> list[str]:
    """Every image the job pulls by digest and then saves BY TAG, with
    no `docker tag` in between.

    Docker's containerd image store does not register the TAG when the
    pull is by digest, so the save of the tag fails with "No such
    image" and the bundle's archive is never written (REL-2 S3 D13;
    PR #335 update-proof, bundle B).
    """
    problems: list[str] = []
    for step in steps_of(document, job_name):
        name = str(step.get("name"))
        lines = logical_lines(str(step.get("run") or ""))
        for pull in digest_pulls(lines):
            for save in tag_saves(lines):
                if save <= pull:
                    continue
                if tags_the_digest_reference(lines, pull, save):
                    continue
                problems.append(
                    f"{job_name}: {name}: the image is pulled by digest and then saved by tag, "
                    f"and nothing tags it in between: on Docker's containerd image store a pull by "
                    f"digest does not create the tag, so the save fails with 'No such image'."
                )
    return problems


def api_calls_without_a_session(document: dict[str, Any], job_name: str) -> list[str]:
    """Every call to the API that carries no session.

    The API's `/api/` routes are authenticated, and a call with no
    session answers 401 — the marker check that ran after the real
    update to bundle B failed on exactly that (PR #335 update-proof,
    marker check 401). A call carries a session either through the
    authenticated helper — which signs in and keeps the session in a
    cookie jar — or through the cookie jar or the bearer token itself.
    """
    problems: list[str] = []
    for step in steps_of(document, job_name):
        name = str(step.get("name"))
        for line in logical_lines(str(step.get("run") or "")):
            if API_BASE_URL not in line or API_PATH not in line:
                continue
            if PROOF_API_HELPER in line or SESSION_CARRYING.search(line):
                continue
            problems.append(
                f"{job_name}: {name}: the call to the API carries no session: {line}"
            )
    return problems


def helper_problems(script: str) -> list[str]:
    """What the helper script is missing.

    The helper is what makes the proof's calls authenticated, so it
    must sign in, keep the session in a cookie jar, and make the call
    with the jar.
    """
    problems: list[str] = []
    if "/api/auth/login" not in script:
        problems.append("the helper never signs in")
    if "jq -n --arg" not in script:
        problems.append(
            "the sign-in payload is not built with jq -n --arg, and a secret is never "
            "spliced into a string by the shell"
        )
    if '-c "$COOKIE_JAR"' not in script:
        problems.append("the helper does not keep the session in a cookie jar")
    if '-b "$COOKIE_JAR"' not in script:
        problems.append("the helper does not make the call with the session")
    return problems


def connector_calls(document: dict[str, Any], job_name: str) -> list[tuple[str, str]]:
    """Every call of the job that touches a connector route: the step it
    is in, and the line it is on."""
    found: list[tuple[str, str]] = []
    for step in steps_of(document, job_name):
        name = str(step.get("name"))
        for line in logical_lines(str(step.get("run") or "")):
            if PROOF_API_HELPER in line and CONNECTOR_PATH in line:
                found.append((name, line))
    return found


def connector_tenant_problems(document: dict[str, Any], job_name: str) -> list[str]:
    """Every connector call that names no tenant, or a tenant of its own.

    The API's connector routes are tenant-scoped (`SEC-50`): a connector
    belongs to a tenant, and a call that acts as another tenant sees no
    connector at all — a 404. The proof's create and its two reads must
    therefore all act as the SAME tenant, and each of them must NAME it
    (the `X-Tenant` header the console sends on every request).
    """
    problems: list[str] = []
    tenants: dict[str, list[str]] = {}
    for name, line in connector_calls(document, job_name):
        match = TENANT_NAMED.search(line)
        if match is None:
            problems.append(
                f"{job_name}: {name}: the connector call names no tenant: {line}"
            )
            continue
        tenants.setdefault(match.group("tenant"), []).append(f"{job_name}: {name}")
    if len(tenants) > 1:
        problems.append(
            f"{job_name}: the connector calls name {len(tenants)} tenants "
            f"({', '.join(sorted(tenants))}), and they must all name the same one: "
            f"a call that acts as another tenant sees no connector at all."
        )
    return problems


def helper_tenant_problems(script: str) -> list[str]:
    """What the helper is missing about the tenant.

    The API's tenant-scoped routes read the tenant from the `X-Tenant`
    header (`tenant_scope::resolve`), and the console sends the active
    tenant on every request (`src/services/http.ts`): the helper sends
    it too, when `PROOF_TENANT` names a tenant.
    """
    problems: list[str] = []
    if "PROOF_TENANT" not in script:
        problems.append("the helper reads no tenant to act as")
    if "X-Tenant: $PROOF_TENANT" not in script:
        problems.append("the helper does not send the tenant as the X-Tenant header")
    return problems


def connector_tenant_file_problems(document: dict[str, Any], job_name: str) -> list[str]:
    """Every connector call that does not act as the tenant the seeding
    step created.

    The seeding step writes the tenant's id to a file, and every step
    that calls the API reads it back from there: the steps of a job are
    separate shells, so a variable of one step is not in the next.
    """
    problems: list[str] = []
    steps = steps_of(document, job_name)
    if not any(
        f"> {TENANT_FILE}" in str(step.get("run") or "") for step in steps
    ):
        problems.append(
            f"{job_name}: no step writes the tenant's id to {TENANT_FILE}"
        )
    for step in steps:
        name = str(step.get("name"))
        run = str(step.get("run") or "")
        if not any(
            PROOF_API_HELPER in line and CONNECTOR_PATH in line
            for line in logical_lines(run)
        ):
            continue
        if f"cat {TENANT_FILE}" not in run:
            problems.append(
                f"{job_name}: {name}: the connector call does not read the tenant from "
                f"{TENANT_FILE}, and so it does not act as the tenant the seeding step "
                f"created"
            )
    return problems


def sbom_steps(document: dict[str, Any]) -> list[tuple[str, dict[str, Any]]]:
    """Every SBOM step of every job, with the job it is in."""
    found: list[tuple[str, dict[str, Any]]] = []
    for job_name, job in (document.get("jobs") or {}).items():
        for step in job.get("steps") or []:
            if is_sbom_step(step):
                found.append((job_name, step))
    return found


def sbom_formats(document: dict[str, Any]) -> list[str]:
    """Every SBOM step whose format is not CycloneDX JSON.

    `cosign attest --type cyclonedx` reads a JSON predicate, and the
    action's plain `cyclonedx` format is XML (REL-2 S2 review round 5
    BLOCKER 1).
    """
    found: list[str] = []
    for job_name, step in sbom_steps(document):
        with_ = step.get("with") or {}
        if str(with_.get("format")) != SBOM_FORMAT:
            found.append(f"{job_name}: {step.get('name')}")
    return found


def steps_writing_the_key_to_the_github_env(document: dict[str, Any]) -> list[str]:
    """Every step that writes the signing key into `$GITHUB_ENV`.

    A variable written to `$GITHUB_ENV` is in the environment of EVERY
    later step of the job, and the runner prints it in each step's
    environment block (REL-2 S2 review round 4 SHOULD-FIX 1).
    """
    found: list[str] = []
    for job_name, job in (document.get("jobs") or {}).items():
        for step in job.get("steps") or []:
            commands = shell_commands(str(step.get("run") or ""))
            if "$GITHUB_ENV" in commands and "COSIGN_PRIVATE_KEY" in commands:
                found.append(f"{job_name}: {step.get('name')}")
    return found


def musl_target_steps(document: dict[str, Any]) -> list[tuple[str, str]]:
    """Every step that adds the musl target, with the job it is in."""
    found: list[tuple[str, str]] = []
    for job_name, job in (document.get("jobs") or {}).items():
        for step in job.get("steps") or []:
            run = str(step.get("run") or "")
            if "rustup target add" in run and MUSL_TARGET in run:
                found.append((job_name, str(step.get("name"))))
    return found


def adds_the_musl_target_inside_rust(step: dict[str, Any]) -> bool:
    """Whether the step adds the musl target AFTER it changed to `rust/`.

    `rustup` picks its toolchain from the `rust-toolchain.toml` it finds
    in the current directory or above it. The pin lives in
    `rust/rust-toolchain.toml`, so a `rustup target add` at the
    repository root installs the target for the runner's DEFAULT
    toolchain — and the build, which runs on the PINNED toolchain, finds
    no musl target (PR #313 dry run).
    """
    lines = [
        line
        for line in str(step.get("run") or "").splitlines()
        if not line.strip().startswith("#")
    ]
    changed_to_rust: int | None = None
    added: int | None = None
    for index, line in enumerate(lines):
        if changed_to_rust is None and re.match(
            r"cd\s+rust(?:\s|$)", line.strip()
        ):
            changed_to_rust = index
        if added is None and "rustup target add" in line and MUSL_TARGET in line:
            added = index
    return (
        changed_to_rust is not None
        and added is not None
        and changed_to_rust < added
    )


def upload_entries(document: dict[str, Any]) -> list[str]:
    """Every path the dry run uploads, as the upload step lists them.

    The upload step's `path:` is a block of lines, one glob per line.
    A line that starts with `#` is a comment about the list and names
    no file, so it is not an entry.
    """
    step = step_named(document, "dry-run", UPLOAD_STEP)
    raw = str((step.get("with") or {}).get("path") or "")
    return [
        line.strip()
        for line in raw.splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]


def dry_run_reads(document: dict[str, Any]) -> list[str]:
    """Every path the proof job reads out of the dry run's artifact.

    A path is a read when a step's command or a step's environment
    names a file under the folder the artifact was unpacked into. The
    commands are read WITHOUT their comment lines: a comment explains
    the step, it does not read a file.
    """
    found: list[str] = []
    pattern = re.compile(rf"{DRY_RUN_FOLDER}/([A-Za-z0-9._*/-]+)")
    for step in steps_of(document, "install-proof"):
        texts = [shell_commands(str(step.get("run") or ""))]
        texts.extend(str(value) for value in (step.get("env") or {}).values())
        for text in texts:
            found.extend(
                f"{DRY_RUN_FOLDER}/{match}" for match in pattern.findall(text)
            )
    return found


def entry_covers(entry: str, path: str) -> bool:
    """Whether an upload entry holds a file at that same relative path.

    The artifact keeps every file at its path relative to the ROOT of
    the upload list, so the file's FOLDER has to be the entry's folder
    and the file's name has to match the entry's glob. A glob that
    spans folders (`*` matches `/` here) is not enough: it would let
    `dist/*.tar.zst` stand for a file read from the artifact's root.
    """
    entry_folder, _, entry_name = entry.rpartition("/")
    path_folder, _, path_name = path.rpartition("/")
    if entry_folder != path_folder:
        return False
    return fnmatch.fnmatchcase(path_name, entry_name)


def reads_without_an_uploaded_file(document: dict[str, Any]) -> list[str]:
    """Every path the proof job reads that the upload list does not hold.

    The path is the one the job uses, under the folder the artifact was
    unpacked into — so a file the dry run wrote into `dist/` and that
    the proof reads from `dry-run/` is reported.
    """
    entries = upload_entries(document)
    missing: list[str] = []
    for path in dry_run_reads(document):
        relative = path[len(DRY_RUN_FOLDER) + 1 :]
        if not any(entry_covers(entry, relative) for entry in entries):
            missing.append(path)
    return missing


def step_index_or_none(document: dict[str, Any], job_name: str, name: str) -> int | None:
    """Where a step sits in a job, or None when the job has no such step."""
    for index, step in enumerate(steps_of(document, job_name)):
        if str(step.get("name")) == name:
            return index
    return None


def first_image_step(document: dict[str, Any], job_name: str) -> int | None:
    """Where the job's first image pull or build sits.

    A build is a `docker/build-push-action` step, a pull is a step whose
    command pulls an image. Both put an image into the daemon's store,
    and that store has to be the containerd one before either of them
    runs (REL-2 S3 D13).
    """
    for index, step in enumerate(steps_of(document, job_name)):
        if str(step.get("uses") or "").startswith("docker/build-push-action@"):
            return index
        if "docker pull" in str(step.get("run") or ""):
            return index
    return None


def containerd_too_late(document: dict[str, Any], job_name: str) -> list[str]:
    """Every job whose containerd step is missing, or after its first image.

    A job that pulls or builds an image before the containerd store is
    on puts that image in the classic store, and the bundle built from
    it is a bundle no air-gapped host can install.
    """
    first = first_image_step(document, job_name)
    if first is None:
        return []
    store = step_index_or_none(document, job_name, CONTAINERD_STEP)
    if store is None or store >= first:
        return [job_name]
    return []


def containerd_step_problems(document: dict[str, Any], job_name: str) -> list[str]:
    """Every way the job's containerd step can be wrong."""
    problems: list[str] = []
    if step_index_or_none(document, job_name, CONTAINERD_STEP) is None:
        return [f"{job_name}: the job never turns the containerd store on"]
    run = str(step_named(document, job_name, CONTAINERD_STEP).get("run") or "")
    for needle, what in (
        ("daemon.json", "it does not touch /etc/docker/daemon.json"),
        ("containerd-snapshotter", "it does not turn the feature on"),
        ("jq", "it does not merge the daemon's own configuration"),
        ("systemctl restart docker", "it does not restart Docker"),
        ("io.containerd.snapshotter.v1", "it does not assert that the store is on"),
    ):
        if needle not in run:
            problems.append(f"{job_name}: {what}")
    return problems


def services_killed_by_the_store_switch(document: dict[str, Any], job_name: str) -> list[str]:
    """Every service container the job loses when the store comes on.

    A service container is started BEFORE the job's first step, and the
    containerd step RESTARTS Docker: the restart stops the service
    container, and the store switch hides the image it ran from — the
    classic store's images are not in the containerd one. A service that
    a step of the job starts AFTER the switch is not lost, and is not
    reported (REL-2 S3 review round 4 BLOCKER 1).
    """
    job = (document.get("jobs") or {}).get(job_name) or {}
    services = job.get("services") or {}
    if not services:
        return []
    store = step_index_or_none(document, job_name, CONTAINERD_STEP)
    if store is None:
        return []
    started_after: set[str] = set()
    for step in steps_of(document, job_name)[store + 1 :]:
        commands = shell_commands(str(step.get("run") or ""))
        for name in services:
            if f"--name {name}" in commands or f"--name={name}" in commands:
                started_after.add(str(name))
    return [str(name) for name in services if str(name) not in started_after]


def air_gap_problems(document: dict[str, Any]) -> list[str]:
    """Every way the proof's air gap can be missing.

    The proof has to be air-gapped IN FACT: a runner that can still
    pull would let Compose fetch the images the bundle should have
    loaded, and the bug the proof exists for would stay hidden.
    """
    problems: list[str] = []
    names = [str(step.get("name")) for step in steps_of(document, "install-proof")]
    if AIR_GAP_STEP not in names:
        return ["the proof has no air-gap step"]
    if "Install the bundle" in names and names.index(AIR_GAP_STEP) > names.index(
        "Install the bundle"
    ):
        problems.append("the proof installs before the runner is cut off")
    run = str(step_named(document, "install-proof", AIR_GAP_STEP).get("run") or "")
    if "/etc/hosts" not in run:
        problems.append("the air-gap step does not touch /etc/hosts")
    for host in REGISTRIES:
        if host not in run:
            problems.append(f"the air-gap step does not cut {host} off")
    if "docker pull" not in run or "exit 1" not in run:
        problems.append("the air-gap step does not prove that the cut holds")
    return problems


def pull_request_paths(document: dict[str, Any]) -> list[str]:
    """The paths a pull request must touch for the workflow to run.

    PyYAML reads the `on:` key as the boolean `True`, because YAML 1.1
    spells "on" as a boolean.
    """
    triggers = document.get(True) or document.get("on") or {}
    pull_request = triggers.get("pull_request") or {}
    return [str(path) for path in (pull_request.get("paths") or [])]


def dockerignore_patterns(text: str) -> list[str]:
    """The patterns of a `.dockerignore`, without its comments."""
    return [
        line.strip()
        for line in text.splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]


def dockerignore_excludes(text: str, path: str) -> bool:
    """Whether a `.dockerignore` keeps a path out of the build context.

    Docker's rule is that the LAST matching pattern wins, and a pattern
    that starts with `!` re-includes what an earlier one excluded. So a
    later `!site` would undo the exclusion, and this answers
    the question the build actually asks.
    """
    excluded = False
    for pattern in dockerignore_patterns(text):
        negated = pattern.startswith("!")
        if negated:
            pattern = pattern[1:]
        pattern = pattern.rstrip("/")
        if pattern == path or path.startswith(f"{pattern}/"):
            excluded = not negated
    return excluded


def landing_proof_problems(document: dict[str, Any]) -> list[str]:
    """Every way the install proof can fail to check the landing page.

    An installed Lakehouse must not serve the landing page (WEB-2): the
    console image excludes it, so the path answers with the console's
    page-not-found. A proof that does not ask for the status, or that
    does not fail the job unless the status is 404, lets the page back
    into the install unnoticed.
    """
    try:
        step = step_named(document, "install-proof", CONSOLE_STEP)
    except AssertionError:
        return ["the proof has no step that asks the console"]
    commands = shell_commands(str(step.get("run") or ""))
    problems: list[str] = []
    if f"127.0.0.1:3000{LANDING_PATH}" not in commands:
        problems.append("the proof does not ask for the landing page's status")
    if LANDING_NOT_FOUND not in commands:
        problems.append("the proof does not name the status the page must answer")
    if "exit 1" not in commands:
        problems.append("the proof does not fail the job")
    return problems


def dagster_assertion_problems(
    document: dict[str, Any], job_name: str, step_name: str
) -> list[str]:
    """Every way the job's Dagster assertion can be missing.

    The assertion is what proves the release install runs the Dagster
    services: they carry the `dagster` profile, and a stack without it
    runs no pipelines at all (REL-2 S4b review round 3 BLOCKER 3,
    decision D22).

    The stack is read with Compose's own JSON, and with the install's
    own `.env`: that file names the profiles the install runs, and
    without it the Dagster services are not part of the deployment the
    command reads.
    """
    try:
        step = step_named(document, job_name, step_name)
    except AssertionError:
        return [f"{job_name}: the job has no step named {step_name!r}"]
    run = str(step.get("run") or "")
    problems: list[str] = []
    for service in DAGSTER_SERVICES:
        if service not in run:
            problems.append(f"{job_name}: {step_name}: it does not assert {service}")
    if "ps --format json" not in run:
        problems.append(
            f"{job_name}: {step_name}: it does not read the stack's own JSON"
        )
    if INSTALL_ENV not in run:
        problems.append(
            f"{job_name}: {step_name}: it does not read the install's own .env"
        )
    return problems


def bundle_parts_problems(document: dict[str, Any], job_name: str) -> list[str]:
    """Every way the job's asset list breaks the bundle into parts wrong.

    GitHub refuses a release asset of 2 GiB or more, and the v0.2.0
    bundle was 3,794,642,569 bytes ("size must be less than
    2147483648"), so the release splits it at 1900 MiB and attaches the
    PARTS (REL-7 D1). The whole file itself must not be an asset, its
    `.sha256` must be one, and `SHA256SUMS` must list every part — that
    is the list the operator checks each download against (D2).
    """
    problems: list[str] = []
    try:
        step = step_named(document, job_name, ASSETS_STEP)
    except AssertionError:
        return [f"{job_name}: the job has no step named {ASSETS_STEP!r}"]
    commands = shell_commands(str(step.get("run") or ""))
    split = f"split -b {BUNDLE_PART_SIZE} -d -a 2"
    if split not in commands:
        problems.append(
            f"{job_name}: the bundle is not split with `{split}`, and a part at "
            f"2 GiB or more is an asset GitHub refuses"
        )
    if 'cp "$bundle" release-assets/' in commands:
        problems.append(
            f"{job_name}: the whole bundle is copied into the assets, and GitHub "
            f"refuses an asset of 2 GiB or more"
        )
    if 'cp "$bundle.sha256" release-assets/' not in commands:
        problems.append(
            f"{job_name}: the whole file's .sha256 is not an asset, so the joined "
            f"file has nothing to be checked against"
        )
    if BUNDLE_PARTS_GLOB not in commands:
        problems.append(
            f"{job_name}: SHA256SUMS does not list the parts (`{BUNDLE_PARTS_GLOB}`)"
        )
    return problems


def draft_reuse_problems(document: dict[str, Any], job_name: str) -> list[str]:
    """Every way `gh release create` cannot be re-run on an existing draft.

    The v0.2.0 run created its draft and then failed on the bundle's
    size, so the job has to run again on the same tag (REL-7 D5): a
    DRAFT is written into with `gh release upload --clobber`, and a
    PUBLISHED release is refused — it is what an operator installed.
    """
    problems: list[str] = []
    try:
        step = step_named(document, job_name, DRAFT_STEP)
    except AssertionError:
        return [f"{job_name}: the job has no step named {DRAFT_STEP!r}"]
    commands = shell_commands(str(step.get("run") or ""))
    if "gh release create" not in commands:
        problems.append(f"{job_name}: the step never creates the draft release")
    if "gh release upload" not in commands or "--clobber" not in commands:
        problems.append(
            f"{job_name}: the step does not write into a draft that already exists "
            f"(`gh release upload --clobber`)"
        )
    if "isDraft" not in commands:
        problems.append(
            f"{job_name}: the step does not ask whether the release is a draft"
        )
    if "published" not in commands:
        problems.append(
            f"{job_name}: the step does not refuse a published release"
        )
    return problems


def round_trip_problems(document: dict[str, Any], job_name: str) -> list[str]:
    """Every way the dry run fails to prove the split round-trips.

    REL-7 D4: the dry run makes the parts the release makes, checks them
    against a `SHA256SUMS` of its own, joins them in name order, and
    checks the joined file against the `.sha256` the bundle script
    wrote. A bundle that does not round-trip must fail the dry run.
    """
    problems: list[str] = []
    try:
        step = step_named(document, job_name, ROUND_TRIP_STEP)
    except AssertionError:
        return [f"{job_name}: the job has no step named {ROUND_TRIP_STEP!r}"]
    commands = shell_commands(str(step.get("run") or ""))
    split = f"split -b {BUNDLE_PART_SIZE} -d -a 2"
    if split not in commands:
        problems.append(f"{job_name}: the round trip does not split with `{split}`")
    if "sha256sum -c SHA256SUMS" not in commands:
        problems.append(f"{job_name}: the round trip does not check each part")
    if 'cat ./"$name".part-* > "$name"' not in commands:
        problems.append(
            f"{job_name}: the round trip does not join the parts in name order"
        )
    if 'sha256sum -c "$name.sha256"' not in commands:
        problems.append(
            f"{job_name}: the round trip does not check the joined file against the "
            f"bundle's own .sha256"
        )
    return problems


class ReleaseWorkflowTest(unittest.TestCase):
    """The release workflow's own properties."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.text = WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.document = yaml.safe_load(cls.text)

    def test_the_workflow_holds_the_dry_run_and_the_real_release(self) -> None:
        self.assertIn("dry-run", self.document["jobs"])
        self.assertIn("release", self.document["jobs"])

    def test_no_signing_secret_sits_in_a_job_level_environment(self) -> None:
        self.assertEqual(cosign_secrets_in_a_job_environment(self.document), [])

    def test_every_third_party_action_is_pinned_to_a_commit_sha(self) -> None:
        self.assertEqual(unpinned_actions(self.document), [])

    def test_every_pinned_action_names_its_release_in_a_trailing_comment(self) -> None:
        self.assertEqual(
            actions_without_a_version_comment(self.text, actions(self.document)),
            [],
        )

    def test_the_release_job_hands_the_key_to_the_three_signing_steps(self) -> None:
        for name in SIGNING_STEPS:
            step = step_named(self.document, "release", name)
            env = step.get("env") or {}
            for secret in SECRET_NAMES:
                self.assertEqual(
                    str(env.get(secret)),
                    "${{ secrets." + secret + " }}",
                    f"{name} does not carry {secret}",
                )

    def test_no_other_step_of_the_release_job_sees_a_signing_secret(self) -> None:
        seen = set()
        for step in steps_of(self.document, "release"):
            for value in (step.get("env") or {}).values():
                if "secrets.COSIGN_" in str(value):
                    seen.add(str(step.get("name")))
        self.assertEqual(seen, set(SIGNING_STEPS))

    def test_the_release_job_signs_with_the_key_from_the_environment(self) -> None:
        env = self.document["jobs"]["release"].get("env") or {}
        self.assertEqual(env.get("COSIGN_KEY"), "env://COSIGN_PRIVATE_KEY")
        for name in SIGNING_STEPS:
            step = step_named(self.document, "release", name)
            self.assertIn('--key "$COSIGN_KEY"', str(step.get("run")), name)

    def test_the_dry_run_signs_with_the_key_from_the_environment(self) -> None:
        env = self.document["jobs"]["dry-run"].get("env") or {}
        self.assertEqual(env.get("COSIGN_KEY"), "env://COSIGN_PRIVATE_KEY")
        for name in SIGNING_STEPS:
            step = step_named(self.document, "dry-run", name)
            self.assertIn('--key "$COSIGN_KEY"', str(step.get("run")), name)

    def test_the_dry_run_publishes_its_throwaway_key_as_a_step_output(self) -> None:
        """The key travels as an output, not as the job's environment.

        The real job's signing steps take the key from the `release`
        environment's secrets; the dry run's take it from the step that
        generated it (REL-2 S2 review round 4 SHOULD-FIX 1).
        """
        step = step_named(self.document, "dry-run", "Generate a throwaway signing key")
        run = str(step.get("run"))
        commands = shell_commands(run)
        self.assertIn("$GITHUB_OUTPUT", commands)
        self.assertNotIn("$GITHUB_ENV", commands)
        self.assertTrue(step.get("id"), "the step needs an id: its output carries the key")

    def test_no_step_writes_the_signing_key_into_the_github_environment(self) -> None:
        """REL-2 S2 review round 4 SHOULD-FIX 1: the key is not for every step.

        A variable written to `$GITHUB_ENV` is in the environment of every
        later step of the job, and the runner prints it in each step's
        environment block — the SBOM step's log showed the throwaway key.
        """
        self.assertEqual(steps_writing_the_key_to_the_github_env(self.document), [])

    def test_the_dry_run_hands_its_throwaway_key_to_the_three_signing_steps(self) -> None:
        """The dry run takes the same path as the real job.

        The key is in the `env:` of the three signing steps and of no
        other step: it comes from the step that generated it.
        """
        for name in SIGNING_STEPS:
            step = step_named(self.document, "dry-run", name)
            env = step.get("env") or {}
            self.assertIn("COSIGN_PRIVATE_KEY", env, name)

    def test_the_dry_run_job_environment_holds_no_key(self) -> None:
        """The job's environment holds the reference, not the key."""
        env = self.document["jobs"]["dry-run"].get("env") or {}
        self.assertNotIn("COSIGN_PRIVATE_KEY", env)

    def test_the_sbom_folder_is_created_before_the_first_sbom_step(self) -> None:
        """REL-2 S2 review round 4 BLOCKER 1: the action does not create it.

        `anchore/sbom-action` writes into `sbom/` and fails with ENOENT
        when the folder is not there, so both jobs create it first.
        """
        for job in ("dry-run", "release"):
            self.assertTrue(
                creates_the_sbom_folder(self.document, job),
                f"{job}: sbom/ is not created before the first SBOM step",
            )

    def test_a_workflow_without_the_sbom_folder_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_NO_SBOM_FOLDER)
        self.assertFalse(creates_the_sbom_folder(document, "dry-run"))

    def test_every_sbom_is_written_as_cyclonedx_json(self) -> None:
        """REL-2 S2 review round 5 BLOCKER 1: the predicate must be JSON.

        `anchore/sbom-action` takes syft's format names, and syft writes
        the plain `cyclonedx` name as XML. `cosign attest --type cyclonedx`
        reads a JSON predicate, so with the XML the attestation failed with
        "invalid character '<' looking for beginning of value".
        """
        for job in ("dry-run", "release"):
            steps = [step for name, step in sbom_steps(self.document) if name == job]
            self.assertTrue(steps, f"{job}: no SBOM step to check")
        self.assertEqual(sbom_formats(self.document), [])

    def test_a_workflow_with_an_xml_sbom_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_XML_SBOM)
        self.assertEqual(sbom_formats(document), ["dry-run: The SBOM of the API image"])

    def test_the_manifest_is_signed_into_a_sigstore_bundle_in_both_jobs(self) -> None:
        """REL-2 S2 D3a: the signature is a bundle, not a detached file.

        cosign v3 ignores `--output-signature` (it is deprecated), so a
        workflow that still used it would write no signature at all.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Sign the manifest and verify the signature")
            run = str(step.get("run"))
            self.assertIn(f"--bundle {SIGSTORE_BUNDLE}", run, job)
            self.assertNotIn("--output-signature", run, job)

    def test_the_bundle_is_verified_offline_in_both_jobs(self) -> None:
        """The verification right after signing must not need a log.

        The bundle carries no transparency-log entry (nothing is uploaded
        to Rekor), so the verification skips the log check and runs
        offline.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Sign the manifest and verify the signature")
            run = str(step.get("run"))
            self.assertIn("cosign verify-blob", run, job)
            self.assertIn("--insecure-ignore-tlog", run, job)
            self.assertIn("--offline", run, job)

    def test_no_step_of_either_job_uploads_to_the_transparency_log(self) -> None:
        """REL-2 S2 D3a: nothing is uploaded to the public Rekor log.

        Every cosign command that SIGNS something carries both flags, so
        neither job can publish an entry — the dry run's throwaway key
        least of all. The commands that only make or read a key
        (`generate-key-pair`, `public-key`) upload nothing and are not
        checked.
        """
        signing = re.compile(r"cosign (sign|sign-blob|attest)\b")
        for job in ("dry-run", "release"):
            for step in steps_of(self.document, job):
                run = str(step.get("run") or "")
                if not signing.search(run):
                    continue
                for flag in NO_TLOG_FLAGS:
                    self.assertIn(flag, run, f"{job}: {step.get('name')}: {flag}")

    def test_the_bundle_script_is_given_the_sigstore_bundle(self) -> None:
        """The bundle carries the bundle, under its own name."""
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Build the offline bundle")
            run = str(step.get("run"))
            self.assertIn(f"--sigstore-bundle {SIGSTORE_BUNDLE}", run, job)

    def test_both_jobs_pull_the_third_party_images_by_digest(self) -> None:
        """REL-2 S2 review round 6 BLOCKER 1: the bundle holds EVERY image.

        The third-party images are the ones the bundle was missing: the
        manifest names them, so the pipeline pulls them by digest and
        the bundle saves them.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "The third-party images, by digest")
            run = str(step.get("run"))
            self.assertIn("compose_images.py", run, job)
            self.assertIn("--published-images images.json", run, job)
            self.assertIn("docker pull", run, job)

    def test_a_workflow_without_the_third_party_pull_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_NO_THIRD_PARTY_PULL)
        self.assertEqual(third_party_pull_steps(document), [])

    def test_the_third_party_images_are_pulled_before_the_bundle_is_built(self) -> None:
        for job in ("dry-run", "release"):
            self.assertLess(
                step_index(self.document, job, "The third-party images, by digest"),
                step_index(self.document, job, "Build the offline bundle"),
                job,
            )

    def test_both_jobs_write_an_sbom_for_every_third_party_image(self) -> None:
        """Every image in a release carries an SBOM — the third-party
        ones included. The bundle refuses an image that has none."""
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "The SBOM of each third-party image")
            run = str(step.get("run"))
            self.assertIn("cyclonedx-json", run, job)
            self.assertIn("image_name.sh", run, job)
            self.assertIn("third-party-images.txt", run, job)

    def test_the_syft_the_pipeline_uses_is_the_sbom_action_own(self) -> None:
        """The same scanner, at the same commit, as the SBOM action.

        The action's own `download-syft` sub-action installs the syft
        the action itself uses (v1.42.3), so the SBOMs of the
        third-party images and the SBOMs of ours come from one scanner.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Install syft")
            uses = str(step.get("uses"))
            self.assertIn("anchore/sbom-action/download-syft@", uses, job)
            self.assertTrue(step.get("id"), "the step needs an id: its output carries syft")
            scan = step_named(self.document, job, "The SBOM of each third-party image")
            self.assertIn("steps.syft.outputs.cmd", str(scan.get("env")), job)

    def test_both_jobs_free_the_runner_disk_before_the_images_are_pulled(self) -> None:
        """The bundle holds every image now: the runner needs the room."""
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Free runner disk")
            run = str(step.get("run"))
            self.assertIn("docker image prune", run, job)
            self.assertIn("df -h /", run, job)
            self.assertLess(
                step_index(self.document, job, "Free runner disk"),
                step_index(self.document, job, "The third-party images, by digest"),
                job,
            )

    def test_the_bundle_step_prints_the_disk_before_and_after(self) -> None:
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Build the offline bundle")
            run = str(step.get("run"))
            self.assertEqual(run.count("df -h /"), 2, job)

    def test_the_manifest_is_written_from_the_published_images_and_the_compose_file(self) -> None:
        """The manifest names every image the release runs.

        Ours come from the published digests, the third-party ones from
        the rendered Compose file — so the flag says "published".
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Write the release manifest")
            run = str(step.get("run"))
            self.assertIn("--published-images", run, job)
            self.assertIn("--compose docker-compose.release.yml", run, job)

    def test_both_jobs_build_the_installer(self) -> None:
        """REL-2 S3-T4: the release ships the installer.

        The bundle holds `lakehousectl`, so an air-gapped host can
        install the stack with it, and the release assets hold it too,
        so `install.sh` can download it.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Build lakehousectl (static, musl)")
            run = str(step.get("run"))
            self.assertIn("x86_64-unknown-linux-musl", run, job)
            self.assertIn("-p lakehousectl", run, job)
            self.assertIn("lakehousectl-linux-x86_64", run, job)

    def test_the_release_build_never_accepts_another_key(self) -> None:
        """A release build verifies against the compiled-in key.

        The `test-key` feature points a build at another key, and it
        exists for the CI dry run only: a release that accepted another
        key would install what its operator's host said to install.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Build lakehousectl (static, musl)")
            run = str(step.get("run"))
            self.assertNotIn("test-key", run, job)
            self.assertNotIn("--features", run, job)

    def test_the_manifest_and_the_bundle_carry_the_installer(self) -> None:
        """The signed manifest names the installer's checksum, and the
        bundle carries the file the manifest names."""
        for job in ("dry-run", "release"):
            manifest = step_named(self.document, job, "Write the release manifest")
            self.assertIn(
                "--lakehousectl lakehousectl-linux-x86_64",
                str(manifest.get("run")),
                job,
            )
            bundle = step_named(self.document, job, "Build the offline bundle")
            self.assertIn(
                "--lakehousectl lakehousectl-linux-x86_64",
                str(bundle.get("run")),
                job,
            )

    def test_the_release_attaches_the_bundle_in_parts(self) -> None:
        """REL-7 D1: GitHub refuses a release asset of 2 GiB or more.

        The v0.2.0 bundle was 3,794,642,569 bytes, and the release failed
        with "size must be less than 2147483648". So the bundle is split
        at 1900 MiB — the margin under the limit — and the PARTS are the
        assets. The whole file's `.sha256` stays one, because the joined
        file must match it (D2).
        """
        self.assertEqual(bundle_parts_problems(self.document, "release"), [])

    def test_a_release_that_attaches_the_whole_bundle_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_WHOLE_BUNDLE_AS_AN_ASSET)
        self.assertNotEqual(bundle_parts_problems(document, "release"), [])

    def test_the_dry_run_proves_the_split_round_trips(self) -> None:
        """REL-7 D4: the split is proved before a tag can need it.

        The dry run never caught the size limit, because it builds the
        bundle and attaches no release asset. It now makes the parts the
        release makes, checks each against `SHA256SUMS`, joins them in
        name order, and checks the joined file against the bundle's own
        `.sha256` — the operator's commands (D2).
        """
        self.assertEqual(round_trip_problems(self.document, "dry-run"), [])

    def test_a_round_trip_that_never_checks_the_join_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_ROUND_TRIP_WITHOUT_THE_CHECK)
        self.assertNotEqual(round_trip_problems(document, "dry-run"), [])

    def test_the_round_trip_comes_after_the_bundle_is_built(self) -> None:
        """A split is worth nothing before the file it splits."""
        self.assertLess(
            step_index(self.document, "dry-run", "Build the offline bundle"),
            step_index(self.document, "dry-run", ROUND_TRIP_STEP),
        )

    def test_the_draft_release_can_be_written_again(self) -> None:
        """REL-7 D5: the v0.2.0 run left a draft, and the job re-runs.

        A draft that already exists for the tag is written into with
        `gh release upload --clobber`, and a PUBLISHED release is
        refused: it is what an operator installed.
        """
        self.assertEqual(draft_reuse_problems(self.document, "release"), [])

    def test_a_create_only_draft_step_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_CREATE_ONLY)
        self.assertNotEqual(draft_reuse_problems(document, "release"), [])

    def test_the_release_job_publishes_the_files_the_release_ships(self) -> None:
        """REL-2 S4 decision D14: the release ships every file its
        Compose file mounts.

        The ONLINE install fetches a shipped file by its own name, so
        each of them is published as an asset of its own. The list
        comes from the script that wrote the manifest, so the release
        and the manifest cannot drift apart.
        """
        step = step_named(self.document, "release", "The release's assets")
        run = str(step.get("run"))
        self.assertIn("--print-shipped-files", run)
        self.assertIn("make_manifest.py", run)
        self.assertIn("release-assets/", run)

    def test_the_bundle_of_both_jobs_carries_the_shipped_files(self) -> None:
        """The bundle holds every file the release ships, at the path
        the install writes it at.

        The bundle script asks the manifest's own script for the list,
        so the bundle and the manifest name the same files.
        """
        for job in ("dry-run", "release"):
            bundle = step_named(self.document, job, "Build the offline bundle")
            run = str(bundle.get("run"))
            self.assertIn("ops/release/make_bundle.sh", run, job)

    def test_the_installer_is_built_before_the_manifest_is_written(self) -> None:
        for job in ("dry-run", "release"):
            self.assertLess(
                step_index(self.document, job, "Build lakehousectl (static, musl)"),
                step_index(self.document, job, "Write the release manifest"),
                job,
            )

    def test_the_dry_run_uploads_the_throwaway_public_key(self) -> None:
        """The proof job verifies the manifest against the dry run's key.

        The dry run signs its manifest with a throwaway key, so the
        proof needs the PUBLIC half of it: the private half was thrown
        away with the job.
        """
        step = step_named(self.document, "dry-run", "Upload the bundle and the manifest")
        self.assertIn("cosign.pub", str(step.get("with", {}).get("path")))

    def test_the_proof_job_installs_the_dry_runs_bundle(self) -> None:
        """REL-2 S3-T5: the end-to-end proof.

        The job takes the dry run's bundle — the bytes an air-gapped
        host would be handed — and installs it on a clean runner.
        """
        steps = steps_of(self.document, "install-proof")
        names = [str(step.get("name")) for step in steps]
        self.assertIn("Download the dry run's bundle", names)
        self.assertIn("Install the bundle", names)
        self.assertIn("The stack is ready and the console answers", names)

        install = step_named(self.document, "install-proof", "Install the bundle")
        run = str(install.get("run"))
        self.assertIn("--bundle", run)
        self.assertIn("lakehousectl install", run)
        self.assertIn("LAKEHOUSECTL_PUBLIC_KEY_FILE", str(install.get("env")))

    def test_the_proof_job_builds_with_the_test_key_feature(self) -> None:
        """The proof's bundle is signed with the throwaway key.

        So the proof's binary is built with the feature that points a
        build at another key — and the RELEASE job's build is not
        (checked above).
        """
        step = step_named(self.document, "install-proof", "Build lakehousectl (the dry run's key)")
        run = str(step.get("run"))
        self.assertIn("--features test-key", run)

    def test_the_proof_job_needs_the_dry_run(self) -> None:
        """The proof installs what the dry run built."""
        job = self.document["jobs"]["install-proof"]
        self.assertEqual(job["needs"], "dry-run")

    def test_the_proof_asserts_that_the_dagster_services_run(self) -> None:
        """REL-2 S4b review round 3 BLOCKER 3, decision D22.

        The Dagster services carry the `dagster` profile, and a stack
        that does not run it runs no pipelines at all. The proof asserts
        that the daemon and the code location run after the install.
        """
        self.assertEqual(
            dagster_assertion_problems(self.document, "install-proof", DAGSTER_STEP),
            [],
        )

    def test_the_proof_reads_the_stack_with_the_installs_own_env(self) -> None:
        """The install's `.env` names the profiles the install runs.

        The compose call reads the stack with that file, so the Dagster
        services are part of the deployment the command reads.
        """
        step = step_named(self.document, "install-proof", DAGSTER_STEP)
        self.assertIn(INSTALL_ENV, str(step.get("run")))

    def test_a_proof_without_the_dagster_assertion_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_NO_DAGSTER_ASSERTION)
        self.assertNotEqual(
            dagster_assertion_problems(document, "install-proof", DAGSTER_STEP),
            [],
        )

    def test_a_proof_that_reads_the_stack_without_the_installs_env_is_reported(
        self,
    ) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_DAGSTER_WITHOUT_THE_INSTALL_ENV)
        problems = dagster_assertion_problems(
            document, "install-proof", DAGSTER_STEP
        )
        self.assertTrue(
            any("does not assert dagster-daemon" in problem for problem in problems),
            problems,
        )
        self.assertTrue(
            any(
                "does not read the install's own .env" in problem
                for problem in problems
            ),
            problems,
        )

    def test_the_image_ids_are_the_config_digests_on_every_docker_store(self) -> None:
        """The two Docker stores identify an image differently.

        The classic store reports the CONFIG digest as the image's ID;
        the containerd store reports the image INDEX digest. The
        manifest names the config digest — the identity a LOADED image
        carries on both stores — so the pipeline corrects the ID when
        the store reports the index digest, and reads the config digest
        from the image's own saved tar.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "The image ids")
            run = str(step.get("run"))
            self.assertIn("docker image inspect --format '{{.Id}}'", run, job)
            self.assertIn("docker save", run, job)
            self.assertIn("manifest.json", run, job)

    def test_a_job_level_secret_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_JOB_SECRET)
        self.assertEqual(
            cosign_secrets_in_a_job_environment(document),
            ["release.COSIGN_PASSWORD"],
        )

    def test_a_step_level_secret_is_not_a_job_level_one(self) -> None:
        document = yaml.safe_load(SYNTHETIC_STEP_SECRET)
        self.assertEqual(cosign_secrets_in_a_job_environment(document), [])

    def test_an_action_pinned_to_a_moving_tag_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_MOVING_TAG)
        self.assertEqual(unpinned_actions(document), ["anchore/sbom-action@v0"])

    def test_a_key_in_the_github_environment_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_KEY_IN_THE_GITHUB_ENV)
        self.assertEqual(
            steps_writing_the_key_to_the_github_env(document),
            ["dry-run: Generate a throwaway signing key"],
        )

    def test_every_musl_target_is_added_on_the_pinned_toolchain(self) -> None:
        """PR #313 dry run: the target must land on the PINNED toolchain.

        `rustup` reads the pin from the `rust-toolchain.toml` in the
        current directory or above it, so the target has to be added
        from inside `rust/`. Added at the repository root it went to the
        runner's default toolchain, and the build on the pinned
        toolchain (1.99.1) failed with "can't find crate for core ...
        the x86_64-unknown-linux-musl target may not be installed".
        """
        steps = musl_target_steps(self.document)
        self.assertTrue(steps, "no step adds the musl target")
        for job_name, step_name in steps:
            step = step_named(self.document, job_name, step_name)
            self.assertTrue(
                adds_the_musl_target_inside_rust(step),
                f"{job_name}: {step_name}: the musl target is added before `cd rust`",
            )

    def test_a_musl_target_added_outside_rust_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_MUSL_TARGET_OUTSIDE_RUST)
        for job_name, step_name in musl_target_steps(document):
            step = step_named(document, job_name, step_name)
            self.assertFalse(
                adds_the_musl_target_inside_rust(step),
                f"{job_name}: {step_name}: the musl target is added before `cd rust`",
            )

    def test_the_toolchain_pin_does_not_ask_for_the_musl_target(self) -> None:
        """The musl target is a release build's need, not every dev's.

        A target in `rust-toolchain.toml` is downloaded by every
        developer and by every CI job that touches Rust — for a build
        they do not make. The release workflow adds the target itself,
        on the pinned toolchain (PR #313 dry run).
        """
        text = TOOLCHAIN_PATH.read_text(encoding="utf-8")
        self.assertNotIn(MUSL_TARGET, text)

    def test_every_file_the_proof_reads_is_on_the_upload_list(self) -> None:
        """PR #313 install-proof: the artifact keeps the relative paths.

        `upload-artifact` stores every file at its path relative to
        the root of the upload list — the repository root, because that
        list names files from `dist/`, from `sbom/` and from the root
        itself. So the bundle the dry run wrote into `dist/` is read
        back from `dry-run/dist/`, and the proof reading it from
        `dry-run/` failed with "tar (child):
        dry-run/rantai-lakehouse-*.tar.zst: Cannot open: No such file or
        directory".
        """
        self.assertTrue(upload_entries(self.document), "no upload list to check")
        self.assertTrue(dry_run_reads(self.document), "the proof reads no file")
        self.assertEqual(reads_without_an_uploaded_file(self.document), [])

    def test_a_proof_reading_a_file_the_dry_run_does_not_upload_is_reported(
        self,
    ) -> None:
        """The check above must be able to fail.

        The synthetic workflow reads the bundle from the artifact's
        root, although the dry run uploaded it from `dist/`.
        """
        document = yaml.safe_load(SYNTHETIC_PROOF_READS_FROM_THE_ROOT)
        self.assertEqual(
            reads_without_an_uploaded_file(document),
            ["dry-run/rantai-lakehouse-*.tar.zst"],
        )

    def test_the_proof_downloads_the_artifact_the_dry_run_uploaded(self) -> None:
        """The two jobs must name the SAME artifact.

        The upload step writes the artifact under one name and the
        download step fetches it under another: the download finds
        nothing, and the proof fails before it reads a file.
        """
        upload = step_named(self.document, "dry-run", UPLOAD_STEP)
        download = step_named(self.document, "install-proof", DOWNLOAD_STEP)
        self.assertEqual(
            str((upload.get("with") or {}).get("name")),
            str((download.get("with") or {}).get("name")),
        )

    def test_the_proof_unpacks_the_artifact_where_it_reads_it(self) -> None:
        """The folder the proof reads from is the folder it unpacked into.

        A read from a folder the artifact was not unpacked into is a
        read of a file that is not there.
        """
        download = step_named(self.document, "install-proof", DOWNLOAD_STEP)
        self.assertEqual(str((download.get("with") or {}).get("path")), DRY_RUN_FOLDER)

    def test_the_containerd_image_store_is_on_before_the_first_pull_or_build(self) -> None:
        """REL-2 S3 D13: the bundle is saved from the containerd store.

        The store has to be on before the job pulls or builds anything:
        an image that lands in the classic store is an image whose
        archive carries no digest an air-gapped host could resolve.
        """
        for job in ("dry-run", "release"):
            self.assertEqual(containerd_too_late(self.document, job), [], job)

    def test_a_job_that_pulls_before_the_store_is_on_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_PULL_BEFORE_THE_STORE)
        self.assertEqual(containerd_too_late(document, "dry-run"), ["dry-run"])

    def test_every_job_turns_the_store_on_merges_the_configuration_and_asserts_it(self) -> None:
        """The step does the whole job: merge, restart, assert.

        The daemon's own configuration is merged rather than replaced,
        Docker is restarted for the setting to take, and the store is
        asserted — a step that turned the feature on and hoped is a
        step that can build a bundle nobody can install.
        """
        for job in ("dry-run", "release", "install-proof"):
            self.assertEqual(containerd_step_problems(self.document, job), [], job)

    def test_the_proof_turns_the_containerd_image_store_on_before_the_install(self) -> None:
        """The proof installs a bundle, so the store has to be on first."""
        store = step_index_or_none(self.document, "install-proof", CONTAINERD_STEP)
        self.assertIsNotNone(store, "the proof never turns the containerd store on")
        install = step_index(self.document, "install-proof", "Install the bundle")
        self.assertLess(store, install)

    def test_the_proof_cuts_the_runner_off_from_the_registries_before_the_install(self) -> None:
        """REL-2 S3 D13: the proof is air-gapped IN FACT.

        Compose pulls silently when it may, so a runner with a path to
        the registries would fetch the images the bundle should have
        loaded — and the bug would stay hidden.
        """
        self.assertEqual(air_gap_problems(self.document), [])

    def test_a_proof_that_installs_before_the_cut_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_INSTALL_BEFORE_THE_CUT)
        problems = air_gap_problems(document)
        self.assertIn("the proof installs before the runner is cut off", problems)

    def test_no_job_loses_a_service_container_to_the_store_switch(self) -> None:
        """REL-2 S3 review round 4 BLOCKER 1: the dry run lost its registry.

        A service container is started before the job's first step, and
        the containerd step restarts Docker — which stops the service
        container and hides the image it ran from. The dry run's
        registry was such a service, so the image builds that push to
        `localhost:5000` had no registry to push to.
        """
        for job in ("dry-run", "release", "install-proof"):
            self.assertEqual(
                services_killed_by_the_store_switch(self.document, job), [], job
            )

    def test_a_service_container_across_the_restart_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_SERVICE_ACROSS_THE_RESTART)
        self.assertEqual(
            services_killed_by_the_store_switch(document, "dry-run"),
            ["registry"],
        )

    def test_a_service_started_after_the_switch_is_not_reported(self) -> None:
        """A service a step starts AFTER the switch is not lost."""
        document = yaml.safe_load(SYNTHETIC_SERVICE_STARTED_AFTER_THE_SWITCH)
        self.assertEqual(
            services_killed_by_the_store_switch(document, "dry-run"), []
        )

    def test_the_dry_run_starts_its_registry_after_the_store_switch(self) -> None:
        """The registry is a container of the job, not a service.

        The job starts it itself, after the store switch, and waits for
        it to answer: the first build may start before the registry is
        up, and a push to a registry that is not up yet fails.
        """
        step = step_named(self.document, "dry-run", REGISTRY_STEP)
        run = str(step.get("run"))
        self.assertLess(
            step_index(self.document, "dry-run", CONTAINERD_STEP),
            step_index(self.document, "dry-run", REGISTRY_STEP),
            "the registry is started before the store switch",
        )
        self.assertIn("docker run -d --name release-registry", run)
        self.assertIn(REGISTRY_API, run)
        self.assertIn("exit 1", run, "the wait for the registry is not bounded")

    def test_the_dry_runs_registry_is_pinned_by_digest(self) -> None:
        """A tag can move under the pipeline's feet; a digest cannot."""
        run = str(step_named(self.document, "dry-run", REGISTRY_STEP).get("run"))
        self.assertIn("registry:2@sha256:", run)

    def test_the_dry_run_pushes_to_the_registry_it_started(self) -> None:
        """The registry the job starts is the registry the job pushes to."""
        env = self.document["jobs"]["dry-run"].get("env") or {}
        self.assertEqual(env.get("RELEASE_REGISTRY"), DRY_RUN_REGISTRY)

    def test_the_console_image_does_not_carry_the_landing_page(self) -> None:
        """WEB-2: an installed Lakehouse does not serve `/landing`.

        The product owner decided on 2026-11-02 that the landing page is a
        separate product page and that an install must not offer it. The
        console image is built from the repository ROOT, so the site that
        holds the page (`site/`, WEB-5) has to be out of its build context —
        otherwise every console image (the release's, the staging
        deploy's, compose's) carries it.
        """
        text = DOCKERIGNORE_PATH.read_text(encoding="utf-8")
        for path in LANDING_PATHS:
            self.assertTrue(
                dockerignore_excludes(text, path),
                f"the console image would carry {path}",
            )

    def test_a_build_context_that_carries_the_landing_page_is_reported(self) -> None:
        """The check above must be able to fail."""
        text = "node_modules\n.next\n"
        for path in LANDING_PATHS:
            self.assertFalse(dockerignore_excludes(text, path), path)

    def test_a_later_negation_would_undo_the_exclusion(self) -> None:
        """Docker's last match wins, and `!` re-includes."""
        text = "site/\n!site\n"
        self.assertFalse(dockerignore_excludes(text, "site"))

    def test_a_change_to_the_build_context_runs_the_dry_run(self) -> None:
        """`.dockerignore` decides what the console image is made of.

        A PR that edits it changes the image the dry run builds and the
        install proof checks, so the workflow has to run for it (WEB-2).
        """
        self.assertIn(".dockerignore", pull_request_paths(self.document))

    def test_a_workflow_without_the_build_context_in_its_paths_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_WITHOUT_THE_BUILD_CONTEXT_IN_THE_PATHS)
        self.assertNotIn(".dockerignore", pull_request_paths(document))

    def test_the_install_proof_checks_that_the_landing_page_is_not_served(self) -> None:
        """WEB-2: the proof is on an installed bundle, not on a build.

        The page must answer 404 there, and the job fails when it does
        not: a 200 means the page is back in the install.
        """
        self.assertEqual(landing_proof_problems(self.document), [])

    def test_a_proof_that_never_asks_about_the_landing_page_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_PROOF_WITHOUT_THE_LANDING_PAGE)
        self.assertIn(
            "the proof does not ask for the landing page's status",
            landing_proof_problems(document),
        )



class UpdateProofTest(unittest.TestCase):
    """The update proof: the job, and the rules it runs on.

    REL-2 S4 decision D21. The proof installs the dry run's bundle A,
    seeds three markers, updates to a bundle B whose API image is
    REBUILT with a bumped version, and updates to a bundle C whose API
    image never becomes ready: that update must fail and roll itself
    back, and every marker must still be there.
    """

    def setUp(self) -> None:
        self.document = load_workflow()

    # ── The job ───────────────────────────────────────────────────────

    def test_the_update_proof_needs_the_dry_run(self) -> None:
        """The proof installs what the dry run built."""
        self.assertEqual(self.document["jobs"]["update-proof"]["needs"], "dry-run")

    def test_the_update_proof_installs_the_dry_runs_bundle(self) -> None:
        """Step 1 of D21: bundle A is the dry run's bundle."""
        install = step_named(self.document, "update-proof", "Install bundle A")
        run = str(install.get("run"))
        self.assertIn("--bundle bundle-a", run)
        self.assertIn("lakehousectl install", run)
        self.assertIn(
            "dry-run/cosign.pub",
            str(install.get("env")),
            "bundle A is signed by the dry run's key, and the tool must trust that key",
        )

    def test_the_update_proof_builds_with_the_test_key_feature(self) -> None:
        """The proof signs its own bundles with its own key."""
        step = step_named(self.document, "update-proof", "Build lakehousectl (the proof's key)")
        self.assertIn("--features test-key", str(step.get("run")))

    def test_the_update_proof_builds_its_images_before_the_runner_is_cut_off(self) -> None:
        """The builder pulls its base images from the registries, so
        the image builds run BEFORE the cut — the runner is cut off
        before the INSTALL, which is where the air-gap matters."""
        cut = step_index(self.document, "update-proof", "Cut the runner off from the registries")
        for step in (
            "Build and push the API image of bundle B",
            "Build and push the API image of bundle C",
        ):
            self.assertLess(
                step_index(self.document, "update-proof", step),
                cut,
                f"{step} runs after the cut",
            )

    def test_the_update_skips_the_checks_the_runner_cannot_meet(self) -> None:
        """The runner is below the release's minimums, so the two
        checks are skipped — and the tool says which checks it
        skipped."""
        run = str(
            step_named(self.document, "update-proof", "The update to bundle B succeeds").get("run")
        )
        self.assertIn("--ignore-preflight disk", run)
        self.assertIn("--ignore-preflight memory", run)

    def test_the_update_proof_cuts_the_runner_off_before_the_install(self) -> None:
        """The proof is air-gapped, as `install-proof` is."""
        self.assertLess(
            step_index(self.document, "update-proof", "Cut the runner off from the registries"),
            step_index(self.document, "update-proof", "Install bundle A"),
            "the install runs before the runner is cut off",
        )

    def test_the_update_proof_seeds_the_three_markers(self) -> None:
        """Step 2 of D21: a row in the application database, a row in
        ClickHouse, and a stored connector credential through the API."""
        run = str(step_named(self.document, "update-proof", "Seed the markers").get("run"))
        self.assertIn("s4_update_marker", run)
        self.assertIn("postgres", run)
        self.assertIn("clickhouse", run)
        self.assertIn("/api/connectors", run)
        self.assertIn("credential", run)

    def test_the_update_proof_builds_bundle_b_with_the_api_image_rebuilt(self) -> None:
        """Step 3 of D21: the API image is rebuilt with a bumped
        version, and every other image is reused from A."""
        run = str(
            step_named(self.document, "update-proof", "The API's version is bumped for bundle B").get(
                "run"
            )
        )
        self.assertIn("lakehouse-api/Cargo.toml", run)
        self.assertIn("Cargo.lock", run)
        assembled = str(
            step_named(self.document, "update-proof", "The bundle B is assembled and signed").get(
                "run"
            )
        )
        self.assertIn("update_proof.py bundle", assembled)
        self.assertIn("--from bundle-a --to bundle-b", assembled)
        self.assertIn(
            "images/rantai-lakehouse-api.tar",
            assembled,
            "the rebuilt image's archive replaces the old one's",
        )

    def test_the_update_proof_bumps_the_api_version_with_a_tested_script(self) -> None:
        """The bump is a script of its own, so it can be tested.

        It was inline Python in this step, and it was wrong (PR #335,
        the `update-proof` job): the lockfile's version was replaced by
        its PREFIX, so the old version text stayed behind and `cargo
        chef prepare` stopped with a TOML parse error. The script that
        replaces it is `ops/release/bump_api_version.py`, and its own
        tests live in `ops/release/test_bump_api_version.py`.
        """
        run = str(
            step_named(self.document, "update-proof", "The API's version is bumped for bundle B").get(
                "run"
            )
        )
        self.assertIn("ops/release/bump_api_version.py", run)
        self.assertNotIn(
            "python3 - ",
            run,
            "the bump must be the tested script, not inline Python",
        )

    def test_the_update_proof_signs_its_own_bundles(self) -> None:
        """The bundles the proof builds are signed with the proof's
        key, and the tool is pointed at that key for the updates."""
        step = step_named(self.document, "update-proof", "The bundle B is assembled and signed")
        self.assertIn("cosign sign-blob", str(step.get("run")))
        update = step_named(self.document, "update-proof", "The update to bundle B succeeds")
        self.assertIn(
            "proof.pub",
            str(update.get("env")),
            "the update must verify against the proof's key",
        )

    def test_the_update_to_bundle_b_must_succeed(self) -> None:
        """The update to bundle B reports the new version as ready."""
        run = str(step_named(self.document, "update-proof", "The update to bundle B succeeds").get("run"))
        self.assertIn("lakehousectl update --bundle bundle-b", run)
        self.assertIn("/health/ready", run)

    def test_the_update_proof_tags_every_image_it_saves_by_tag(self) -> None:
        """Every image the proof pulls by digest is TAGGED before it is
        saved.

        Docker's containerd image store does not register the tag when
        the pull is by digest: the digest reference and the tag are two
        names for one image, and only the digest one is registered. A
        `docker save` of the tag then fails with "No such image", and
        the bundle's archive is never written (PR #335 update-proof,
        bundle B; REL-2 S3 D13).
        """
        problems = untagged_saves(self.document, "update-proof")
        self.assertEqual(problems, [], "\n".join(problems))

    def test_a_proof_that_saves_a_tag_without_tagging_it_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_PROOF_SAVES_A_TAG_WITHOUT_TAGGING_IT)
        self.assertNotEqual(untagged_saves(document, "update-proof"), [])
        tagged = yaml.safe_load(SYNTHETIC_PROOF_TAGS_BEFORE_IT_SAVES)
        self.assertEqual(untagged_saves(tagged, "update-proof"), [])

    def test_the_update_to_bundle_c_must_fail_and_roll_back(self) -> None:
        """Step 4 of D21: the update to the bundle that never becomes
        ready exits non-zero, and the rollback is reported."""
        run = str(
            step_named(self.document, "update-proof", "The update to bundle C fails and rolls back").get(
                "run"
            )
        )
        self.assertIn("lakehousectl update --bundle bundle-c", run)
        self.assertIn("--ready-timeout 180", run)
        self.assertIn("rolled back", run)

    def test_the_update_proof_checks_the_markers_after_the_update(self) -> None:
        """A marker that is gone after the update is data the update
        lost."""
        run = str(
            step_named(
                self.document,
                "update-proof",
                "The markers are still there after the update to bundle B",
            ).get("run")
        )
        self.assertIn("the postgres marker", run)
        self.assertIn("the clickhouse marker", run)
        self.assertIn("credentialManaged", run)

    def test_the_update_proof_checks_the_markers_after_the_rollback(self) -> None:
        """The same three markers, after the rollback of bundle C."""
        run = str(
            step_named(
                self.document,
                "update-proof",
                "The stack runs bundle B again, and the markers are still there",
            ).get("run")
        )
        self.assertIn("the postgres marker", run)
        self.assertIn("the clickhouse marker", run)
        self.assertIn("credentialManaged", run)

    def test_the_update_proof_counts_the_marker_rows_after_the_rollback(self) -> None:
        """Each marker table holds EXACTLY ONE row after the rollback.

        The rollback RESTORES the backup onto the data the failed
        update left behind, and a restore that MERGES instead of
        replacing leaves the marker of both: the row of the backup and
        the row the failed update wrote (REL-2 S4b review round 6
        BLOCKER 1, decision D24). A marker that is still there is not
        enough — the row count is what says the data was replaced.
        """
        run = str(
            step_named(
                self.document,
                "update-proof",
                "The stack runs bundle B again, and the markers are still there",
            ).get("run")
        )
        self.assertIn("SELECT count(*) FROM s4_update_marker", run)
        self.assertIn("SELECT count() FROM s4_update_marker", run)
        self.assertIn('"$postgres_rows" != "1"', run)
        self.assertIn('"$clickhouse_rows" != "1"', run)

    # ── The session the API calls carry ──────────────────────────────

    def test_every_api_call_of_the_update_proof_carries_a_session(self) -> None:
        """A call to the API with no session answers 401.

        The marker check that ran after the REAL update to bundle B
        failed on it (PR #335 update-proof, marker check 401): it read
        the stored connector credential with a bare `curl`, and the API
        refused the call. Every call of the proof now goes through the
        authenticated helper, which signs in and keeps the session in a
        cookie jar.
        """
        problems = api_calls_without_a_session(self.document, "update-proof")
        self.assertEqual(problems, [], "\n".join(problems))

    def test_a_proof_that_calls_the_api_without_a_session_is_reported(self) -> None:
        """The check above must be able to fail."""
        document = yaml.safe_load(SYNTHETIC_PROOF_CALLS_THE_API_WITHOUT_A_SESSION)
        self.assertNotEqual(api_calls_without_a_session(document, "update-proof"), [])
        with_jar = yaml.safe_load(SYNTHETIC_PROOF_CALLS_THE_API_WITH_THE_COOKIE_JAR)
        self.assertEqual(api_calls_without_a_session(with_jar, "update-proof"), [])

    def test_the_marker_checks_read_the_credential_through_the_helper(self) -> None:
        """The two marker checks call the API through the helper.

        The helper signs in per call: the update restarts the stack, and
        the rollback restarts it again, so no step may depend on the
        session of the step before it.
        """
        for name in MARKER_STEPS:
            run = str(step_named(self.document, "update-proof", name).get("run"))
            self.assertIn(PROOF_API_HELPER, run, name)

    def test_the_helper_signs_in_and_calls_the_api_with_its_session(self) -> None:
        """The helper is what makes the call authenticated.

        It signs in, it keeps the session in a cookie jar, and it makes
        the call with the jar.
        """
        script = (RELEASE_DIR / "proof_api.sh").read_text(encoding="utf-8")
        problems = helper_problems(script)
        self.assertEqual(problems, [], "\n".join(problems))

    def test_a_helper_that_calls_the_api_without_its_session_is_reported(self) -> None:
        """The check above must be able to fail."""
        self.assertNotEqual(helper_problems(SYNTHETIC_HELPER_WITHOUT_A_SESSION), [])

    # ── The tenant the connector calls act as ─────────────────────────

    def test_every_connector_call_of_the_update_proof_names_the_same_tenant(self) -> None:
        """The create and the two reads must act as ONE tenant.

        The API's connector routes are tenant-scoped (`SEC-50`,
        `rust/crates/lakehouse-api/src/tenant_scope.rs`): a connector
        belongs to a tenant, and `GET /api/connectors/{id}` answers 404
        for a caller that does not belong to it. The read of the marker
        connector answered 404 after the real update to bundle B, because
        the connector had no tenant at all — the bootstrap administrator
        belongs to no tenant, so the create fell back to the typed name
        and the row's `tenant_id` stayed NULL (PR #335 update-proof,
        connector 404).

        Every call therefore NAMES the tenant it acts as, through the
        `X-Tenant` header the console sends on every request, and they
        all name the same one.
        """
        problems = connector_tenant_problems(self.document, "update-proof")
        self.assertEqual(problems, [], "\n".join(problems))

    def test_a_proof_whose_connector_calls_name_two_tenants_is_reported(self) -> None:
        """The check above must be able to fail."""
        two_tenants = yaml.safe_load(SYNTHETIC_PROOF_CALLS_THE_CONNECTORS_IN_TWO_TENANTS)
        self.assertNotEqual(connector_tenant_problems(two_tenants, "update-proof"), [])
        no_tenant = yaml.safe_load(SYNTHETIC_PROOF_CALLS_THE_CONNECTOR_WITHOUT_A_TENANT)
        self.assertNotEqual(connector_tenant_problems(no_tenant, "update-proof"), [])

    def test_the_connector_calls_act_as_the_tenant_the_seeding_step_created(self) -> None:
        """The tenant every call names is the one the seed made.

        The steps of a job are separate shells, so the tenant's id is
        written to a file by the seeding step and read back from there by
        every step that calls the API.
        """
        problems = connector_tenant_file_problems(self.document, "update-proof")
        self.assertEqual(problems, [], "\n".join(problems))

    def test_the_seeding_step_creates_the_tenant_and_the_membership(self) -> None:
        """The tenant exists, and the administrator is a member of it.

        A connector in a tenant the caller does not belong to is a 404
        (`SEC-50`), so the proof needs both: the tenant, and the
        administrator's membership of it — the console's Identity → Users
        page does exactly this, through `PUT /api/identity/users/{id}/
        tenants/{tenant_id}`.
        """
        run = str(step_named(self.document, "update-proof", "Seed the markers").get("run"))
        self.assertIn("INSERT INTO tenant", run)
        self.assertIn(f"> {TENANT_FILE}", run)
        self.assertIn("ops/release/proof_api.sh put", run)
        self.assertIn("/api/identity/users/", run)
        self.assertIn("/tenants/", run)

    def test_the_helper_sends_the_tenant_the_api_reads(self) -> None:
        """The helper sends `X-Tenant`, the header the API reads.

        The API's tenant-scoped routes resolve the tenant from the
        `X-Tenant` header (`tenant_scope::resolve`), and the console
        sends the active tenant on every request (`src/services/http.ts`).
        The helper sends it when `PROOF_TENANT` names a tenant — and no
        header at all when it does not, so a call that names no tenant
        acts as the caller's first tenant, the way the API resolves it.
        """
        script = (RELEASE_DIR / "proof_api.sh").read_text(encoding="utf-8")
        problems = helper_tenant_problems(script)
        self.assertEqual(problems, [], "\n".join(problems))

    def test_a_helper_that_names_no_tenant_is_reported(self) -> None:
        """The check above must be able to fail."""
        self.assertNotEqual(helper_tenant_problems(SYNTHETIC_HELPER_WITHOUT_A_TENANT), [])

    def test_the_update_proof_says_why_the_real_previous_release_test_is_not_here(self) -> None:
        """The real "previous release" test joins this job once 0.2.0
        exists (REL-2 S5), and the reason is written in the job."""
        text = WORKFLOW_PATH.read_text(encoding="utf-8")
        self.assertIn("joins this job once 0.2.0 exists", text)

    def test_the_update_proof_asserts_that_the_dagster_services_run(self) -> None:
        """REL-2 S4b review round 3 BLOCKER 3, decision D22.

        The three points of the proof: after the install, after the
        update, and after the rollback. A stack that stops running the
        Dagster services runs no pipelines at all, and the rollback is
        the point where that is most likely to happen.
        """
        for name in DAGSTER_STEPS:
            self.assertEqual(
                dagster_assertion_problems(self.document, "update-proof", name),
                [],
                name,
            )

    def test_the_dagster_assertions_come_after_the_three_points(self) -> None:
        """The assertion is worth nothing before the thing it asserts."""
        for point, assertion in (
            ("Install bundle A", DAGSTER_STEPS[0]),
            ("The update to bundle B succeeds", DAGSTER_STEPS[1]),
            ("The update to bundle C fails and rolls back", DAGSTER_STEPS[2]),
        ):
            self.assertLess(
                step_index(self.document, "update-proof", point),
                step_index(self.document, "update-proof", assertion),
                assertion,
            )

    def test_the_update_proof_reads_the_stack_with_the_installs_own_env(self) -> None:
        """The install's `.env` names the profiles the install runs.

        The compose call reads the stack with that file, so the Dagster
        services are part of the deployment the command reads.
        """
        for name in DAGSTER_STEPS:
            step = step_named(self.document, "update-proof", name)
            self.assertIn(INSTALL_ENV, str(step.get("run")), name)

    # ── The rules the job runs on ────────────────────────────────────

    def test_the_version_of_a_proof_bundle_is_the_release_and_the_step(self) -> None:
        """D21: bundle B is `V-update-proof.1` and bundle C is
        `V-update-proof.2`."""
        self.assertEqual(
            update_proof.proof_version("0.1.0", 1), "0.1.0-update-proof.1"
        )
        self.assertEqual(
            update_proof.proof_version("0.1.0", 2), "0.1.0-update-proof.2"
        )

    def test_a_proof_bundle_reuses_every_image_but_the_api(self) -> None:
        """D21: "every other image reused from A"."""
        manifest = {
            "images": {
                "lakehouse-api": "localhost:5000/rantai-lakehouse-api:0.1.0@sha256:aaa",
                "console": "localhost:5000/rantai-lakehouse-console:0.1.0@sha256:bbb",
                "dagster": "localhost:5000/rantai-lakehouse-dagster:0.1.0@sha256:ccc",
            }
        }
        self.assertEqual(
            update_proof.reused_images(manifest),
            {
                "console": "localhost:5000/rantai-lakehouse-console:0.1.0@sha256:bbb",
                "dagster": "localhost:5000/rantai-lakehouse-dagster:0.1.0@sha256:ccc",
            },
        )
        self.assertEqual(
            update_proof.api_reference(manifest),
            "localhost:5000/rantai-lakehouse-api:0.1.0@sha256:aaa",
        )

    def test_the_compose_file_of_a_proof_bundle_names_the_new_image(self) -> None:
        """The reference is pinned by digest, and the rebuilt image has
        another digest."""
        old = "localhost:5000/rantai-lakehouse-api:0.1.0@sha256:aaa"
        new = "localhost:5000/rantai-lakehouse-api:0.1.0-update-proof.1@sha256:bbb"
        compose = f"services:\n  lakehouse-api:\n    image: {old}\n"
        self.assertEqual(
            update_proof.compose_with(compose, old, new),
            f"services:\n  lakehouse-api:\n    image: {new}\n",
        )

    def test_a_compose_file_that_does_not_name_the_old_image_is_refused(self) -> None:
        """A file that does not name the old reference would run the
        old image."""
        with self.assertRaises(update_proof.ProofError):
            update_proof.compose_with("services: {}\n", "one", "two")

    def test_the_manifest_of_a_proof_bundle_names_the_new_version_and_the_new_image(self) -> None:
        """Everything else is the manifest of bundle A: the other
        images, and the other files."""
        old = "localhost:5000/rantai-lakehouse-api:0.1.0@sha256:aaa"
        new = "localhost:5000/rantai-lakehouse-api:0.1.0-update-proof.1@sha256:bbb"
        manifest = {
            "schemaVersion": 1,
            "version": "0.1.0",
            "gitSha": "a" * 40,
            "createdAt": "2026-11-02T00:00:00Z",
            "minFrom": "0.0.0",
            "stops": [],
            "images": {"lakehouse-api": old, "console": "console@sha256:ccc"},
            "imageIds": {old: "sha256:111", "console@sha256:ccc": "sha256:222"},
            "files": {"docker-compose.yml": "sha256:old", "lakehousectl": "sha256:tool"},
        }
        new_manifest = update_proof.manifest_for(
            manifest,
            version="0.1.0-update-proof.1",
            compose_sha256="sha256:new",
            api_reference_new=new,
            api_image_id="sha256:333",
            created_at="2026-11-02T01:00:00Z",
        )
        self.assertEqual(new_manifest["version"], "0.1.0-update-proof.1")
        self.assertEqual(new_manifest["images"]["lakehouse-api"], new)
        self.assertEqual(new_manifest["images"]["console"], "console@sha256:ccc")
        self.assertEqual(new_manifest["imageIds"][new], "sha256:333")
        self.assertNotIn(old, new_manifest["imageIds"])
        self.assertEqual(new_manifest["files"]["docker-compose.yml"], "sha256:new")
        self.assertEqual(new_manifest["files"]["lakehousectl"], "sha256:tool")

    def test_a_manifest_that_names_no_api_image_is_refused(self) -> None:
        """A bundle with no API image is not a bundle the proof can
        rebuild."""
        with self.assertRaises(update_proof.ProofError):
            update_proof.api_reference({"images": {"console": "console@sha256:ccc"}})



if __name__ == "__main__":
    unittest.main()
