#!/usr/bin/env python3
"""Unit tests for `ops/lint/check_image_pin_resolution.py` (REL-17).

No network, no Docker daemon: every registry response is a canned `Response`
served by `FakeHttpClient`, including the bearer-token challenge, 429/5xx
retries, multi-arch indexes and config blobs. Run in CI's "Repo lints" by
`python3 -m unittest discover -s ops/lint -p 'test_*.py'`.

The `RecordedPinsTest` drives the checker with metadata inspected from the
18 images actually pinned on main. It asserts the set of recorded refs is the
set the shared extractor finds, so a new pin cannot silently go untested.
"""

from __future__ import annotations

import hashlib
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_image_digests  # noqa: E402
import check_image_pin_resolution as pins  # noqa: E402


def digest_of(payload: bytes) -> str:
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def ref_for(tag: str, digest_hex: str) -> pins.ImageRef:
    return pins.parse_reference(f"registry.example/product:{tag}@sha256:{digest_hex}")


class FakeHttpClient:
    """A URL -> responses map. A repeated URL is drained in order."""

    def __init__(self, responses: dict[str, list[pins.Response]]) -> None:
        self.responses = {url: list(values) for url, values in responses.items()}
        self.calls: list[tuple[str, dict[str, str]]] = []

    def get(self, url: str, headers: dict[str, str]) -> pins.Response:
        self.calls.append((url, dict(headers)))
        key = url if url in self.responses else url.split("?", 1)[0]
        values = self.responses.get(key)
        if not values:
            return pins.Response(404, {}, b'{"errors":[{"code":"MANIFEST_UNKNOWN"}]}')
        return values.pop(0) if len(values) > 1 else values[0]

    def urls(self) -> list[str]:
        return [url for url, _ in self.calls]


def served_client(
    ref: pins.ImageRef,
    labels: dict[str, str] | None,
    env: list[str] | None,
    *,
    index: bool = True,
    status: int = 200,
    verify_digest: bool = False,
    sleeper=None,
    attempts: int = 3,
    other_platforms: bool = False,
) -> tuple[pins.RegistryClient, FakeHttpClient]:
    """Build a RegistryClient whose registry serves one fully described image.

    `verify_digest=False` is used because a fixture's manifest bytes are not
    the real upstream bytes. Digest verification has its own test below.
    """
    config_bytes = json.dumps({"config": {"Labels": labels, "Env": env}}).encode()
    config_digest = digest_of(config_bytes)
    manifest = {
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"digest": config_digest, "size": len(config_bytes)},
        "layers": [],
    }
    manifest_bytes = json.dumps(manifest).encode()
    manifest_digest = digest_of(manifest_bytes)

    top = manifest
    if index:
        entries = [
            {
                "digest": manifest_digest,
                "platform": {"os": "linux", "architecture": "amd64"},
            }
        ]
        if other_platforms:
            entries.insert(
                0,
                {"digest": "sha256:" + "a" * 64, "platform": {"os": "linux", "architecture": "arm64"}},
            )
        top = {"schemaVersion": 2, "manifests": entries}

    client_for = pins.RegistryClient
    # The helper is a bound constructor; build a temporary client only to
    # reuse its URL builder without issuing a request.
    url_builder = client_for(FakeHttpClient({}), verify_digest=verify_digest)
    top_url = url_builder._endpoint(ref, "manifests", ref.digest)
    child_url = url_builder._endpoint(ref, "manifests", manifest_digest)
    blob_url = url_builder._endpoint(ref, "blobs", config_digest)
    http = FakeHttpClient(
        {
            top_url: [pins.Response(status, {}, json.dumps(top).encode())],
            child_url: [pins.Response(200, {}, manifest_bytes)],
            blob_url: [pins.Response(200, {}, config_bytes)],
        }
    )
    client = pins.RegistryClient(
        http, verify_digest=verify_digest, attempts=attempts,
        sleeper=sleeper if sleeper is not None else (lambda _: None),
    )
    return client, http


