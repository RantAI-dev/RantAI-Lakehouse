#!/usr/bin/env python3
"""Tests for the files the release ships (REL-2 S4-T1, decision D14).

A release's Compose file mounts files by a path that is RELATIVE to
the Compose file: `./ops/backups/clickhouse/backups.xml`. On a host
that does not have that file, Docker creates an empty DIRECTORY in its
place — the service starts, and it runs against nothing. That is the
gap this decision closes: the release ships every file its Compose file
mounts, the signed manifest names the checksum of each of them, and the
install writes them next to the Compose file it installs.

No network, no registry and no container: the manifest is written from
files the test writes itself, and the bundle is built with a `docker`
that is a shell script.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
RELEASE_DIR = Path(__file__).resolve().parent
BUNDLE_SCRIPT = RELEASE_DIR / "make_bundle.sh"

sys.path.insert(0, str(RELEASE_DIR))

import make_manifest  # noqa: E402
import render_compose  # noqa: E402

DIGEST = "sha256:" + "a" * 64
THIRD_PARTY_DIGEST = "sha256:" + "b" * 64
IMAGE = f"ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@{DIGEST}"
THIRD_PARTY_IMAGE = f"postgres:16.15@{THIRD_PARTY_DIGEST}"
CLICKHOUSE_IMAGE = "clickhouse/clickhouse-server:26.8@sha256:" + "c" * 64

# The relative mounts the release's Compose file has TODAY. The list
# is the whole point of the test: a new relative mount of a service a
# release runs has to be added here, and the release has to ship it.
RELEASE_MOUNTS = [
    "ops/backups/clickhouse/backups.xml",
    "ops/seaweedfs/iam.template.json",
    "ops/trino/optimize_bronze.sh",
]

FAKE_DOCKER = """#!/usr/bin/env bash
# A stand-in for the docker commands the bundle script runs: it writes
# a file that stands for the image, so the bundle's logic can be tested
# without a daemon.
set -euo pipefail
command="${1:-}"
shift || true
case "$command" in
    info)
        echo '[["driver-type","io.containerd.snapshotter.v1"]]'
        ;;
    save)
        output=""
        reference=""
        while [ "$#" -gt 0 ]; do
            case "$1" in
                -o|--output) output="$2"; shift 2 ;;
                *) reference="$1"; shift ;;
            esac
        done
        printf 'a stand-in for the image tar of %s\\n' "$reference" > "$output"
        ;;
    pull|tag)
        exit 0
        ;;
    image)
        # `docker image inspect`: the stand-in says the image is NOT in
        # the store, so the bundle script pulls it first.
        exit 1
        ;;
    *)
        echo "fake docker: only 'info', 'save', 'pull', 'tag' and 'image inspect' are implemented" >&2
        exit 1
        ;;
esac
"""


class RelativeMountTest(unittest.TestCase):
    """The relative mounts of the release's Compose file."""

    def test_the_rule_names_the_relative_mounts_of_the_services_that_stay(self) -> None:
        compose = """\
services:
  clickhouse:
    image: clickhouse/clickhouse-server:26.8@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
    volumes:
      - ./ops/backups/clickhouse/backups.xml:/etc/clickhouse-server/config.d/rel2-backups.xml:ro
      - lakehouse_clickhouse_data:/var/lib/clickhouse
  g1-test-runner:
    profiles: ["test"]
    image: rust:1.99.1-slim
    volumes:
      - ./rust:/workspace
  trino-maintenance-cron:
    profiles: ["trino"]
    image: trinodb/trino:483
    volumes:
      - ./ops/trino/optimize_bronze.sh:/opt/optimize_bronze.sh:ro
"""
        self.assertEqual(
            render_compose.relative_mount_sources(compose),
            [
                "ops/backups/clickhouse/backups.xml",
                "ops/trino/optimize_bronze.sh",
            ],
        )

    def test_the_release_template_mounts_exactly_the_known_files(self) -> None:
        """The REAL template: the release ships what it mounts.

        A new relative mount of a service a release runs has to be added
        to this list — and to the release, which the manifest does.
        """
        template = render_compose.COMPOSE_PATH.read_text(encoding="utf-8")
        self.assertEqual(render_compose.relative_mount_sources(template), RELEASE_MOUNTS)

    def test_every_mounted_file_exists_in_the_repository(self) -> None:
        for mount in RELEASE_MOUNTS:
            self.assertTrue(
                (REPO_ROOT / mount).is_file(),
                f"the release mounts {mount}, and the file does not exist",
            )


