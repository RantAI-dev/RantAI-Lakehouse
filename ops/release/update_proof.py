#!/usr/bin/env python3
"""The bundles of the update proof (REL-2 S4 decision D21).

What this owns
--------------
The update proof builds TWO bundles out of the dry run's bundle A:

- **bundle B**: the API image REBUILT with its version set to
  `V-update-proof.1`, every other image reused from A;
- **bundle C**: the same, with the version `V-update-proof.2` and an
  API image that never becomes ready (it sleeps).

This module owns the RULES of that, and the workflow owns the Docker:

- the version of a proof bundle;
- which image is rebuilt, and which are reused;
- what the new manifest says — the version, the API's reference, the
  API's image id, and the Compose file's checksum;
- what the new Compose file says — the API's reference.

The rules are here, and not in the workflow's shell, so they can be
tested with no Docker and no network: the job's own logic is testable,
and the job itself is proved by the PR's dry run (REL-2 S4 T7).

Why the proof builds bundles at all
-----------------------------------
0.2.0 is the first release (REL-2 decision D4), so there is no earlier
release to install and update FROM. The proof builds the "previous
release" itself: bundle A is the release that is installed, bundle B is
the release it is updated to, and bundle C is a release that breaks —
so the rollback is proved on a real stack, with real data, and not on a
mock (D21).
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import shutil
import sys
from pathlib import Path
from typing import Any

# The service that runs the API image.
#
# The release's Compose file names the service `lakehouse-api`, and the
# manifest's `images` is keyed by service — so the API's image is found
# by the service, and not by the image's name (whose rule belongs to
# `ops/release/image_name.sh`).
API_SERVICE = "lakehouse-api"

# The suffix a proof bundle's version carries.
PROOF_SUFFIX = "update-proof"

# The Compose file's name in the manifest's `files`.
COMPOSE_FILE = "docker-compose.yml"

# The installer's name in the manifest's `files`.
INSTALLER_FILE = "lakehousectl"


class ProofError(Exception):
    """The proof cannot be built from what it was given."""


def proof_version(version: str, step: int) -> str:
    """The version of a proof bundle.

    `0.1.0` and step 1 give `0.1.0-update-proof.1`: the version of the
    release the proof starts from, and the step of the proof.
    """
    if step < 1:
        raise ProofError(f"the step of a proof bundle is {step}, and it is at least 1")
    return f"{version}-{PROOF_SUFFIX}.{step}"


def api_reference(manifest: dict[str, Any]) -> str:
    """The reference of the API image the manifest names."""
    images = manifest.get("images")
    if not isinstance(images, dict):
        raise ProofError("the manifest names no images")
    reference = images.get(API_SERVICE)
    if not isinstance(reference, str) or not reference:
        raise ProofError(f"the manifest names no image for the service {API_SERVICE}")
    return reference


def reused_images(manifest: dict[str, Any]) -> dict[str, str]:
    """Every image of the manifest that is NOT the API's.

    Those are the images bundle B and bundle C REUSE: every image but
    the one that is rebuilt (D21).
    """
    images = manifest.get("images")
    if not isinstance(images, dict):
        raise ProofError("the manifest names no images")
    return {
        service: reference
        for service, reference in images.items()
        if service != API_SERVICE
    }


def compose_with(compose: str, old: str, new: str) -> str:
    """The Compose file with the API's reference replaced.

    The reference is pinned by digest, and the rebuilt image has
    another digest — so the file has to name the new one, and the
    manifest's checksum of the file has to be computed again.

    The old reference has to be there EXACTLY ONCE: a file that names
    it twice would run two images, and a file that does not name it at
    all would run the old one.
    """
    count = compose.count(old)
    if count == 0:
        raise ProofError(f"the Compose file does not name {old}")
    if count > 1:
        raise ProofError(f"the Compose file names {old} {count} times")
    return compose.replace(old, new)


def manifest_for(
    manifest: dict[str, Any],
    *,
    version: str,
    compose_sha256: str,
    api_reference_new: str,
    api_image_id: str,
    created_at: str,
) -> dict[str, Any]:
    """The manifest of a proof bundle.

    Everything is the manifest of bundle A, except:

    - the version, which is the proof bundle's own;
    - the API's image reference, which is the REBUILT image;
    - the API's image id, which is the rebuilt image's;
    - the Compose file's checksum, which is the new file's.

    Every other image, and every other file, is the one bundle A
    carries: the proof reuses them (D21).
    """
    images = manifest.get("images")
    if not isinstance(images, dict):
        raise ProofError("the manifest names no images")
    files = manifest.get("files")
    if not isinstance(files, dict):
        raise ProofError("the manifest names no files")

    old_reference = api_reference(manifest)

    new_images = copy.deepcopy(images)
    new_images[API_SERVICE] = api_reference_new

    # The image ids are keyed by the REFERENCE, so the rebuilt image's
    # entry replaces the old one's.
    ids = manifest.get("imageIds")
    if not isinstance(ids, dict):
        raise ProofError("the manifest names no image ids")
    new_ids = copy.deepcopy(ids)
    new_ids.pop(old_reference, None)
    new_ids[api_reference_new] = api_image_id

    new_files = copy.deepcopy(files)
    new_files[COMPOSE_FILE] = compose_sha256

    return {
        "schemaVersion": manifest.get("schemaVersion", 1),
        "version": version,
        "gitSha": manifest.get("gitSha", "0" * 40),
        "createdAt": created_at,
        "minFrom": manifest.get("minFrom", "0.0.0"),
        "stops": manifest.get("stops", []),
        "images": new_images,
        "imageIds": new_ids,
        "files": new_files,
    }


def build_bundle(
    source: Path,
    target: Path,
    *,
    version: str,
    api_reference_new: str,
    api_image_id: str,
    created_at: str,
) -> dict[str, Any]:
    """Build a proof bundle out of another one.

    The bundle is the source bundle with:

    - the API image REBUILT: its reference and its id are the new
      ones, and the caller replaces its archive in the target;
    - the Compose file that names the new image;
    - the manifest that says so, and that is signed by the job's key.

    Every OTHER image is REUSED: the archives are hard-linked, so a
    bundle of ten gigabytes costs nothing (D21).
    """
    manifest = json.loads((source / "release.json").read_text(encoding="utf-8"))
    compose = (source / "docker-compose.yml").read_text(encoding="utf-8")
    old_reference = api_reference(manifest)
    new_compose = compose_with(compose, old_reference, api_reference_new)

    target.mkdir(parents=True, exist_ok=True)

    # The images: REUSED, and hard-linked.
    images = source / "images"
    if images.is_dir():
        target_images = target / "images"
        target_images.mkdir(parents=True, exist_ok=True)
        for archive in sorted(images.iterdir()):
            link = target_images / archive.name
            if link.exists() or link.is_symlink():
                link.unlink()
            os.link(archive, link)

    # Everything else the bundle carries: the SBOMs, the tools and the
    # files the release ships.
    for item in source.iterdir():
        if item.name in ("images", "release.json", "release.json.sigstore.json"):
            continue
        if item.name == "docker-compose.yml":
            continue
        destination = target / item.name
        if item.is_dir():
            shutil.copytree(item, destination, dirs_exist_ok=True)
        else:
            shutil.copy2(item, destination)

    (target / "docker-compose.yml").write_text(new_compose, encoding="utf-8")
    new_manifest = manifest_for(
        manifest,
        version=version,
        compose_sha256=sha256_of(target / "docker-compose.yml"),
        api_reference_new=api_reference_new,
        api_image_id=api_image_id,
        created_at=created_at,
    )
    (target / "release.json").write_text(
        json.dumps(new_manifest, indent=2) + "\n", encoding="utf-8"
    )
    return new_manifest


def sha256_of(path: Path) -> str:
    """The SHA-256 of a file, as lowercase hex."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main(argv: list[str] | None = None) -> int:
    """The command line of the proof's helper."""
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    version = commands.add_parser("version", help="the version of a proof bundle")
    version.add_argument("release", help="the version of the release the proof starts from")
    version.add_argument("step", type=int, help="the step of the proof")

    bundle = commands.add_parser("bundle", help="build a proof bundle out of another one")
    bundle.add_argument("--from", dest="source", type=Path, required=True)
    bundle.add_argument("--to", dest="target", type=Path, required=True)
    bundle.add_argument("--version", required=True)
    bundle.add_argument("--api-reference", required=True)
    bundle.add_argument("--api-image-id", required=True)
    bundle.add_argument("--created-at", required=True)

    args = parser.parse_args(argv)
    try:
        if args.command == "version":
            print(proof_version(args.release, args.step))
            return 0
        manifest = build_bundle(
            args.source,
            args.target,
            version=args.version,
            api_reference_new=args.api_reference,
            api_image_id=args.api_image_id,
            created_at=args.created_at,
        )
    except ProofError as error:
        print(f"update_proof: {error}", file=sys.stderr)
        return 1
    print(manifest["version"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