class ReferenceParsingTest(unittest.TestCase):
    """How a pinned reference becomes a registry API path."""

    def test_docker_hub_official_image_gets_library_namespace(self) -> None:
        ref = pins.parse_reference(
            "postgres:16.15@sha256:" + "1" * 64
        )
        self.assertEqual(ref.registry, "registry-1.docker.io")
        self.assertEqual(ref.repository, "library/postgres")
        self.assertEqual(ref.tag, "16.15")

    def test_scoped_docker_hub_image_keeps_its_namespace(self) -> None:
        ref = pins.parse_reference("clickhouse/clickhouse-server:26.8@sha256:" + "2" * 64)
        self.assertEqual(ref.repository, "clickhouse/clickhouse-server")
        self.assertEqual(ref.tag, "26.8")

    def test_a_registry_host_is_not_part_of_the_repository(self) -> None:
        ref = pins.parse_reference("quay.io/debezium/connect:3.7.0.Final@sha256:" + "3" * 64)
        self.assertEqual(ref.registry, "quay.io")
        self.assertEqual(ref.repository, "debezium/connect")
        self.assertEqual(ref.tag, "3.7.0.Final")

    def test_a_composite_tag_and_a_registry_with_a_port_are_parsed(self) -> None:
        ref = pins.parse_reference(
            "localhost:5000/mssql/server:2022-CU27-GDR1-ubuntu-22.04@sha256:" + "4" * 64
        )
        self.assertEqual(ref.registry, "localhost:5000")
        self.assertEqual(ref.repository, "mssql/server")
        self.assertEqual(ref.tag, "2022-CU27-GDR1-ubuntu-22.04")

    def test_a_variant_suffix_stays_part_of_the_tag(self) -> None:
        ref = pins.parse_reference("rust:1.99.1-slim@sha256:" + "5" * 64)
        self.assertEqual(ref.tag, "1.99.1-slim")

    def test_a_missing_or_short_digest_is_refused(self) -> None:
        with self.assertRaises(pins.PinError):
            pins.parse_reference("postgres:16.15")
        with self.assertRaises(pins.PinError):
            pins.parse_reference("postgres:16.15@sha256:" + "a" * 63)


class DiscoveryTest(unittest.TestCase):
    """The pin list comes from the existing digest lint, not a new parser."""

    def test_only_pinned_third_party_images_are_discovered(self) -> None:
        text = """
services:
  ours:
    image: ${LAKEHOUSE_API_IMAGE:-rantai-lakehouse-api:local}
  unpinned:
    image: postgres:16
  pinned:
    image: postgres:16.15@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
  same:
    image: postgres:16.15@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
"""
        found = pins.pinned_images(text)
        self.assertEqual(
            found,
            ["postgres:16.15@sha256:" + "a" * 64],
            "references must be unique and local/unpinned images must be absent",
        )

    def test_every_discovered_image_in_the_compose_file_has_a_digest(self) -> None:
        text = pins.DEFAULT_COMPOSE.read_text(encoding="utf-8")
        found = pins.pinned_images(text)
        self.assertGreater(len(found), 10)
        for value in found:
            self.assertFalse(check_image_digests.is_local_build(value))
            self.assertTrue(check_image_digests.check_image(value))


