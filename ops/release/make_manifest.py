#!/usr/bin/env python3
"""Write the release manifest, `release.json` (REL-2 S2-T3).

Why this exists
---------------
One idea carries the whole release design: SIGN ONE FILE, VERIFY
EVERYTHING THROUGH IT (REL-2 decision D3). The manifest is that file. It
holds

  - the version;
  - every image the release runs, by digest;
  - the checksum of every file the release ships;
  - the lowest version an update may start from (`minFrom`) and the
    versions an update must pass through (`stops`).

`lakehousectl` (REL-2 S3) holds the public key and checks the manifest's
signature, then checks every image and file against the digests in it —
online or offline, with no `cosign`, no registry and no network. So the
manifest has to be complete and exact: a missing image or a wrong
checksum is not a warning, it is a release that cannot be installed.

What it does
------------
`make_manifest.py` reads the published images (service -> `ref@sha256:…`),
the rendered Compose file and the upgrade path, and writes the manifest.

The upgrade path
----------------
`minFrom` DEFAULTS to two minor versions back: the spec says "a direct
update from the previous two minor releases always works"
(`docs/core/specs/rel-2.md`), and the manifest is where that promise is
written down. A release that needs a stop names it in the ONE place that
may: `ops/release/upgrade-path.json`. That file holds an entry per
release:

    {
      "releases": {
        "0.3.0": {"minFrom": "0.1.0", "stops": ["0.2.0"]}
      }
    }

An entry may set `minFrom`, `stops`, or both; anything it does not set
keeps the default.

What it deliberately does NOT do
--------------------------------
- It does not sign anything. Cosign signs the manifest (the pipeline
  does that, with the release key); this script only writes the bytes
  that get signed.
- It does not check that a digest exists in a registry, or that the image
  is pullable. The pipeline proves that by pushing the image and reading
  the digest back.
- It does not decide the upgrade path itself. `minFrom`'s default is the
  spec's promise; a stop is a policy decision, recorded in the checked-in
  file, not made by a script.
- It does not hash anything but the Compose file yet. `lakehousectl`
  joins the manifest's `files` map in REL-2 S3, when the binary exists.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
UPGRADE_PATH = REPO_ROOT / "ops" / "release" / "upgrade-path.json"

SCHEMA_VERSION = 1

VERSION_RE = re.compile(r"^(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)$")
GIT_SHA_RE = re.compile(r"^[0-9a-f]{7,40}$")

# The image parser of the repo's lint, reused rather than copied (rule 4):
# the manifest's references must satisfy the same digest rule the Compose
# file does.
LINT_DIR = REPO_ROOT / "ops" / "lint"
if str(LINT_DIR) not in sys.path:
    sys.path.insert(0, str(LINT_DIR))

import check_image_digests  # noqa: E402  (path set just above)


class ManifestError(RuntimeError):
    """The manifest could not be written safely.

    Raised instead of writing a manifest that cannot be verified: a
    release that names an image by tag, or a checksum that does not
    match the file, is a release an operator cannot trust.
    """


def parse_version(version: str) -> tuple[int, int, int]:
    """A version as its three numbers."""
    match = VERSION_RE.match(version.strip())
    if match is None:
        raise ManifestError(
            f"the version {version!r} is not `major.minor.patch`"
        )
    return (
        int(match.group("major")),
        int(match.group("minor")),
        int(match.group("patch")),
    )


def default_min_from(version: str) -> str:
    """The lowest version that may update to this one directly.

    Two minor versions back, per the spec's promise. Below `0.2.0` there
    is no such version and the floor is `0.0.0`: any release may update
    directly.
    """
    major, minor, _patch = parse_version(version)
    if minor >= 2:
        return f"{major}.{minor - 2}.0"
    if major >= 1:
        return f"{major - 1}.0.0"
    return "0.0.0"


def sha256_of(path: Path) -> str:
    """A file's SHA-256, as lowercase hex."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_upgrade_path(path: Path) -> dict[str, dict[str, object]]:
    """The checked-in upgrade path, as release -> its requirements."""
    if not path.exists():
        return {}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise ManifestError(f"{path} could not be read as JSON: {error}") from error
    if not isinstance(data, dict):
        raise ManifestError(f"{path} must hold a JSON object")
    releases = data.get("releases", {})
    if not isinstance(releases, dict):
        raise ManifestError(f"{path}: `releases` must hold a JSON object")
    return releases


