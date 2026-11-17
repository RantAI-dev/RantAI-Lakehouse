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

The manifest names EVERY image the release runs, keyed by the service that
runs it:

- ours come from the published digests, which the pipeline read back from
  the registry;
- the third-party ones come from the rendered Compose file, which the S1
  lint already pinned by digest.

A third-party image without a digest is refused. The bundle holds one
`docker save` per image the manifest names (REL-2 S2 review round 6
BLOCKER 1), so an image the manifest does not name is an image the
air-gapped host cannot run.

The files the release ships
--------------------------
`files` names the checksum of every file the release carries, and the
install writes each of them into the install's own directory (REL-2 S4
decision D14):

- `docker-compose.yml` — the release's Compose file;
- `lakehousectl` — the installer;
- every RELATIVE bind-mount source of the release's Compose file. A
  relative mount (`./ops/backups/clickhouse/backups.xml`) is a file the
  deployment needs NEXT TO its Compose file: Docker creates an empty
  directory in its place when the file is missing, and the service that
  mounts it then runs against nothing;
- the two tools under `tools/` — `backup-local.sh` and
  `restore-from-backup.sh` (REL-2 S4 decision D15).

The rule that says which files those are lives in `render_compose.py`,
which owns the parsing of a Compose file, and in the `TOOL_FILES` table
above. A mount whose file does not exist, two files whose names are the
same, and a path that is not a plain relative path are REFUSALS: a
release that cannot be installed is not written.

The path rule
-------------
The install writes every shipped file at the path the manifest names,
UNDER the install's own directory. A path that is absolute, or that
holds a `..` component, would write OUTSIDE that directory. The
manifest is signed, but a signed path still escapes the directory it
is written in — so a path that is not relative, or that holds a
component that is not a normal one, is refused here (REL-2 S4 review
round 1 SHOULD-FIX 1).

The name rule
-------------
The release publishes ONE asset per file, and the asset's name is the
file's own name — its LAST path component (`files::asset_name` in
`lakehousectl`). Two files whose last component is the same are two
assets with the same name, and the online install fetches a shipped
file by that name: it could not tell them apart. So two `files`
entries whose last component is the same are refused (REL-2 S4 review
round 1 SHOULD-FIX 2).

The image IDs
-------------
`images` names every image by its REGISTRY digest — the digest the
registry answers with. An air-gapped host cannot ask a registry, and
`docker save` of a pulled image DROPS that digest: after `docker load`
the host knows only the image ID, which is the CONFIG digest
(`docker image inspect --format '{{.Id}}'`).

So the manifest carries `imageIds` as well: the unique image reference
mapped to its config digest. The installer compares the ID of every
image it loads with the one the signed manifest names, and refuses any
mismatch (REL-2 S3-T0). Without it there is nothing to compare, so an
image without an ID is refused here rather than at the install.

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
- It does not read a Docker daemon. The image IDs come from the
  pipeline, which read them with `docker image inspect`.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Iterable

REPO_ROOT = Path(__file__).resolve().parents[2]
RELEASE_DIR = Path(__file__).resolve().parent
UPGRADE_PATH = REPO_ROOT / "ops" / "release" / "upgrade-path.json"

SCHEMA_VERSION = 1

VERSION_RE = re.compile(r"^(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)$")
GIT_SHA_RE = re.compile(r"^[0-9a-f]{7,40}$")
# A Docker image ID is `sha256:` and 64 hex characters — the config
# digest `docker image inspect --format '{{.Id}}'` prints.
IMAGE_ID_RE = re.compile(r"^sha256:[0-9a-f]{64}$")

# The files the release ships that are NOT Compose mounts: the two
# scripts `lakehousectl backup` and `lakehousectl restore` run (REL-2
# S4 decision D15).
#
# The key is the path the INSTALL writes the file at — under `tools/`,
# next to the install's own files — and the value is where the file
# lives in THIS repository. The release ships the scripts under
# `tools/`; the repository keeps them where their own history put them.
TOOL_FILES = {
    "tools/backup-local.sh": "ops/backups/backup-local.sh",
    "tools/restore-from-backup.sh": "scripts/restore-from-backup.sh",
}

# The names a release publishes as assets of their own. A shipped file
# whose name is one of these would be fetched from the wrong asset by
# an ONLINE install, so the release is refused rather than published
# with an ambiguous name (REL-2 S4 decision D14).
RESOURCE_ASSET_NAMES = frozenset(
    {
        "docker-compose.yml",
        "release.json",
        "release.json.sigstore.json",
        "lakehousectl",
        "lakehousectl-linux-x86_64",
        "install.sh",
    }
)