class VersionMatchingTest(unittest.TestCase):
    """Which declared versions name the same release as a tag."""

    def test_exact_and_leading_v_match(self) -> None:
        self.assertTrue(pins.version_matches("3.7.0.Final", "3.7.0.Final"))
        self.assertTrue(pins.version_matches("v1.8.3", "1.8.3"))
        self.assertTrue(pins.version_matches("1.0.0-rc.4", "v1.0.0-rc.4"))

    def test_an_image_may_declare_a_more_precise_patch(self) -> None:
        self.assertTrue(pins.version_matches("26.8", "26.8.36.3"))
        self.assertTrue(pins.version_matches("16.15", "16.15-1.pgdg13+2"))
        self.assertTrue(pins.version_matches("3.12-slim", "3.12.16"))
        self.assertTrue(pins.version_matches("8.4.13", "8.4.13-1.el9"))

    def test_a_prerelease_claim_does_not_match_a_stable_tag(self) -> None:
        # The #264 incident: 3.8 moved to 3.8.0.Alpha1.
        self.assertFalse(pins.version_matches("3.8", "3.8.0.Alpha1"))
        self.assertFalse(pins.version_matches("3.7.0.Final", "3.7.0.Alpha1"))
        self.assertFalse(pins.version_matches("1.0.0", "1.0.0-rc.4"))

    def test_a_different_release_series_does_not_match(self) -> None:
        self.assertFalse(pins.version_matches("3.8", "3.9.0"))
        self.assertFalse(pins.version_matches("16.15", "17.0"))
        self.assertFalse(pins.version_matches("483", "10.2"))

    def test_base_image_versions_are_not_product_versions(self) -> None:
        self.assertFalse(pins.version_matches("26.8", "22.04"))
        self.assertFalse(pins.version_matches("2022-CU27-GDR1-ubuntu-22.04", "16.0.4305.5"))

    def test_build_metadata_is_ignored(self) -> None:
        self.assertTrue(pins.version_matches("16.15", "16.15+build.7"))


class ClaimTest(unittest.TestCase):
    """Which config fields count as a claim about the pinned product."""

    def test_product_env_vars_are_authoritative(self) -> None:
        ref = parse_debezium()
        env = ["KAFKA_VERSION=4.3.1", "DEBEZIUM_VERSION=3.7.0.Final"]
        claims = pins.version_claims({"config": {"Env": env}}, ref)
        by_source = {claim.source: claim for claim in claims}
        self.assertTrue(by_source["env DEBEZIUM_VERSION"].authoritative)
        self.assertFalse(by_source["env KAFKA_VERSION"].authoritative)

    def test_product_labels_are_authoritative_generic_labels_are_not(self) -> None:
        ref = pins.parse_reference("clickhouse/clickhouse-server:26.8@sha256:" + "6" * 64)
        labels = {
            "com.clickhouse.build.version": "26.8.36.3",
            "org.opencontainers.image.version": "22.04",
            "io.buildah.version": "1.44.0",
        }
        claims = pins.version_claims({"config": {"Labels": labels}}, ref)
        by_source = {claim.source: claim for claim in claims}
        self.assertTrue(by_source["label com.clickhouse.build.version"].authoritative)
        self.assertFalse(by_source["label org.opencontainers.image.version"].authoritative)
        self.assertFalse(by_source["label io.buildah.version"].authoritative)

    def test_missing_or_null_config_fields_are_not_errors(self) -> None:
        ref = parse_debezium()
        self.assertEqual(pins.version_claims({}, ref), [])
        self.assertEqual(pins.version_claims({"config": None}, ref), [])
        self.assertEqual(
            pins.version_claims({"config": {"Labels": None, "Env": ["PATH=/bin", 42]}}, ref),
            [],
        )

    def test_pg_alias_and_mysql_shell_are_classified_correctly(self) -> None:
        pg = pins.parse_reference("postgres:16.15@sha256:" + "7" * 64)
        claims = pins.version_claims({"config": {"Env": ["PG_VERSION=16.15-1"]}}, pg)
        self.assertTrue(claims[0].authoritative)

        mysql = pins.parse_reference("mysql:8.4.13@sha256:" + "8" * 64)
        claims = pins.version_claims(
            {"config": {"Env": ["MYSQL_VERSION=8.4.13-1", "MYSQL_SHELL_VERSION=8.4.10-1"]}},
            mysql,
        )
        self.assertTrue(claims[0].authoritative)
        self.assertFalse(claims[1].authoritative)


