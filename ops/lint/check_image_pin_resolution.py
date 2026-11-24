#!/usr/bin/env python3
"""REL-17: every pinned third-party image must still resolve AND name the
version its tag claims.

# Why this exists

`ops/lint/check_image_digests.py` proves that every third-party image in the
deployment template carries a `@sha256:…` pin. That is a statement about the
FILE, not about the registry: it cannot tell that the digest was withdrawn
upstream, or that the tag written next to it describes a different release.
Both happened to `quay.io/debezium/connect:3.8@sha256:d67d…` (#264,
2026-11-04): the digest disappeared from quay.io and the `3.8` tag had moved
to `3.8.0.Alpha1`. The image was re-pinned to `3.7.0.Final`, but nothing in
CI would have caught the next occurrence.

This script closes that gap. For each pinned image it

  1. asks the registry for the manifest BY DIGEST and checks that the bytes
     the registry returns hash to that digest (a withdrawn digest is a 404);
  2. follows a multi-arch manifest list / OCI index to its `linux/amd64`
     (or first) manifest, reads that manifest's config blob, and
  3. compares the version the config declares with the pin's tag.

# Version claims: authoritative vs advisory

Image configs lie about "version" in a very specific way: base images put
THEIR version in generic labels. `org.opencontainers.image.version` is
`22.04` on Ubuntu-based images, `44` on Fedora-based ones, `version=10.2` on
UBI-micro-based ones, and `io.buildah.version` names the build tool. If every
version-shaped field were treated as a claim about the pinned product, most
of this repository's images would fail.

So a claim is only *authoritative* when its key names the product: an env
`*_VERSION` whose prefix is a product token from the image's repository
(`DEBEZIUM_VERSION` on `debezium/connect`, `PG_VERSION` on `postgres`,
`PYTHON_VERSION` on `python`), or a label whose key contains such a token
(`com.clickhouse.build.version` on `clickhouse-server`). Generic keys
(`version`, `org.opencontainers.image.version`, `io.buildah.version`,
dependency versions like `KAFKA_VERSION` on a Debezium image) are
*advisory*.

The rule is deliberately asymmetric about what passes:

  - an authoritative claim that disagrees with the tag is a FAILURE — that
    is the product saying it is a different release than the pin claims;
  - when only advisory claims exist, at least one agreeing with the tag is
    enough to pass, and none agreeing is reported as "no product version
    declared" rather than a failure. A false positive here would block a
    deployment for a base image's version label.

`version_matches` compares numeric component prefixes (so `26.8` agrees
with `26.8.36.3`, and `3.12-slim` with `3.12.16`) and refuses a prerelease
mismatch (so `3.8` does NOT agree with `3.8.0.Alpha1`).

# Where this runs, and where it does not

Live registry access is a scheduled workflow (`.github/workflows/image-pins.yml`,
daily plus manual dispatch), NOT a pull-request check: Docker Hub returns
429 and registries have 5xx incidents, and a required PR check must not go
red because of someone else's outage. The unit tests of this module DO run
in `Repo lints` (they are discovered by `python3 -m unittest discover -s
ops/lint`) because they use no network.

The image list is the repository's ONE list: `image_lines`,
`is_local_build` and `check_image` from `ops/lint/check_image_digests.py`.
There is no second discovery regex or file list here (AGENTS.md rule 4).

# What this deliberately does NOT cover

- Only `docker-compose.yml`, the deployment template — the same scope as
  the digest lint. The CI-only gate override stacks carry their own images
  and are not checked here.
- No layers are pulled, no signatures or attestations are verified. This
  checks that the pin resolves and that the config agrees with the tag.
- A config with no version claim passes. "Absent" is reported, not treated
  as a match.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path

# The lint that owns the image parser (rule 4: one parser, not two).
LINT_DIR = Path(__file__).resolve().parent
if str(LINT_DIR) not in sys.path:
    sys.path.insert(0, str(LINT_DIR))

import check_image_digests  # noqa: E402  (path set just above)

DEFAULT_COMPOSE = check_image_digests.COMPOSE_PATH

# Ask for both Docker and OCI media types: an index is returned when the
# registry stores one, a plain manifest when it does not.
ACCEPT = ", ".join(
    (
        "application/vnd.oci.image.index.v1+json",
        "application/vnd.docker.distribution.manifest.list.v2+json",
        "application/vnd.oci.image.manifest.v1+json",
        "application/vnd.docker.distribution.manifest.v2+json",
    )
)
CONFIG_ACCEPT = "application/vnd.docker.container.image.v1+json"

# Transient registry responses worth retrying. A 404 (withdrawn digest) and
# a 401 that survives authentication are NOT transient: they are the finding.
RETRYABLE = frozenset({429, 500, 502, 503, 504})

# Prerelease markers in a version string. `Final` and `GA` are releases.
PRERELEASE_RE = re.compile(
    r"(?<![a-z])(alpha|beta|rc|pre|preview|snapshot|dev|milestone|cr)(?![a-z])"
)

# Env aliases that name the product but not its repository token.
ENV_ALIASES = frozenset({"PG", "VERSION"})


class PinError(RuntimeError):
    """A pin could not be checked, or the registry contradicted it."""


@dataclass(frozen=True)
class ImageRef:
    """A parsed `name:tag@sha256:digest` reference."""

    text: str
    registry: str
    repository: str
    tag: str
    digest: str


@dataclass(frozen=True)
class Response:
    """The part of an HTTP response the checker uses."""

    status: int
    headers: dict[str, str]
    body: bytes


@dataclass(frozen=True)
class VersionClaim:
    """A version-shaped field in the image config."""

    source: str
    value: str
    authoritative: bool


@dataclass(frozen=True)
class Check:
    """The result for one pin."""

    ref: str
    ok: bool
    detail: str


def pinned_images(text: str) -> list[str]:
    """The unique pinned third-party references in a Compose file.

    Uses the digest lint's own extractor and exemptions, so a change to what
    counts as an image cannot make the two checks disagree.
    """
    found: dict[str, None] = {}
    for _number, _service, value in check_image_digests.image_lines(text):
        if check_image_digests.is_local_build(value):
            continue
        if not check_image_digests.check_image(value):
            continue
        found[value] = None
    return list(found)


def parse_reference(value: str) -> ImageRef:
    """Split a pinned reference into registry, repository, tag and digest.

    A name with no registry host is Docker Hub: `postgres` is
    `library/postgres`, and `clickhouse/clickhouse-server` keeps its namespace.
    A first component containing `.` or `:` (or `localhost`) is a registry
    host, so `quay.io/debezium/connect` keeps `debezium/connect` as its repo.
    """
    if "@" not in value:
        raise PinError(f"no digest in image reference {value!r}")
    name, digest = value.rsplit("@", 1)
    if check_image_digests.DIGEST_RE.search(value) is None:
        raise PinError(f"not a full sha256 digest: {value!r}")

    # Strip the tag BEFORE looking for a registry host: the colon in
    # `postgres:16.15` is a tag, while the one in `localhost:5000` is a port.
    last = name.rsplit("/", 1)[-1]
    if ":" in last:
        tag = last.rsplit(":", 1)[1]
        name = name.rsplit(":", 1)[0]
    else:
        tag = "latest"

    parts = name.split("/")
    first = parts[0]
    is_host = first == "localhost" or "." in first or (
        ":" in first and first.rsplit(":", 1)[1].isdigit()
    )
    if is_host:
        registry = first
        repo_parts = parts[1:]
    else:
        registry = "registry-1.docker.io"
        repo_parts = parts
        if len(repo_parts) == 1:
            repo_parts = ["library", *repo_parts]
    if not repo_parts:
        raise PinError(f"no repository in image reference {value!r}")

    return ImageRef(value, registry, "/".join(repo_parts), tag, digest)


def product_tokens(repository: str) -> frozenset[str]:
    """The uppercase product names derived from a repository path.

    `debezium/connect` gives `DEBEZIUM` and `CONNECT`; `rustfs/rustfs`
    gives `RUSTFS`. Used to tell the product's own `*_VERSION` env var from
    the many dependency versions a base image may declare.
    """
    tokens: set[str] = set()
    for part in repository.split("/"):
        cleaned = re.sub(r"[^A-Za-z0-9]+", "_", part).strip("_").upper()
        if cleaned:
            tokens.add(cleaned)
    return frozenset(tokens)


def is_version_label(key: str) -> bool:
    """Whether a label key names a version (not whether it is authoritative)."""
    lowered = key.lower()
    return lowered == "version" or lowered.endswith(".version")


def label_is_authoritative(key: str, tokens: frozenset[str]) -> bool:
    """A label is authoritative when its key contains a product token."""
    lowered = key.lower()
    return any(token.lower() in lowered for token in tokens)


def env_is_authoritative(key: str, tokens: frozenset[str]) -> bool:
    """An env var is authoritative when its name names the product."""
    if key == "VERSION":
        return True
    if not key.endswith("_VERSION"):
        return False
    prefix = key[: -len("_VERSION")]
    return prefix in ENV_ALIASES or prefix in tokens


def version_claims(config: dict, ref: ImageRef) -> list[VersionClaim]:
    """The version-shaped fields of an image config, labelled by trust.

    A missing `config` object, `Labels: null` or a non-string env entry is
    not an error: old image configs legitimately have those shapes.
    """
    body = config.get("config")
    if not isinstance(body, dict):
        return []
    tokens = product_tokens(ref.repository)
    claims: list[VersionClaim] = []

    labels = body.get("Labels")
    if isinstance(labels, dict):
        for key, value in labels.items():
            if isinstance(key, str) and is_version_label(key) and isinstance(value, str):
                if value.strip():
                    claims.append(
                        VersionClaim(
                            f"label {key}", value.strip(), label_is_authoritative(key, tokens)
                        )
                    )

    env = body.get("Env")
    if isinstance(env, list):
        for item in env:
            if not isinstance(item, str) or "=" not in item:
                continue
            key, value = item.split("=", 1)
            key = key.strip()
            value = value.strip()
            if (key == "VERSION" or key.endswith("_VERSION")) and value:
                claims.append(
                    VersionClaim(f"env {key}", value, env_is_authoritative(key, tokens))
                )
    return claims


def _normalize(version: str) -> str:
    lowered = version.strip().lower()
    if lowered.startswith("v") and len(lowered) > 1 and lowered[1].isdigit():
        lowered = lowered[1:]
    return lowered.split("+", 1)[0]


def prerelease_kinds(version: str) -> frozenset[str]:
    """The prerelease markers in a version (`3.8.0.Alpha1` -> `alpha`)."""
    return frozenset(match.group(1) for match in PRERELEASE_RE.finditer(_normalize(version)))


def version_matches(tag: str, declared: str) -> bool:
    """Whether a declared version names the same release line as a tag.

    Numeric components may be extended (`26.8` vs `26.8.36.3`) but may not
    differ (`3.8` vs `3.9`). A variant suffix (`-slim`, `-alpine`) is not a
    version component, so `1.99.1-slim` agrees with `1.99.1`. Prerelease
    markers must agree: this is what catches `3.8` vs `3.8.0.Alpha1`.
    """
    tagged = _normalize(tag)
    declared_normalized = _normalize(declared)
    if prerelease_kinds(tagged) != prerelease_kinds(declared_normalized):
        return False
    tag_numbers = re.findall(r"\d+", tagged)
    declared_numbers = re.findall(r"\d+", declared_normalized)
    if tag_numbers and declared_numbers:
        shortest = min(len(tag_numbers), len(declared_numbers))
        return tag_numbers[:shortest] == declared_numbers[:shortest]
    return tagged == declared_normalized


def decide(ref: ImageRef, claims: list[VersionClaim]) -> Check:
    """Turn the config's version claims into a pass/fail for one pin."""
    authoritative = [claim for claim in claims if claim.authoritative]
    if authoritative:
        bad = [claim for claim in authoritative if not version_matches(ref.tag, claim.value)]
        if bad:
            described = ", ".join(f"{c.source}={c.value}" for c in bad)
            return Check(
                ref.text,
                False,
                f"tag {ref.tag} disagrees with product version(s): {described}",
            )
        agreeing = ", ".join(f"{c.source}={c.value}" for c in authoritative)
        return Check(ref.text, True, f"tag {ref.tag} confirmed by {agreeing}")

    if not claims:
        return Check(ref.text, True, "image config declares no version")
    agreeing = [claim for claim in claims if version_matches(ref.tag, claim.value)]
    if agreeing:
        described = ", ".join(f"{c.source}={c.value}" for c in agreeing)
        return Check(ref.text, True, f"tag {ref.tag} agrees with {described}")
    described = ", ".join(f"{c.source}={c.value}" for c in claims)
    return Check(
        ref.text,
        True,
        f"no product version declared; advisory field(s) ignored: {described}",
    )