def build_manifest(
    *,
    version: str,
    git_sha: str,
    images: dict[str, str],
    compose_path: Path,
    created_at: str,
    upgrade_path: Path = UPGRADE_PATH,
) -> dict[str, object]:
    """The manifest, as the dictionary that is written out."""
    parse_version(version)

    if not GIT_SHA_RE.match(git_sha.strip()):
        raise ManifestError(
            f"the git sha {git_sha!r} is not a hexadecimal commit sha"
        )

    if not images:
        raise ManifestError("the manifest needs at least one image")

    for service, reference in sorted(images.items()):
        if not check_image_digests.check_image(reference):
            raise ManifestError(
                f"the image of service `{service}` carries no digest: "
                f"{reference}. A tag is a name and a name moves; the manifest "
                f"must name a digest."
            )

    requirements = read_upgrade_path(upgrade_path).get(version, {})
    if not isinstance(requirements, dict):
        raise ManifestError(
            f"the upgrade path's entry for {version} must hold a JSON object"
        )

    min_from = requirements.get("minFrom")
    if min_from is None:
        min_from = default_min_from(version)
    else:
        parse_version(str(min_from))

    stops = requirements.get("stops", [])
    if not isinstance(stops, list) or not all(isinstance(s, str) for s in stops):
        raise ManifestError(
            f"the upgrade path's `stops` for {version} must be a list of versions"
        )

    return {
        "schemaVersion": SCHEMA_VERSION,
        "version": version,
        "gitSha": git_sha.strip(),
        "createdAt": created_at,
        "minFrom": min_from,
        "stops": list(stops),
        "images": dict(sorted(images.items())),
        "files": {"docker-compose.yml": sha256_of(compose_path)},
    }


def now_utc() -> str:
    """The current time, as the manifest writes it."""
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def write_manifest(manifest: dict[str, object], path: Path) -> None:
    """Write the manifest, in the order the release notes read in."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(manifest, indent=2, sort_keys=False) + "\n", encoding="utf-8"
    )


def load_images(value: str) -> dict[str, str]:
    """The published images, from an inline JSON document or a file."""
    text = value.strip()
    if text.startswith("{"):
        document = text
    else:
        document = Path(value).read_text(encoding="utf-8")
    try:
        data = json.loads(document)
    except ValueError as error:
        raise ManifestError(f"the --images document is not JSON: {error}") from error
    if not isinstance(data, dict):
        raise ManifestError("the --images document must be a JSON object")
    images: dict[str, str] = {}
    for name, reference in data.items():
        if not isinstance(name, str) or not isinstance(reference, str):
            raise ManifestError(
                "the --images document must map a service name to an image reference"
            )
        images[name] = reference
    return images


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Write the release manifest, release.json.",
    )
    parser.add_argument("--version", required=True, help="the release version")
    parser.add_argument("--git-sha", required=True, help="the commit the release is built from")
    parser.add_argument(
        "--images",
        required=True,
        help="the published images, as JSON: service -> ref@sha256:… "
        "(inline JSON or a path to a .json file)",
    )
    parser.add_argument(
        "--compose",
        type=Path,
        required=True,
        help="the rendered Compose file the release ships",
    )
    parser.add_argument(
        "--out",
        type=Path,
        required=True,
        help="the manifest to write",
    )
    parser.add_argument(
        "--created-at",
        default=None,
        help="the manifest's timestamp (default: now, in UTC)",
    )
    parser.add_argument(
        "--upgrade-path",
        type=Path,
        default=UPGRADE_PATH,
        help="the checked-in upgrade path (default: ops/release/upgrade-path.json)",
    )
    args = parser.parse_args(argv)

    try:
        images = load_images(args.images)
        manifest = build_manifest(
            version=args.version,
            git_sha=args.git_sha,
            images=images,
            compose_path=args.compose,
            created_at=args.created_at or now_utc(),
            upgrade_path=args.upgrade_path,
        )
    except (ManifestError, OSError) as error:
        print(f"make_manifest failed: {error}", file=sys.stderr)
        return 1

    write_manifest(manifest, args.out)
    print(
        f"make_manifest: wrote {args.out} for version {manifest['version']} "
        f"({len(manifest['images'])} image(s), minFrom {manifest['minFrom']})."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