class DecisionTest(unittest.TestCase):
    """When a mismatch is a failure and when absence is left alone."""

    def test_authoritative_mismatch_fails(self) -> None:
        ref = parse_debezium("3.8")
        claims = [
            pins.VersionClaim("env DEBEZIUM_VERSION", "3.8.0.Alpha1", True),
            pins.VersionClaim("label org.opencontainers.image.version", "44", False),
        ]
        check = pins.decide(ref, claims)
        self.assertFalse(check.ok)
        self.assertIn("3.8.0.Alpha1", check.detail)

    def test_authoritative_match_passes_even_with_decoy_labels(self) -> None:
        ref = parse_debezium()
        claims = [
            pins.VersionClaim("env DEBEZIUM_VERSION", "3.7.0.Final", True),
            pins.VersionClaim("env KAFKA_VERSION", "4.3.1", False),
            pins.VersionClaim("label org.opencontainers.image.version", "44", False),
        ]
        check = pins.decide(ref, claims)
        self.assertTrue(check.ok)
        self.assertIn("DEBEZIUM_VERSION", check.detail)

    def test_advisory_match_passes(self) -> None:
        ref = pins.parse_reference("curlimages/curl:8.10.1@sha256:" + "9" * 64)
        claims = [
            pins.VersionClaim("label version", "8.10.1", False),
            pins.VersionClaim("label io.buildah.version", "1.23.1", False),
        ]
        self.assertTrue(pins.decide(ref, claims).ok)

    def test_advisory_only_without_a_match_passes_gracefully(self) -> None:
        ref = pins.parse_reference("trinodb/trino:483@sha256:" + "a" * 64)
        claims = [pins.VersionClaim("label version", "10.2", False)]
        check = pins.decide(ref, claims)
        self.assertTrue(check.ok)
        self.assertIn("no product version declared", check.detail)

    def test_no_claims_passes(self) -> None:
        ref = pins.parse_reference("alpine:3.20@sha256:" + "b" * 64)
        self.assertTrue(pins.decide(ref, []).ok)