class UrllibHttpClient:
    """The network boundary: a plain GET, with no repository knowledge."""

    def __init__(self, timeout: float = 30.0) -> None:
        self.timeout = timeout

    def get(self, url: str, headers: dict[str, str]) -> Response:
        request = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as answer:
                return Response(
                    answer.status,
                    {key.lower(): value for key, value in answer.headers.items()},
                    answer.read(),
                )
        except urllib.error.HTTPError as error:
            return Response(
                error.code,
                {key.lower(): value for key, value in error.headers.items()},
                error.read(),
            )


class RegistryClient:
    """Registry v2 reads with bearer-token auth and transient-error retries.

    `http` and `sleeper` are injectable so the unit tests can drive every
    branch without a network or a real delay.
    """

    def __init__(
        self,
        http: UrllibHttpClient | None = None,
        *,
        sleeper=time.sleep,
        attempts: int = 3,
        backoff: float = 1.0,
        verify_digest: bool = True,
    ) -> None:
        if attempts < 1:
            raise ValueError("attempts must be at least 1")
        self.http = http if http is not None else UrllibHttpClient()
        self.sleeper = sleeper
        self.attempts = attempts
        self.backoff = backoff
        self.verify_digest = verify_digest
        self._tokens: dict[tuple[str, str], str] = {}

    def _endpoint(self, ref: ImageRef, kind: str, value: str) -> str:
        host = "http" if ref.registry == "localhost" or ref.registry.startswith("localhost:") else "https"
        return f"{host}://{ref.registry}/v2/{ref.repository}/{kind}/{value}"

    def _bearer_token(self, headers: dict[str, str]) -> str | None:
        """Follow the `WWW-Authenticate` challenge to a pull token.

        Docker Hub, quay.io, MCR and ghcr.io all use this standard flow; the
        realm and scope come from the challenge rather than being hard-coded.
        """
        challenge = headers.get("www-authenticate", "")
        if not challenge.lower().startswith("bearer"):
            return None
        parameters = dict(re.findall(r'(\w+)="([^"]*)"', challenge))
        realm = parameters.get("realm")
        if not realm:
            return None
        cache_key = (realm, parameters.get("scope", ""))
        if cache_key in self._tokens:
            return self._tokens[cache_key]

        parsed = urllib.parse.urlsplit(realm)
        query = dict(urllib.parse.parse_qsl(parsed.query))
        query.update({key: value for key, value in parameters.items() if key != "realm"})
        url = urllib.parse.urlunsplit(
            (parsed.scheme, parsed.netloc, parsed.path, urllib.parse.urlencode(query), parsed.fragment)
        )
        answer = self._get_with_retries(url, {"Accept": "application/json"})
        if answer.status != 200:
            return None
        try:
            payload = json.loads(answer.body)
        except (ValueError, UnicodeDecodeError):
            return None
        token = None
        if isinstance(payload, dict):
            token = payload.get("token") or payload.get("access_token")
        if not isinstance(token, str) or not token:
            return None
        self._tokens[cache_key] = token
        return token

    def _get_with_retries(self, url: str, headers: dict[str, str]) -> Response:
        """GET, retrying only transient responses (both manifests and tokens)."""
        retries = 0
        while True:
            answer = self.http.get(url, headers)
            if answer.status in RETRYABLE and retries < self.attempts - 1:
                self.sleeper(self.backoff * (2**retries))
                retries += 1
                continue
            return answer

    def get(self, url: str, headers: dict[str, str]) -> Response:
        """GET with one bearer-token challenge and bounded transient retries."""
        request_headers = dict(headers)
        answer = self._get_with_retries(url, request_headers)
        if answer.status == 401:
            token = self._bearer_token(answer.headers)
            if token:
                request_headers["Authorization"] = f"Bearer {token}"
                answer = self._get_with_retries(url, request_headers)
        return answer

    def manifest(self, ref: ImageRef, digest: str) -> dict:
        """Fetch a manifest BY DIGEST and prove the bytes hash to that digest."""
        answer = self.get(
            self._endpoint(ref, "manifests", digest),
            {"Accept": ACCEPT, "User-Agent": "lakehouse-image-pins/1 REL-17"},
        )
        if answer.status != 200:
            raise PinError(f"registry returned HTTP {answer.status} for digest {digest}")
        if self.verify_digest:
            actual = hashlib.sha256(answer.body).hexdigest()
            if actual != digest.split(":", 1)[1]:
                raise PinError(
                    f"manifest bytes hash to sha256:{actual}, not the pinned {digest}"
                )
        try:
            payload = json.loads(answer.body)
        except (ValueError, UnicodeDecodeError) as error:
            raise PinError(f"manifest is not JSON: {error}") from error
        if not isinstance(payload, dict):
            raise PinError("manifest is not a JSON object")
        return payload

    def blob(self, ref: ImageRef, digest: str) -> dict:
        """Fetch and decode a config blob."""
        answer = self.get(
            self._endpoint(ref, "blobs", digest),
            {"Accept": CONFIG_ACCEPT, "User-Agent": "lakehouse-image-pins/1 REL-17"},
        )
        if answer.status != 200:
            raise PinError(f"registry returned HTTP {answer.status} for config blob {digest}")
        try:
            payload = json.loads(answer.body)
        except (ValueError, UnicodeDecodeError) as error:
            raise PinError(f"config blob is not JSON: {error}") from error
        if not isinstance(payload, dict):
            raise PinError("config blob is not a JSON object")
        return payload


