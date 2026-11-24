#!/usr/bin/env python3
"""Static guards for the two standalone `BI-44` installation files.

# Why this suite exists

`docker-compose.bi-only-clickhouse.yml` and
`docker-compose.bi-only-trino.yml` are deployment templates for a customer
who wants dashboards, Query Studio and the assistant over their own
database, with nothing copied into the lakehouse. The owner decided that
only ONE query engine runs in such an installation so the two do not
compete for the host's memory. A compose edit that adds the second engine,
pulls in a warehouse service, or pins a different image would break that
decision without any Rust test noticing — `docker compose config` would
happily accept all of those shapes.

# What is checked, and why

- both files parse as YAML (a duplicate key is a silent last-wins in some
  parsers, so a strict loader is used);
- exactly one engine service per file, and the two names never occur
  together (the config gate in `lakehouse-api` must agree with the file);
- no warehouse-only service is present;
- the API names its mode and engine and never sets the other engine's URL;
- every image reference is byte-identical to the pinned reference in the
  root compose file, including the digest.

# What this deliberately does NOT do

It never starts containers. `docker compose up` on each file, from a clean
project name, is the proof that the daemon accepts the shape; these tests
are the fast static check that runs on every commit. No network, no Docker
daemon, no credentials.

Run: `python3 -m unittest discover -s ops/bi-only -v`.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
ROOT_COMPOSE = REPO_ROOT / "docker-compose.yml"
FILES = {
    "clickhouse": HERE / "docker-compose.bi-only-clickhouse.yml",
    "trino": HERE / "docker-compose.bi-only-trino.yml",
}

ENGINE_SERVICES = ("clickhouse", "trino")
FORBIDDEN_SERVICES = ("rustfs", "lakekeeper", "openfga", "dagster", "kafka", "seaweedfs")
CONFLICTING_URL = {"clickhouse": "TRINO_URL", "trino": "CH_URL"}
INSTALL_SERVICES = {
    "clickhouse": {
        "postgres-cert-init",
        "postgres",
        "lakehouse-db-migrate",
        "clickhouse",
        "lakehouse-api",
        "console",
        "gotenberg",
    },
    "trino": {
        "postgres-cert-init",
        "postgres",
        "lakehouse-db-migrate",
        "trino",
        "lakehouse-api",
        "console",
        "gotenberg",
    },
}
DIGEST_RE = re.compile(r"@sha256:[0-9a-f]{64}$")


def load(path: Path) -> dict:
    """Parse a compose file with duplicate keys refused, not last-wins."""

    class UniqueKeysLoader(yaml.SafeLoader):
        pass

    def unique_mapping(loader: UniqueKeysLoader, node: yaml.MappingNode) -> dict:
        mapping: dict = {}
        for key_node, value_node in node.value:
            key = loader.construct_object(key_node, deep=True)
            if key in mapping:
                raise ValueError(f"duplicate YAML key {key!r}")
            mapping[key] = loader.construct_object(value_node, deep=True)
        return mapping

    UniqueKeysLoader.add_constructor(
        yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, unique_mapping
    )
    document = yaml.load(path.read_text(encoding="utf-8"), Loader=UniqueKeysLoader)
    assert isinstance(document, dict), f"{path.name} must hold a mapping"
    return document


def images(document: dict) -> dict[str, str]:
    """Every service's `image:` value, including the repository's own ones."""
    services = document["services"]
    return {
        name: service["image"]
        for name, service in services.items()
        if isinstance(service, dict) and "image" in service
    }


class BiOnlyComposeTest(unittest.TestCase):
    """The shape of each standalone installation file."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.root = load(ROOT_COMPOSE)
        cls.documents = {key: load(path) for key, path in FILES.items()}
        cls.texts = {key: path.read_text(encoding="utf-8") for key, path in FILES.items()}

    def test_both_files_parse_and_hold_only_the_installation_services(self) -> None:
        for engine, document in self.documents.items():
            with self.subTest(engine=engine):
                self.assertEqual(set(document["services"]), INSTALL_SERVICES[engine])

    def test_the_two_engines_never_appear_together_in_either_file(self) -> None:
        for engine, document in self.documents.items():
            with self.subTest(engine=engine):
                services = set(document["services"])
                self.assertIn(engine, services)
                self.assertNotIn(
                    next(name for name in ENGINE_SERVICES if name != engine), services
                )
                # Comments count too: an operator reading the file must not
                # be told a second engine is part of this installation.
                other = next(name for name in ENGINE_SERVICES if name != engine)
                self.assertNotIn(other, self.texts[engine].lower())

    def test_no_warehouse_only_service_is_present_or_named(self) -> None:
        for engine, document in self.documents.items():
            with self.subTest(engine=engine):
                for name in FORBIDDEN_SERVICES:
                    self.assertNotIn(name, document["services"])
                    self.assertNotIn(name, self.texts[engine].lower())

    def test_the_api_names_its_mode_and_engine_and_not_the_other_engine_url(self) -> None:
        for engine, document in self.documents.items():
            with self.subTest(engine=engine):
                env = document["services"]["lakehouse-api"]["environment"]
                self.assertEqual(env["LAKEHOUSE_INSTALL_MODE"], "bi-only")
                self.assertEqual(env["LAKEHOUSE_QUERY_ENGINE"], engine)
                self.assertNotIn(CONFLICTING_URL[engine], env)
                self.assertIn(
                    {"clickhouse": "CH_URL", "trino": "TRINO_URL"}[engine], env
                )

    def test_every_image_reference_matches_the_root_compose_exactly(self) -> None:
        root_images = images(self.root)
        for engine, document in self.documents.items():
            with self.subTest(engine=engine):
                found = images(document)
                self.assertTrue(found)
                for service, value in found.items():
                    self.assertIn(service, root_images, f"unknown image for {service}")
                    self.assertEqual(
                        value,
                        root_images[service],
                        f"{engine}/{service} must reuse the root image",
                    )
                    if not value.startswith("${"):
                        self.assertRegex(value, DIGEST_RE, f"{service} is not pinned")

    def test_the_federated_file_mounts_operator_catalogs_read_only(self) -> None:
        trino = self.documents["trino"]["services"]["trino"]
        mounts = [str(mount) for mount in trino["volumes"]]
        self.assertTrue(
            any(
                "${BI_TRINO_CATALOG_DIR:-./catalog}:/etc/trino/catalog:ro" in mount
                for mount in mounts
            ),
            "the coordinator must read operator-supplied catalog properties only",
        )