class RegistryFlowTest(unittest.TestCase):
    """Digest resolution, auth, retries, indexes and config blobs."""

    def test_resolves_an_index_and_reads_the_amd64_config(self) -> None:
        ref = ref_for("1.0.0", "c" * 64)
        client, http = served_client(ref, {"version": "1.0.0"}, [], other_platforms=True)
        check = pins.verify_pin(ref, client)
        self.assertTrue(check.ok, check.detail)

        # The index's arm64 entry (digest `sha256:aaaa…`) must not be read:
        # the checker picks linux/amd64 and fetches that child only.
        self.assertFalse(any("sha256:" + "a" * 64 in url for url in http.urls()))
        self.assertEqual(len([url for url in http.urls() if "/manifests/" in url]), 2)

    def test_resolves_a_single_manifest_without_an_index(self) -> None:
        ref = ref_for("3.7.0.Final", "d" * 64)
        client, http = served_client(
            ref,
            {"org.opencontainers.image.version": "44"},
            ["DEBEZIUM_VERSION=3.7.0.Final"],
            index=False,
        )
        check = pins.verify_pin(ref, client)
        self.assertTrue(check.ok, check.detail)
        manifest_urls = [url for url in http.urls() if "/manifests/" in url]
        self.assertEqual(len(manifest_urls), 1)

    def test_bearer_challenge_is_followed_and_token_reused(self) -> None:
        ref = ref_for("16.15", "e" * 64)
        client, http = served_client(ref, {}, ["PG_VERSION=16.15-1"])
        top_url = client._endpoint(ref, "manifests", ref.digest)
        challenge = (
            'Bearer realm="https://auth.example/token",'
            'service="registry.example",scope="repository:product:pull"'
        )
        http.responses[top_url].insert(0, pins.Response(401, {"www-authenticate": challenge}, b""))
        http.responses["https://auth.example/token"] = [
            pins.Response(200, {}, b'{"token":"a-token"}')
        ]
        check = pins.verify_pin(ref, client)
        self.assertTrue(check.ok, check.detail)
        authorized = [headers for url, headers in http.calls if "Authorization" in headers]
        self.assertTrue(authorized)
        self.assertTrue(all(h["Authorization"] == "Bearer a-token" for h in authorized))
        self.assertTrue(any(url == top_url for url, headers in http.calls if "Authorization" in headers))
        self.assertTrue(
            any("scope=repository%3Aproduct%3Apull" in url for url in http.urls()),
            "the challenge's scope must be sent to the token realm",
        )

    def test_a_transient_token_endpoint_error_is_retried(self) -> None:
        ref = ref_for("1.0.0", "b" * 64)
        client, http = served_client(ref, {"version": "1.0.0"}, [])
        top_url = client._endpoint(ref, "manifests", ref.digest)
        challenge = (
            'Bearer realm="https://auth.example/token",'
            'service="registry.example",scope="repository:product:pull"'
        )
        http.responses[top_url].insert(0, pins.Response(401, {"www-authenticate": challenge}, b""))
        http.responses["https://auth.example/token"] = [
            pins.Response(429, {}, b""),
            pins.Response(200, {}, b'{"token":"a-token"}'),
        ]
        sleeps: list[float] = []
        client.sleeper = sleeps.append
        check = pins.verify_pin(ref, client)
        self.assertTrue(check.ok, check.detail)
        self.assertEqual(sleeps, [1.0])

    def test_a_withdrawn_digest_fails(self) -> None:
        ref = ref_for("3.8", "f" * 64)
        client, _ = served_client(ref, {}, [], status=404)
        check = pins.verify_pin(ref, client)
        self.assertFalse(check.ok)
        self.assertIn("404", check.detail)

    def test_a_manifest_that_does_not_hash_to_the_pin_fails(self) -> None:
        ref = ref_for("1.0.0", "0" * 64)
        client, _ = served_client(ref, {"version": "1.0.0"}, [], verify_digest=True)
        check = pins.verify_pin(ref, client)
        self.assertFalse(check.ok)
        self.assertIn("hash to", check.detail)

    def test_a_manifest_that_hashes_to_the_pin_passes(self) -> None:
        payload = json.dumps({"config": {"digest": "sha256:" + "1" * 64}}).encode()
        ref = ref_for("1.0.0", digest_of(payload).split(":", 1)[1])
        http = FakeHttpClient(
            {f"https://registry.example/v2/product/manifests/{ref.digest}": [pins.Response(200, {}, payload)]}
        )
        client = pins.RegistryClient(http, verify_digest=True)
        manifest = client.manifest(ref, ref.digest)
        self.assertEqual(manifest["config"]["digest"], "sha256:" + "1" * 64)

    def test_429_and_500_are_retried_with_backoff(self) -> None:
        ref = ref_for("1.0.0", "2" * 64)
        client, http = served_client(ref, {"version": "1.0.0"}, [])
        url = client._endpoint(ref, "manifests", ref.digest)
        ok = http.responses[url][0]
        http.responses[url] = [
            pins.Response(429, {}, b""),
            pins.Response(500, {}, b""),
            ok,
        ]
        sleeps: list[float] = []
        client.sleeper = sleeps.append
        check = pins.verify_pin(ref, client)
        self.assertTrue(check.ok, check.detail)
        self.assertEqual(sleeps, [1.0, 2.0])
        self.assertEqual(len([u for u in http.urls() if u == url]), 3)

    def test_retries_are_bounded(self) -> None:
        ref = ref_for("1.0.0", "3" * 64)
        client, http = served_client(ref, {}, [], attempts=3)
        url = client._endpoint(ref, "manifests", ref.digest)
        http.responses[url] = [pins.Response(503, {}, b"") for _ in range(3)]
        client.sleeper = lambda _: None
        check = pins.verify_pin(ref, client)
        self.assertFalse(check.ok)
        self.assertIn("503", check.detail)
        self.assertEqual(len([u for u in http.urls() if u == url]), 3)

    def test_a_manifest_without_a_config_blob_has_no_claims(self) -> None:
        ref = ref_for("1.0.0", "4" * 64)
        payload = json.dumps({"schemaVersion": 1, "fsLayers": []}).encode()
        http = FakeHttpClient(
            {f"https://registry.example/v2/product/manifests/{ref.digest}": [pins.Response(200, {}, payload)]}
        )
        client = pins.RegistryClient(http, verify_digest=False)
        check = pins.verify_pin(ref, client)
        self.assertTrue(check.ok, check.detail)
        self.assertIn("no version", check.detail)