def select_manifest(manifest: dict) -> tuple[dict, bool]:
    """Pick the image manifest from a manifest list / OCI index.

    Returns the chosen object and whether the input was an index. `linux/amd64`
    is preferred; the first Linux manifest, then the first manifest at all,
    is the fallback. An index carries no config, so its child must be fetched.
    """
    entries = manifest.get("manifests")
    if not isinstance(entries, list) or not entries:
        return manifest, False
    usable = [entry for entry in entries if isinstance(entry, dict)]
    if not usable:
        raise PinError("manifest list has no usable entries")
    amd64 = [
        entry
        for entry in usable
        if entry.get("platform", {}).get("os") == "linux"
        and entry.get("platform", {}).get("architecture") == "amd64"
    ]
    if amd64:
        return amd64[0], True
    linux = [entry for entry in usable if entry.get("platform", {}).get("os") == "linux"]
    return (linux or usable)[0], True


def image_config(manifest: dict, ref: ImageRef, client: RegistryClient) -> dict:
    """The config JSON of the platform image behind a manifest or index."""
    selected, is_index = select_manifest(manifest)
    if is_index:
        digest = selected.get("digest")
        if not isinstance(digest, str):
            raise PinError("selected manifest has no digest")
        manifest = client.manifest(ref, digest)
        selected, _ = select_manifest(manifest)
    config = selected.get("config")
    if not isinstance(config, dict) or not isinstance(config.get("digest"), str):
        # A schema-1 manifest has no config blob: no version can be declared.
        return {}
    return client.blob(ref, config["digest"])