# The image parser of the repo's lint, reused rather than copied (rule 4):
# the manifest's references must satisfy the same digest rule the Compose
# file does.
LINT_DIR = REPO_ROOT / "ops" / "lint"
if str(LINT_DIR) not in sys.path:
    sys.path.insert(0, str(LINT_DIR))

import check_image_digests  # noqa: E402  (path set just above)

# The rule "which image does this service run", and which of them are
# ours: the same file the release pipeline asks for the third-party
# images (rule 4 — one rule, one place, or the bundle and the manifest
# drift apart).
if str(RELEASE_DIR) not in sys.path:
    sys.path.insert(0, str(RELEASE_DIR))

import compose_images  # noqa: E402  (path set just above)
import render_compose  # noqa: E402  (path set just above)


class ManifestError(RuntimeError):
    """The manifest could not be written safely.

    Raised instead of writing a manifest that cannot be verified: a
    release that names an image by tag, or a checksum that does not
    match the file, is a release an operator cannot trust.
    """


@dataclass(frozen=True)
class ShippedFile:
    """One file the release ships.

    `install_path` is where the install writes it — relative to the
    install's own directory, and for a Compose mount the SAME relative
    path the Compose file mounts it at. `source_path` is where the
    file lives in this repository.
    """

    install_path: str
    source_path: Path
    sha256: str


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


def absolute(path: Path) -> Path:
    """A path that says where the file is, whatever the caller's
    directory is.

    The bundle script and the release job copy the shipped files from
    the path this prints, and neither of them runs in the directory the
    Compose file lives in by accident — so the path is absolute.
    """
    return Path(os.path.abspath(path))


def check_install_path(install_path: str) -> None:
    """Refuse a shipped file's path that is not a plain relative path.

    The install writes every shipped file at the path the manifest
    names, UNDER the install's own directory. A path that is absolute,
    or that holds a `..` component, would write OUTSIDE that directory
    — the manifest is signed, but a signed path still escapes the
    directory it is written in (REL-2 S4 review round 1 SHOULD-FIX 1).

    So the path must be relative and must hold normal components only:
    no leading `/`, no `.`, no `..` and no empty component. A Compose
    file can mount `../shared/secret.txt`, so this is a path the
    release tooling really meets.
    """
    if not install_path:
        raise ManifestError(
            "the release ships a file with an empty path. A shipped file is written under "
            "the install's own directory, so its path must name a file."
        )
    if install_path.startswith("/"):
        raise ManifestError(
            f"the release ships {install_path}, whose path is absolute. A shipped file is "
            "written under the install's own directory, so its path must be relative."
        )
    for component in install_path.split("/"):
        if component in ("", ".", ".."):
            raise ManifestError(
                f"the release ships {install_path}, whose path holds the component "
                f"{component!r}, which is not a normal one. A shipped file is written under "
                "the install's own directory, so its path must hold normal components only."
            )


def check_file_names(paths: Iterable[str]) -> None:
    """Refuse two files whose LAST component is the same.

    The release publishes one asset per file, and the asset's name is
    the file's own name — its last path component (`files::asset_name`
    in `lakehousectl`). Two files whose last component is the same are
    two assets with the same name, and the online install fetches a
    shipped file by that name: it would fetch one of them twice, and
    the checksum would catch it. A release that cannot be installed is
    refused rather than published (REL-2 S4 review round 1 SHOULD-FIX
    2).
    """
    seen: dict[str, str] = {}
    for path in paths:
        name = PurePosixPath(path).name
        other = seen.get(name)
        if other is not None:
            raise ManifestError(
                f"the release ships {path} and {other}, and both are named {name}. The release "
                "publishes one asset per file, and the asset's name is the file's own name, so "
                "the online install could not tell them apart; the release is refused."
            )
        seen[name] = path


def check_shipped_names(files: list[ShippedFile]) -> None:
    """Refuse two shipped files an ONLINE install could not tell apart.

    The online install fetches a shipped file by its NAME: the release
    publishes one asset per file, and the asset's name is the file's
    own name. Two shipped files with the same name would be two assets
    with the same name, and the install would fetch one of them twice
    — the checksum would catch it, but a release that cannot be
    installed is refused rather than published (REL-2 S4 decision D14).

    A shipped file named after a release asset of its own is refused
    for the same reason: the release publishes that asset too.
    """
    for shipped in files:
        name = PurePosixPath(shipped.install_path).name
        if name in RESOURCE_ASSET_NAMES:
            raise ManifestError(
                f"the release ships {shipped.install_path}, whose name is the name of a release "
                f"asset of its own ({name}). Two assets would carry the same name, so the "
                "release is refused."
            )
    check_file_names(shipped.install_path for shipped in files)