# Metadata inspected from the 18 pinned images on main (2026-11-23). Only the
# version-shaped fields are recorded: the checker ignores the rest.
RECORDED_PINS: dict[str, tuple[dict[str, str] | None, list[str]]] = {
    "postgres:16.15@sha256:1beaa3084eee2676f20a9cd9928f0bf2c5bf674f8e49f81017912183d377c61c": (
        {}, ["GOSU_VERSION=1.19", "PG_VERSION=16.15-1.pgdg13+2"],
    ),
    "clickhouse/clickhouse-server:26.8@sha256:d426d2f612e1e189c90f9a3f8dcd85a03c6851273833817a296bb327e63aa7a7": (
        {"com.clickhouse.build.version": "26.8.36.3", "org.opencontainers.image.version": "22.04"}, [],
    ),
    "rustfs/rustfs:1.0.0-rc.4@sha256:a9fbb5e5bfce09ccd0869ac9a7b0e39191c6868d75ec4c5d08ebbd5475db5d6b": (
        {"version": "v1.0.0-rc.4"}, [],
    ),
    "amazon/aws-cli:2.36.34@sha256:cf1f5e5741c33f3fee383296ba8045b96b9eb93c3de017be6688fb00356352d7": (
        None, ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
    ),
    "quay.io/lakekeeper/catalog:v0.13.3@sha256:db2ba6168eb107f22242fb7f2edc4016fa35e57bdcc606894e809c418e32e8dc": ({}, []),
    "openfga/openfga:v1.8@sha256:1e84deddc7c84de8344f3eb0d443c5439666198b68ac37c24fd76fd2e8c1f629": ({}, []),
    "curlimages/curl:8.10.1@sha256:d9b4541e214bcd85196d6e92e2753ac6d0ea699f0af5741f8c6cccbfcf00ef4b": (
        {"io.buildah.version": "1.23.1", "version": "8.10.1"}, [],
    ),
    "gotenberg/gotenberg:8.37.0@sha256:f29984bd1e226bf1b93ba90af06000afa8b315853e99d27b9aaa41b93f15c769": (
        {"org.opencontainers.image.version": "8.37.0"}, ["GOTENBERG_VERSIONS_DIR_PATH=/opt/gotenberg/versions"],
    ),
    "alpine:3.20@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc": ({}, []),
    "rust:1.99.1-slim@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247": (
        {}, ["RUST_VERSION=1.99.1"],
    ),
    "python:3.12-slim@sha256:aa943ce411423c7582288f18707ff66eddcbaeb510502d747a72aeb34f30d159": (
        {}, ["PYTHON_VERSION=3.12.16"],
    ),
    "chrislusf/seaweedfs:4.44@sha256:e67e8c385484120b78bff47ba5f4debbca47fbd27ed1a39f016f47e8baea615b": (
        {"org.opencontainers.image.version": "4.44"}, [],
    ),
    "trinodb/trino:483@sha256:db58cc93e593a2706553745f276bb119c9810e69918be56ecde088ba7ccb0534": (
        {"io.buildah.version": "1.44.0", "version": "10.2", "com.redhat.component": "ubi10-micro-container"}, [],
    ),
    "apache/kafka:3.8.0@sha256:c89f315cff967322c5d2021434b32271393cb193aa7ec1d43e97341924e57069": (
        {}, ["JAVA_VERSION=jdk-21.0.4+7"],
    ),
    "quay.io/debezium/connect:3.7.0.Final@sha256:b9b983eda59570c526ddffa86ff73e56c53180d2676158ba5a5184cfd1b59a79": (
        {
            "io.buildah.version": "1.43.5",
            "org.opencontainers.image.version": "44",
            "org.opencontainers.image.title": "fedora-minimal",
        },
        [
            "KAFKA_VERSION=4.3.1", "SCALA_VERSION=2.13", "APICURIO_VERSION=3.2.5",
            "JOLOKIA_VERSION=1.7.2", "OPENTELEMETRY_VERSION=1.23.1",
            "OPENTELEMETRY_INSTRUMENTATION_VERSION=1.23.0", "DEBEZIUM_VERSION=3.7.0.Final",
        ],
    ),
    "mysql:8.4.13@sha256:a0225d1ea6631c8bc2385dc4c2faba17bbf1834ecf369e1659a9a382bd8d5dd7": (
        {}, ["GOSU_VERSION=1.19", "MYSQL_VERSION=8.4.13-1.el9", "MYSQL_SHELL_VERSION=8.4.10-1.el9"],
    ),
    "mcr.microsoft.com/mssql/server:2022-CU27-GDR1-ubuntu-22.04@sha256:cf8f0ab2ebe61feec2eed2629061a07d247f791c2a8ddda74a4ac9f06a291718": (
        {"com.microsoft.version": "16.0.4305.5", "org.opencontainers.image.version": "22.04"},
        ["MSSQL_RPC_PORT=135", "MSSQL_PID=developer"],
    ),
    "mongo:7.0.45@sha256:8bb4442761a63a8dda452744082cdac9644b429807eb977b42324a63cef59ca4": (
        {"org.opencontainers.image.version": "22.04"},
        ["GOSU_VERSION=1.19", "JSYAML_VERSION=3.13.1", "MONGO_VERSION=7.0.45"],
    ),
}