def verify_pin(ref: ImageRef, client: RegistryClient) -> Check:
    """Resolve one pin, read its config, and compare declared versions to the tag."""
    try:
        manifest = client.manifest(ref, ref.digest)
        config = image_config(manifest, ref, client)
        return decide(ref, version_claims(config, ref))
    except (PinError, OSError, urllib.error.URLError, ValueError, KeyError, TypeError) as error:
        return Check(ref.text, False, f"could not verify pin: {error}")


def write_summary(checks: list[Check], path: str | None) -> None:
    """Append a Markdown table to GitHub's job summary when one is provided."""
    if not path:
        return
    lines = [
        "## Pinned image resolution (REL-17)",
        "",
        "| Image pin | Result | Detail |",
        "| --- | --- | --- |",
    ]
    for check in checks:
        detail = check.detail.replace("|", "\\|")
        lines.append(f"| `{check.ref}` | {'PASS' if check.ok else 'FAIL'} | {detail} |")
    with open(path, "a", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Verify that pinned third-party image digests resolve and name their tag's version."
    )
    parser.add_argument(
        "--compose",
        type=Path,
        default=DEFAULT_COMPOSE,
        help=f"compose file to read pins from (default: {DEFAULT_COMPOSE})",
    )
    parser.add_argument(
        "--attempts",
        type=int,
        default=int(os.environ.get("IMAGE_PIN_ATTEMPTS", "3")),
        help="registry GET attempts for transient 429/5xx responses (default: 3)",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=float(os.environ.get("IMAGE_PIN_TIMEOUT", "30")),
        help="per-request timeout in seconds (default: 30)",
    )
    args = parser.parse_args(argv)

    try:
        text = args.compose.read_text(encoding="utf-8")
    except OSError as error:
        print(f"image pin guard: cannot read {args.compose}: {error}", file=sys.stderr)
        return 1

    references = pinned_images(text)
    client = RegistryClient(
        UrllibHttpClient(timeout=args.timeout),
        attempts=args.attempts,
    )
    checks: list[Check] = []
    for value in references:
        try:
            ref = parse_reference(value)
        except PinError as error:
            checks.append(Check(value, False, str(error)))
            continue
        check = verify_pin(ref, client)
        checks.append(check)
        print(f"{'PASS' if check.ok else 'FAIL'}  {value}")
        print(f"      {check.detail}")

    write_summary(checks, os.environ.get("GITHUB_STEP_SUMMARY"))
    failures = sum(not check.ok for check in checks)
    print(
        f"image pin guard: {len(checks) - failures}/{len(checks)} pinned image(s) "
        f"resolve and name their tag's version."
    )
    if failures:
        print(
            f"image pin guard: {failures} pin(s) failed. A withdrawn digest or a "
            "tag/config mismatch must be re-pinned in docker-compose.yml."
        )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