class ShippedFilesTest(unittest.TestCase):
    """The files the release ships, and the manifest that names them."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.compose = self.root / "docker-compose.yml"
        self.compose.write_text(
            "services:\n"
            "  lakehouse-api:\n"
            "    image: ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@sha256:"
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"
            "  postgres:\n"
            "    image: postgres:16.15@sha256:"
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n"
            "  clickhouse:\n"
            "    image: clickhouse/clickhouse-server:26.8@sha256:"
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\n"
            "    volumes:\n"
            "      - ./ops/backups/clickhouse/backups.xml:/etc/clickhouse-server/config.d/x.xml:ro\n",
            encoding="utf-8",
        )
        (self.root / "ops" / "backups" / "clickhouse").mkdir(parents=True)
        (self.root / "ops" / "backups" / "clickhouse" / "backups.xml").write_text(
            "<clickhouse/>\n", encoding="utf-8"
        )

    def tearDown(self) -> None:
        self._directory.cleanup()

    def test_the_manifest_names_every_mounted_file_with_its_checksum(self) -> None:
        manifest = make_manifest.build_manifest(
            version="0.2.0",
            git_sha="0123456789abcdef0123456789abcdef01234567",
            published_images={"lakehouse-api": IMAGE},
            image_ids={
                IMAGE: "sha256:" + "1" * 64,
                THIRD_PARTY_IMAGE: "sha256:" + "2" * 64,
                CLICKHOUSE_IMAGE: "sha256:" + "3" * 64,
            },
            compose_path=self.compose,
            created_at="2026-11-01T12:00:00Z",
            upgrade_path=self.root / "upgrade-path.json",
        )
        expected = hashlib.sha256(
            (self.root / "ops/backups/clickhouse/backups.xml").read_bytes()
        ).hexdigest()
        self.assertEqual(
            manifest["files"]["ops/backups/clickhouse/backups.xml"], expected
        )

    def test_the_manifest_names_the_two_tools(self) -> None:
        """REL-2 S4 decision D15: the scripts ship as release files."""
        manifest = make_manifest.build_manifest(
            version="0.2.0",
            git_sha="0123456789abcdef0123456789abcdef01234567",
            published_images={"lakehouse-api": IMAGE},
            image_ids={
                IMAGE: "sha256:" + "1" * 64,
                THIRD_PARTY_IMAGE: "sha256:" + "2" * 64,
                CLICKHOUSE_IMAGE: "sha256:" + "3" * 64,
            },
            compose_path=self.compose,
            created_at="2026-11-01T12:00:00Z",
            upgrade_path=self.root / "upgrade-path.json",
        )
        for tool in ("tools/backup-local.sh", "tools/restore-from-backup.sh"):
            self.assertIn(tool, manifest["files"])
            self.assertEqual(
                manifest["files"][tool],
                hashlib.sha256(
                    (REPO_ROOT / make_manifest.TOOL_FILES[tool]).read_bytes()
                ).hexdigest(),
            )

    def test_a_mount_whose_file_does_not_exist_is_refused(self) -> None:
        """A release that ships a Compose file mounting a file it does
        not carry installs a service that runs against an empty
        directory."""
        (self.root / "ops" / "backups" / "clickhouse" / "backups.xml").unlink()
        with self.assertRaises(make_manifest.ManifestError) as caught:
            make_manifest.build_manifest(
                version="0.2.0",
                git_sha="0123456789abcdef0123456789abcdef01234567",
                published_images={"lakehouse-api": IMAGE},
                image_ids={
                IMAGE: "sha256:" + "1" * 64,
                THIRD_PARTY_IMAGE: "sha256:" + "2" * 64,
                CLICKHOUSE_IMAGE: "sha256:" + "3" * 64,
            },
                compose_path=self.compose,
                created_at="2026-11-01T12:00:00Z",
                upgrade_path=self.root / "upgrade-path.json",
            )
        self.assertIn("backups.xml", str(caught.exception))

    def test_a_manifest_that_names_no_checksum_for_a_mount_is_refused(self) -> None:
        """REL-2 S4 decision D14: the release tooling fails when the
        rendered Compose file mounts a relative source that `files` does
        not list."""
        with self.assertRaises(make_manifest.ManifestError) as caught:
            make_manifest.check_files_cover_the_mounts(
                {"docker-compose.yml": "a checksum"}, self.compose
            )
        message = str(caught.exception)
        self.assertIn("ops/backups/clickhouse/backups.xml", message)
        self.assertIn("mounts", message)

    def test_two_shipped_files_with_the_same_name_are_refused(self) -> None:
        """The online install fetches a shipped file by its name, so two
        files with the same name would be two assets with the same
        name."""
        files = [
            make_manifest.ShippedFile("ops/one/backups.xml", self.root, "a"),
            make_manifest.ShippedFile("ops/two/backups.xml", self.root, "b"),
        ]
        with self.assertRaises(make_manifest.ManifestError) as caught:
            make_manifest.check_shipped_names(files)
        self.assertIn("backups.xml", str(caught.exception))

    def test_a_shipped_file_named_after_a_release_asset_is_refused(self) -> None:
        files = [
            make_manifest.ShippedFile("ops/one/docker-compose.yml", self.root, "a"),
        ]
        with self.assertRaises(make_manifest.ManifestError):
            make_manifest.check_shipped_names(files)

    def test_the_list_of_the_shipped_files_names_the_source_of_each(self) -> None:
        shipped = make_manifest.shipped_files(self.compose)
        by_path = {item.install_path: item for item in shipped}
        self.assertIn("ops/backups/clickhouse/backups.xml", by_path)
        self.assertEqual(
            by_path["ops/backups/clickhouse/backups.xml"].source_path,
            Path(os.path.abspath(self.root / "ops/backups/clickhouse/backups.xml")),
        )
        self.assertIn("tools/backup-local.sh", by_path)


class ShippedPathTest(unittest.TestCase):
    """The path a shipped file is written at (REL-2 S4 review round 1
    SHOULD-FIX 1).

    The install writes every shipped file under its own directory, at
    the path the manifest names. A path that is absolute, or that holds
    a `..` component, would write OUTSIDE that directory — the manifest
    is signed, but a signed path still escapes.
    """

    def test_a_plain_relative_path_is_accepted(self) -> None:
        make_manifest.check_install_path("ops/backups/clickhouse/backups.xml")
        make_manifest.check_install_path("tools/backup-local.sh")

    def test_an_absolute_path_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError) as caught:
            make_manifest.check_install_path("/etc/passwd")
        message = str(caught.exception)
        self.assertIn("/etc/passwd", message)
        self.assertIn("absolute", message)

    def test_a_path_that_holds_a_parent_component_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError) as caught:
            make_manifest.check_install_path("../outside/secret.txt")
        message = str(caught.exception)
        self.assertIn("../outside/secret.txt", message)
        self.assertIn("..", message)

    def test_a_path_that_holds_a_component_that_is_not_a_normal_one_is_refused(self) -> None:
        for path in ("", "a//b", "a/", "./a", "."):
            with self.assertRaises(make_manifest.ManifestError):
                make_manifest.check_install_path(path)


class MountOutsideTheComposeDirectoryTest(unittest.TestCase):
    """A Compose file that mounts a file OUTSIDE its own directory.

    The path is a path the release tooling really meets: a Compose
    file may mount `../shared/secret.txt`, and the install would write
    it outside the install's own directory.
    """

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        (self.root / "deploy").mkdir()
        (self.root / "shared").mkdir()
        (self.root / "shared" / "secret.txt").write_text("the secret\n", encoding="utf-8")
        self.compose = self.root / "deploy" / "docker-compose.yml"
        self.compose.write_text(
            "services:\n"
            "  postgres:\n"
            "    image: postgres:16.15@sha256:"
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n"
            "    volumes:\n"
            "      - ../shared/secret.txt:/etc/secret.txt:ro\n",
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self._directory.cleanup()

    def test_a_mount_outside_the_compose_directory_is_refused(self) -> None:
        with self.assertRaises(make_manifest.ManifestError) as caught:
            make_manifest.build_manifest(
                version="0.2.0",
                git_sha="0123456789abcdef0123456789abcdef01234567",
                published_images={},
                image_ids={THIRD_PARTY_IMAGE: "sha256:" + "2" * 64},
                compose_path=self.compose,
                created_at="2026-11-01T12:00:00Z",
                upgrade_path=self.root / "upgrade-path.json",
            )
        message = str(caught.exception)
        self.assertIn("../shared/secret.txt", message)
        self.assertIn("..", message)


class BundleWithShippedFilesTest(unittest.TestCase):
    """The bundle carries the files the release ships."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        (self.bin / "docker").write_text(FAKE_DOCKER, encoding="utf-8")
        (self.bin / "docker").chmod(0o755)

        self.compose = self.root / "docker-compose.yml"
        self.compose.write_text(
            "services:\n"
            "  lakehouse-api:\n"
            "    image: ghcr.io/rantai-dev/rantai-lakehouse-api:0.2.0@sha256:"
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"
            "  postgres:\n"
            "    image: postgres:16.15@sha256:"
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n"
            "  clickhouse:\n"
            "    image: clickhouse/clickhouse-server:26.8@sha256:"
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\n"
            "    volumes:\n"
            "      - ./ops/backups/clickhouse/backups.xml:/etc/clickhouse-server/config.d/x.xml:ro\n",
            encoding="utf-8",
        )
        (self.root / "ops" / "backups" / "clickhouse").mkdir(parents=True)
        (self.root / "ops" / "backups" / "clickhouse" / "backups.xml").write_text(
            "<clickhouse/>\n", encoding="utf-8"
        )

        self.sbom_dir = self.root / "sboms"
        self.sbom_dir.mkdir()
        for name in ("rantai-lakehouse-api", "postgres"):
            (self.sbom_dir / f"{name}.cdx.json").write_text(
                json.dumps({"bomFormat": "CycloneDX"}), encoding="utf-8"
            )

        self.sigstore_bundle = self.root / "release.json.sigstore.json"
        self.sigstore_bundle.write_text(
            '{"mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json"}\n',
            encoding="utf-8",
        )

        self.manifest = self.root / "release.json"
        self.write_manifest()

        self.out = self.root / "out"

    def tearDown(self) -> None:
        self._directory.cleanup()

    def write_manifest(self) -> None:
        """The manifest, with the checksum of every file it ships."""
        files = {
            "docker-compose.yml": hashlib.sha256(
                self.compose.read_bytes()
            ).hexdigest(),
        }
        for shipped in make_manifest.shipped_files(self.compose):
            files[shipped.install_path] = shipped.sha256
        self.manifest.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "version": "0.2.0",
                    "gitSha": "0123456789abcdef0123456789abcdef01234567",
                    "createdAt": "2026-11-01T12:00:00Z",
                    "minFrom": "0.0.0",
                    "stops": [],
                    "images": {
                        "lakehouse-api": IMAGE,
                        "postgres": THIRD_PARTY_IMAGE,
                    },
                    "files": files,
                },
                indent=2,
            ),
            encoding="utf-8",
        )

    def run_bundle(self) -> subprocess.CompletedProcess[str]:
        environment = dict(os.environ)
        environment["PATH"] = f"{self.bin}{os.pathsep}{environment['PATH']}"
        return subprocess.run(
            [
                "bash",
                str(BUNDLE_SCRIPT),
                "--manifest",
                str(self.manifest),
                "--compose",
                str(self.compose),
                "--sigstore-bundle",
                str(self.sigstore_bundle),
                "--sbom-dir",
                str(self.sbom_dir),
                "--out",
                str(self.out),
            ],
            capture_output=True,
            text=True,
            env=environment,
            check=False,
        )

    def extract(self) -> Path:
        bundle = self.out / "rantai-lakehouse-0.2.0-linux-x86_64.tar.zst"
        target = self.root / "extracted"
        target.mkdir()
        subprocess.run(
            ["tar", "--zstd", "-xf", str(bundle), "-C", str(target)],
            check=True,
            capture_output=True,
        )
        return target

    @unittest.skipUnless(shutil.which("zstd"), "zstd is needed to build a bundle")
    def test_the_bundle_holds_every_file_the_release_ships(self) -> None:
        self.assertEqual(self.run_bundle().returncode, 0)
        extracted = self.extract()
        self.assertTrue(
            (extracted / "ops/backups/clickhouse/backups.xml").is_file(),
            "the mounted file is not in the bundle",
        )
        self.assertTrue(
            (extracted / "tools/backup-local.sh").is_file(),
            "the backup tool is not in the bundle",
        )
        self.assertTrue(
            (extracted / "tools/restore-from-backup.sh").is_file(),
            "the restore tool is not in the bundle",
        )

    @unittest.skipUnless(shutil.which("zstd"), "zstd is needed to build a bundle")
    def test_every_checksum_in_the_bundle_matches_the_file_it_names(self) -> None:
        self.assertEqual(self.run_bundle().returncode, 0)
        extracted = self.extract()
        result = subprocess.run(
            ["sha256sum", "-c", "SHA256SUMS"],
            cwd=extracted,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("tools/backup-local.sh", result.stdout)

    @unittest.skipUnless(shutil.which("zstd"), "zstd is needed to build a bundle")
    def test_a_shipped_file_the_manifest_does_not_name_is_refused(self) -> None:
        """The signed manifest is what says the bytes are the release's."""
        data = json.loads(self.manifest.read_text(encoding="utf-8"))
        data["files"]["ops/backups/clickhouse/backups.xml"] = "0" * 64
        self.manifest.write_text(json.dumps(data, indent=2), encoding="utf-8")
        result = self.run_bundle()
        self.assertEqual(result.returncode, 1)
        self.assertIn("backups.xml", result.stderr)
        self.assertFalse(
            (self.out / "rantai-lakehouse-0.2.0-linux-x86_64.tar.zst").exists()
        )


if __name__ == "__main__":
    unittest.main()