def parse_debezium(tag: str = "3.7.0.Final") -> pins.ImageRef:
    return pins.parse_reference(f"quay.io/debezium/connect:{tag}@sha256:" + "5" * 64)


class RecordedPinsTest(unittest.TestCase):
    """Every image pinned on main passes with its inspected metadata."""

    def test_recorded_metadata_covers_every_pin_in_the_compose_file(self) -> None:
        actual = set(pins.pinned_images(pins.DEFAULT_COMPOSE.read_text(encoding="utf-8")))
        self.assertEqual(actual, set(RECORDED_PINS))

    def test_every_recorded_pin_passes(self) -> None:
        for value, (labels, env) in RECORDED_PINS.items():
            with self.subTest(pin=value):
                ref = pins.parse_reference(value)
                client, _ = served_client(ref, labels, env)
                check = pins.verify_pin(ref, client)
                self.assertTrue(check.ok, f"{value}: {check.detail}")

    def test_the_withdrawn_debezium_pin_would_have_failed(self) -> None:
        # #264: a 3.8 tag whose config names a 3.8.0.Alpha1 build. Start from
        # the recorded config of the 3.7.0.Final pin and drift its tag/version.
        actual_key = next(key for key in RECORDED_PINS if key.startswith("quay.io/debezium/connect"))
        labels, env = RECORDED_PINS[actual_key]
        stable = pins.parse_reference(actual_key)
        check = pins.decide(stable, pins.version_claims({"config": {"Labels": labels, "Env": env}}, stable))
        self.assertTrue(check.ok, check.detail)

        drifting = parse_debezium("3.8")
        drifting_env = [
            item for item in env if not item.startswith("DEBEZIUM_VERSION")
        ] + ["DEBEZIUM_VERSION=3.8.0.Alpha1"]
        check = pins.decide(
            drifting,
            pins.version_claims({"config": {"Labels": labels, "Env": drifting_env}}, drifting),
        )
        self.assertFalse(check.ok)
        self.assertIn("3.8.0.Alpha1", check.detail)


if __name__ == "__main__":
    unittest.main()