def shipped_files(compose_path: Path) -> list[ShippedFile]:
    """Every file the release ships: the Compose file's relative mounts
    and the two tools.

    The mounts are relative to the directory the Compose file lives in,
    and the install writes them at the SAME relative path — a mount
    that is not next to the Compose file is a mount Docker replaces
    with an empty directory (REL-2 S4 decision D14).

    A mount whose file does not exist is a REFUSAL, not a warning: a
    release that ships a Compose file mounting a file it does not carry
    installs a stack whose service runs against an empty directory.
    """
    compose_text = compose_path.read_text(encoding="utf-8")
    root = compose_path.parent
    files: list[ShippedFile] = []
    for install_path in render_compose.relative_mount_sources(compose_text):
        check_install_path(install_path)
        source = absolute(root / install_path)
        if not source.is_file():
            raise ManifestError(
                f"the Compose file mounts {install_path}, and the file does not exist: {source}. "
                "A release ships every file its Compose file mounts (REL-2 S4 decision D14)."
            )
        files.append(ShippedFile(install_path, source, sha256_of(source)))
    for install_path, repo_path in TOOL_FILES.items():
        check_install_path(install_path)
        source = absolute(REPO_ROOT / repo_path)
        if not source.is_file():
            raise ManifestError(
                f"the release ships {install_path}, and the file does not exist: {source}."
            )
        files.append(ShippedFile(install_path, source, sha256_of(source)))
    check_shipped_names(files)
    return files


def check_files_cover_the_mounts(files: dict[str, str], compose_path: Path) -> None:
    """Refuse a manifest whose `files` does not name every mounted source.

    The signed manifest is what says "these are the bytes the release
    published". A Compose file that mounts a file the manifest names no
    checksum for is a file NOTHING vouches for, and the install would
    have to trust it — so the manifest is refused instead (REL-2 S4
    decision D14).
    """
    compose_text = compose_path.read_text(encoding="utf-8")
    for install_path in render_compose.relative_mount_sources(compose_text):
        if install_path not in files:
            raise ManifestError(
                f"the Compose file mounts {install_path}, and the manifest names no checksum for "
                "it. A release ships every file its Compose file mounts (REL-2 S4 decision D14)."
            )


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


def manifest_images(
    compose_text: str, published_images: dict[str, str]
) -> dict[str, str]:
    """Every image the release runs, service by service.

    OURS come from the published digests: the pipeline built them,
    pushed them and read their digests back from the registry. THEIRS
    come from the rendered Compose file, which the S1 lint already
    pinned by digest.

    A third-party image without a digest is refused. The manifest is
    what the operator's signature covers, and a tag is a name and a name
    moves: a manifest that names a tag promises bytes it cannot name.
    """
    images: dict[str, str] = {}
    for service, reference in compose_images.compose_images(compose_text).items():
        published = published_images.get(service)
        if published is not None:
            if not check_image_digests.check_image(published):
                raise ManifestError(
                    f"the published image of service `{service}` carries no "
                    f"digest: {published}. A tag is a name and a name moves; "
                    f"the manifest must name a digest."
                )
            images[service] = published
            continue
        if not check_image_digests.check_image(reference):
            raise ManifestError(
                f"the image of service `{service}` carries no digest: "
                f"{reference}. A tag is a name and a name moves; the manifest "
                f"must name a digest."
            )
        images[service] = reference

    if not images:
        raise ManifestError("the manifest needs at least one image")
    return images


def manifest_image_ids(
    images: dict[str, str], image_ids: dict[str, str]
) -> dict[str, str]:
    """The config digest of every image the release runs.

    The manifest's `images` names each image by its REGISTRY digest; the
    `imageIds` map names the same images by their CONFIG digest — the
    image ID. An air-gapped host gets the images from `docker load`, and
    a loaded image has no registry digest: it has an ID. So the ID is
    what the installer compares (REL-2 S3-T0).

    Two rules, both refusals:

    - an image the manifest names must have an ID, or the installer has
      nothing to compare it with;
    - an ID may only be given for an image the manifest names, or the
      manifest carries bytes for an image the release does not run.
    """
    references = set(images.values())
    ids: dict[str, str] = {}
    for reference, image_id in image_ids.items():
        if reference not in references:
            raise ManifestError(
                f"the image id {image_id} is given for {reference}, which the "
                f"manifest does not name as an image of the release."
            )
        if not IMAGE_ID_RE.match(image_id):
            raise ManifestError(
                f"the image id of {reference} is {image_id!r}, which is not "
                f"`sha256:` and 64 hexadecimal characters."
            )
        ids[reference] = image_id
    missing = sorted(references - set(ids))
    if missing:
        raise ManifestError(
            "the manifest names an image with no id: "
            + ", ".join(missing)
            + ". An air-gapped install compares the loaded image's id with "
            "the manifest's, so every image needs one."
        )
    return dict(sorted(ids.items()))


