#!/usr/bin/env python3
"""The images a rendered Compose file runs (REL-2 S2 review round 6).

Why this exists
---------------
The release manifest must name EVERY image the release runs: the bundle
holds one `docker save` per image the manifest names, and an image the
manifest does not name is an image an air-gapped host cannot run.

Two things need that list, and they must not compute it differently:

  - `make_manifest.py` writes it into the manifest, service by service;
  - the release pipeline pulls the third-party images and writes an
    SBOM for each.

So the rule lives here, once. The parser is the repository's own lint's
parser (`ops/lint/check_image_digests.py`), so the release and the S1
lint cannot disagree about what an image is.

What it does
------------
`compose_images.py` reads a rendered Compose file and answers two
questions:

  - which image does this service run? (`compose_images`)
  - which of those are ours, and which are theirs? (`third_party`)

OURS are the services the pipeline published — the four images this
repository builds, published by digest. THEIRS is every other service:
a third-party image, which the S1 lint already pinned by digest.

Their integrity does NOT come from a signature. We cannot sign another
project's image in its own registry, so the third-party images are not
signed and not attested: their integrity comes from the digest in the
SIGNED manifest, and the bundle holds the image that digest names.

What it deliberately does NOT do
--------------------------------
- It does not resolve `${VAR}` references. The rendered file the release
  ships names every image at its digest already.
- It does not check that a digest exists in a registry. The pipeline
  proves that by pulling the image, and a pull that fails stops the
  release.
- It does not decide what is signed. Only this repository's own images
  are, and that is the pipeline's decision, not this script's.
- It does not read the deployment TEMPLATE. The template names the
  images this repository builds by a local tag; only the RENDERED file
  names the images the release runs.

Usage
-----
  ops/release/compose_images.py --compose <file> [--published-images <file>]

It prints each reference ONCE: several services run the same image, and
the pipeline pulls that image and describes it once.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# The lint that owns the image parser (rule 4: one parser, not two).
LINT_DIR = REPO_ROOT / "ops" / "lint"
if str(LINT_DIR) not in sys.path:
    sys.path.insert(0, str(LINT_DIR))

import check_image_digests  # noqa: E402  (path set just above)


class ComposeImagesError(RuntimeError):
    """The images of a Compose file could not be read.

    Raised instead of naming an image that may not be there: a manifest
    that names the wrong image is a release an operator cannot install.
    """


def compose_images(text: str) -> dict[str, str]:
    """Every service of a Compose file, and the image it runs.

    The services that name no image are not in the map: a service
    without an image runs nothing the release has to ship.

    The parser is the lint's own (`image_lines`), so the service name
    is the one the S1 digest lint names when it refuses an unpinned
    image.
    """
    images: dict[str, str] = {}
    for _number, service, value in check_image_digests.image_lines(text):
        images[service] = value
    return images


def third_party(images: dict[str, str], published: dict[str, str]) -> dict[str, str]:
    """The images this repository did NOT publish.

    The published map is service -> reference: the services that run
    the four images the pipeline built and pushed. Every other service
    runs a third-party image.
    """
    return {
        service: reference
        for service, reference in images.items()
        if service not in published
    }


def load_published(value: str) -> dict[str, str]:
    """The published images, from an inline JSON document or a file.

    A value that opens a JSON document is the document; anything else is
    the path of a file holding it.
    """
    text = value.strip()
    if text.startswith("{") or text.startswith("["):
        document = text
    else:
        document = Path(value).read_text(encoding="utf-8")
    try:
        data = json.loads(document)
    except ValueError as error:
        raise ComposeImagesError(
            f"the published-images document is not JSON: {error}"
        ) from error
    if not isinstance(data, dict):
        raise ComposeImagesError(
            "the published-images document must be a JSON object"
        )
    published: dict[str, str] = {}
    for name, reference in data.items():
        if not isinstance(name, str) or not isinstance(reference, str):
            raise ComposeImagesError(
                "the published-images document must map a service name to an "
                "image reference"
            )
        published[name] = reference
    return published


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="The images a rendered Compose file runs.",
    )
    parser.add_argument(
        "--compose",
        type=Path,
        required=True,
        help="the rendered Compose file the release ships",
    )
    parser.add_argument(
        "--published-images",
        default=None,
        help="the published images, as JSON: service -> ref@sha256:… "
        "(inline JSON or a path to a .json file). When it is given, only "
        "the third-party images are printed — the ones the pipeline must "
        "pull and describe.",
    )
    args = parser.parse_args(argv)

    try:
        images = compose_images(args.compose.read_text(encoding="utf-8"))
        if args.published_images is None:
            chosen = images
        else:
            chosen = third_party(images, load_published(args.published_images))
    except (ComposeImagesError, OSError) as error:
        print(f"compose_images failed: {error}", file=sys.stderr)
        return 1

    for reference in dict.fromkeys(chosen.values()):
        print(reference)
    return 0


if __name__ == "__main__":
    sys.exit(main())