def build_manifest(
    *,
    version: str,
    git_sha: str,
    published_images: dict[str, str],
    image_ids: dict[str, str],
    compose_path: Path,
    created_at: str,
    upgrade_path: Path = UPGRADE_PATH,
    lakehousectl_path: Path | None = None,
) -> dict[str, object]:
    """The manifest, as the dictionary that is written out."""
    parse_version(version)

    if not GIT_SHA_RE.match(git_sha.strip()):
        raise ManifestError(
            f"the git sha {git_sha!r} is not a hexadecimal commit sha"
        )

    compose_text = compose_path.read_text(encoding="utf-8")
    images = manifest_images(compose_text, published_images)
    ids = manifest_image_ids(images, image_ids)

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

    files = {"docker-compose.yml": sha256_of(compose_path)}
    if lakehousectl_path is not None:
        # The installer is a file the release ships, so the signed
        # manifest names its checksum too (REL-2 S3-T4): the bundle's
        # copy is checked against it.
        files["lakehousectl"] = sha256_of(lakehousectl_path)
    # Every other file the release ships: the Compose file's relative
    # mounts and the two tools (REL-2 S4 decision D14). The manifest
    # names their checksums, so the install writes bytes the signature
    # covers.
    for shipped in shipped_files(compose_path):
        files[shipped.install_path] = shipped.sha256
    # Every path the manifest names is a plain relative path: the
    # install writes them under its own directory, and a path that
    # escapes it is refused (REL-2 S4 review round 1 SHOULD-FIX 1).
    for install_path in files:
        check_install_path(install_path)
    # The release publishes one asset per file, and the asset's name is
    # the file's own name: two `files` entries whose last component is
    # the same are two assets with the same name (REL-2 S4 review round
    # 1 SHOULD-FIX 2).
    check_file_names(files)
    check_files_cover_the_mounts(files, compose_path)

    return {
        "schemaVersion": SCHEMA_VERSION,
        "version": version,
        "gitSha": git_sha.strip(),
        "createdAt": created_at,
        "minFrom": min_from,
        "stops": list(stops),
        "images": dict(sorted(images.items())),
        "imageIds": ids,
        "files": files,
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


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Write the release manifest, release.json.",
    )
    parser.add_argument("--version", help="the release version")
    parser.add_argument("--git-sha", help="the commit the release is built from")
    parser.add_argument(
        "--published-images",
        help="the published images, as JSON: service -> ref@sha256:… "
        "(inline JSON or a path to a .json file). The third-party images "
        "come from the rendered Compose file.",
    )
    parser.add_argument(
        "--image-ids",
        help="the image ids, as JSON: image reference -> sha256:<config "
        "digest> (inline JSON or a path to a .json file). The pipeline "
        "reads them with `docker image inspect --format '{{.Id}}'`. An "
        "air-gapped install compares the loaded image's id with these.",
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
    parser.add_argument(
        "--lakehousectl",
        type=Path,
        default=None,
        help="the installer binary the release ships. Its checksum joins "
        "the manifest's `files`, so the signed manifest names every file "
        "the release carries.",
    )
    parser.add_argument(
        "--print-shipped-files",
        action="store_true",
        help="print every file the release ships, one per line: the path "
        "the install writes it at, a tab, and the path it lives at in "
        "this repository. No manifest is written. `make_bundle.sh` reads "
        "this to copy the files into the bundle (REL-2 S4 decision D14).",
    )
    args = parser.parse_args(argv)

    if args.print_shipped_files:
        try:
            for shipped in shipped_files(args.compose):
                print(f"{shipped.install_path}\t{shipped.source_path}")
        except (ManifestError, OSError) as error:
            print(f"make_manifest failed: {error}", file=sys.stderr)
            return 1
        return 0

    missing = [
        name
        for name, value in (
            ("--version", args.version),
            ("--git-sha", args.git_sha),
            ("--published-images", args.published_images),
            ("--image-ids", args.image_ids),
            ("--out", args.out),
        )
        if value is None
    ]
    if missing:
        parser.error(f"the following arguments are required: {', '.join(missing)}")

    try:
        published_images = compose_images.load_published(args.published_images)
        image_ids = compose_images.load_string_map(args.image_ids, "image-ids")
        manifest = build_manifest(
            version=args.version,
            git_sha=args.git_sha,
            published_images=published_images,
            image_ids=image_ids,
            compose_path=args.compose,
            created_at=args.created_at or now_utc(),
            upgrade_path=args.upgrade_path,
            lakehousectl_path=args.lakehousectl,
        )
    except (ManifestError, compose_images.ComposeImagesError, OSError) as error:
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
