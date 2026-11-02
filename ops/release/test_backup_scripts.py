#!/usr/bin/env python3
"""Tests for the backup and restore scripts (REL-2 S4-T2, decision D15).

The two scripts are the ONE source of the backup and restore steps
(REL-2 S4 decision D15): `lakehousectl backup` and `lakehousectl
restore` run them, they are not ported to Rust. So the scripts' own
behaviour is what the tool's behaviour is.

`LAKEHOUSE_DIR` is what makes them work on an installed host: the
deployment's own directory, the one holding `docker-compose.yml` and
`.env`. It defaults to the repository's root, so a developer's
behaviour is unchanged.

No container and no network: `docker` and `curl` are stand-ins on the
PATH, and they record what they were asked to do.
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
sys.path.insert(0, str(RELEASE_DIR))

BACKUP_SCRIPT = REPO_ROOT / "ops" / "backups" / "backup-local.sh"
RESTORE_SCRIPT = REPO_ROOT / "scripts" / "restore-from-backup.sh"

FAKE_DOCKER = """#!/usr/bin/env bash
# A stand-in for the docker commands the scripts run. It records what
# it was asked to do, and it says what the script needs to hear.
set -euo pipefail
echo "docker $*" >> "${FAKE_DOCKER_LOG:-/dev/null}"
case "$*" in
    *pg_dumpall*)
        echo "-- a stand-in for the dump"
        ;;
    *"SELECT name FROM system.databases"*)
        echo "lakehouse"
        ;;
    *"SELECT 1 FROM pg_database WHERE datname"*)
        # The restore asks whether every database the dump names is
        # there again (REL-2 S4b review round 6 BLOCKER 1, decision
        # D24). The stand-in says yes.
        echo "1"
        ;;
    *"volume inspect"*)
        # The connector-secrets volume is not there: the script warns
        # and carries on.
        exit 1
        ;;
esac
"""

FAKE_CURL = """#!/usr/bin/env bash
# A stand-in for the API's own health route: it says which version
# the deployment runs.
set -euo pipefail
echo '{"version": "0.2.0"}'
"""

# The trap the two scripts carry: it says which line failed and what
# that line ran (REL-2 S4b review round 8 BLOCKER 2).
ERROR_TRAP = r"""trap 'echo "Error: ${BASH_SOURCE[0]##*/} line ${LINENO}: \`${BASH_COMMAND}\` exited with $?" >&2' ERR"""


class ScriptTestCase(unittest.TestCase):
    """A copy of the scripts, in a directory of their own."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name, body in (("docker", FAKE_DOCKER), ("curl", FAKE_CURL)):
            path = self.bin / name
            path.write_text(body, encoding="utf-8")
            path.chmod(0o755)
        self.docker_log = self.root / "docker.log"

        # The scripts, in a directory that is their own "repository
        # root": the script asks its own path where the repository is.
        (self.root / "ops" / "backups").mkdir(parents=True)
        (self.root / "scripts").mkdir(parents=True)
        self.backup = self.root / "ops" / "backups" / "backup-local.sh"
        self.restore = self.root / "scripts" / "restore-from-backup.sh"
        shutil.copy(BACKUP_SCRIPT, self.backup)
        shutil.copy(RESTORE_SCRIPT, self.restore)
        self.backup.chmod(0o755)
        self.restore.chmod(0o755)

        # The repository root, as the scripts see it: a Compose file
        # and a `.env` of their own.
        (self.root / "docker-compose.yml").write_text(
            "services:\n  postgres:\n    image: postgres:16.15\n", encoding="utf-8"
        )
        (self.root / ".env").write_text("POSTGRES_PASSWORD=y\n", encoding="utf-8")

        # The deployment's own directory: the Compose file, the `.env`
        # and the release's own files.
        self.deployment = self.root / "deployment"
        self.deployment.mkdir()
        (self.deployment / "docker-compose.yml").write_text(
            "services:\n  postgres:\n    image: postgres:16.15\n", encoding="utf-8"
        )
        (self.deployment / ".env").write_text("POSTGRES_PASSWORD=x\n", encoding="utf-8")
        (self.deployment / "release.json").write_text('{"version": "0.2.0"}\n', encoding="utf-8")
        (self.deployment / "state.json").write_text(
            '{"schemaVersion": 1, "currentVersion": "0.2.0"}\n', encoding="utf-8"
        )

    def tearDown(self) -> None:
        self._directory.cleanup()

    def environment(self, **extra: str) -> dict[str, str]:
        environment = dict(os.environ)
        environment["PATH"] = f"{self.bin}{os.pathsep}{environment['PATH']}"
        environment["FAKE_DOCKER_LOG"] = str(self.docker_log)
        environment.update(extra)
        return environment

    def docker_calls(self) -> list[str]:
        if not self.docker_log.exists():
            return []
        return self.docker_log.read_text(encoding="utf-8").splitlines()

    def clear_docker_log(self) -> None:
        """The calls recorded so far are forgotten."""
        if self.docker_log.exists():
            self.docker_log.unlink()


class BackupScriptTest(ScriptTestCase):
    """`ops/backups/backup-local.sh`."""

    def run_backup(
        self, backup_root: Path | None = None, **environment: str
    ) -> subprocess.CompletedProcess[str]:
        """The backup script, with the backup root it is given."""
        root = backup_root if backup_root is not None else self.deployment / "backups"
        return subprocess.run(
            ["bash", str(self.backup), str(root)],
            capture_output=True,
            text=True,
            env=self.environment(**environment),
            check=False,
        )

    def test_the_developers_behaviour_is_unchanged_when_the_variable_is_unset(self) -> None:
        """Without `LAKEHOUSE_DIR`, the script works on the repository
        it lives in — and it runs the command it always ran:
        `docker compose -p <project>` there.

        Naming a Compose file would stop compose from finding the
        developer's own `docker-compose.override.yml` by itself
        (REL-2 S4 review round 1 SHOULD-FIX 3).
        """
        result = self.run_backup(self.root / "backups")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.docker_calls()
        compose = [call for call in calls if "compose" in call]
        self.assertTrue(compose, f"the script never ran compose: {calls}")
        first = compose[0]
        self.assertTrue(
            first.startswith(f"docker compose -p {self.root.name} "),
            f"the command is not the one the repository always ran: {first}",
        )
        self.assertNotIn("-f ", first)
        self.assertNotIn("--env-file", first)
        self.assertNotIn("override.yml", first)

    def test_the_developers_own_override_is_left_to_compose_in_the_repository(self) -> None:
        """In the repository, compose finds `docker-compose.override.yml`
        by itself. A command that names a Compose file would stop it
        from doing so (REL-2 S4 review round 1 SHOULD-FIX 3)."""
        (self.root / "docker-compose.override.yml").write_text(
            "services:\n  postgres:\n    image: postgres:16.15\n", encoding="utf-8"
        )
        result = self.run_backup(self.root / "backups")
        self.assertEqual(result.returncode, 0, result.stderr)
        compose = [call for call in self.docker_calls() if "compose" in call]
        self.assertTrue(compose, f"the script never ran compose: {self.docker_calls()}")
        self.assertNotIn("-f ", compose[0])
        self.assertNotIn("override.yml", compose[0])

    def test_the_script_works_on_the_deployment_its_own_directory(self) -> None:
        """REL-2 S4 decision D15: `LAKEHOUSE_DIR` is the directory
        holding the Compose file and the `.env`."""
        result = self.run_backup(LAKEHOUSE_DIR=str(self.deployment))
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.docker_calls()
        compose = [call for call in calls if "compose" in call]
        self.assertTrue(compose, f"the script never ran compose: {calls}")
        self.assertIn(f"-f {self.deployment}/docker-compose.yml", compose[0])
        self.assertIn(f"--env-file {self.deployment}/.env", compose[0])

    def test_the_operators_own_override_is_used_when_there_is_one(self) -> None:
        (self.deployment / "override.yml").write_text(
            "services:\n  postgres:\n    image: postgres:16.15\n", encoding="utf-8"
        )
        result = self.run_backup(LAKEHOUSE_DIR=str(self.deployment))
        self.assertEqual(result.returncode, 0, result.stderr)
        compose = [call for call in self.docker_calls() if "compose" in call]
        self.assertIn(f"-f {self.deployment}/override.yml", compose[0])

    def test_the_backup_set_holds_the_release_and_the_state(self) -> None:
        """REL-2 S4 decision D15: the set carries `release.json` and
        `state.json`, so a restore brings the release back with the
        data."""
        result = self.run_backup(LAKEHOUSE_DIR=str(self.deployment))
        self.assertEqual(result.returncode, 0, result.stderr)
        backups = sorted((self.deployment / "backups").iterdir())
        self.assertEqual(len(backups), 1, f"the backups are {backups}")
        self.assertTrue((backups[0] / "release.json").is_file())
        self.assertTrue((backups[0] / "state.json").is_file())
        self.assertTrue((backups[0] / "manifest.json").is_file())

    def test_the_backup_is_named_after_the_version_the_caller_knows(self) -> None:
        """REL-2 S4 decision D17: the update stops the API before it
        backs up, so the script cannot ask the API which version the
        backup belongs to - and the update knows.

        The backup is named `backups/<stamp>-<old version>/`, and the
        manifest of the set says the same version.
        """
        result = self.run_backup(
            LAKEHOUSE_DIR=str(self.deployment), BACKUP_VERSION="0.1.0"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        backups = sorted((self.deployment / "backups").iterdir())
        self.assertEqual(len(backups), 1, f"the backups are {backups}")
        self.assertTrue(
            backups[0].name.endswith("-0.1.0"),
            f"the backup is not named after the release: {backups[0].name}",
        )
        manifest = json.loads((backups[0] / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["version"], "0.1.0")

    def test_the_backup_reads_the_api_when_the_caller_names_no_version(self) -> None:
        """Without `BACKUP_VERSION` the script reads the API, exactly
        as it always did."""
        result = self.run_backup(LAKEHOUSE_DIR=str(self.deployment))
        self.assertEqual(result.returncode, 0, result.stderr)
        backups = sorted((self.deployment / "backups").iterdir())
        self.assertEqual(len(backups), 1, f"the backups are {backups}")
        self.assertTrue(
            backups[0].name.endswith("-0.2.0"),
            f"the backup is not named after the API's version: {backups[0].name}",
        )

    def test_the_manifest_of_the_set_lists_every_part(self) -> None:
        result = self.run_backup(LAKEHOUSE_DIR=str(self.deployment))
        self.assertEqual(result.returncode, 0, result.stderr)
        backups = sorted((self.deployment / "backups").iterdir())
        import json

        manifest = json.loads((backups[0] / "manifest.json").read_text(encoding="utf-8"))
        names = {part["name"] for part in manifest["parts"]}
        self.assertIn("release.json", names)
        self.assertIn("state.json", names)
        self.assertIn("postgres.dump", names)


class RestoreScriptTest(ScriptTestCase):
    """`scripts/restore-from-backup.sh`."""

    def make_backup(self) -> Path:
        """A backup set, made by the backup script."""
        result = subprocess.run(
            ["bash", str(self.backup), str(self.deployment / "backups")],
            capture_output=True,
            text=True,
            env=self.environment(LAKEHOUSE_DIR=str(self.deployment)),
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return sorted((self.deployment / "backups").iterdir())[0]

    def run_restore(self, backup: Path, **environment: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(self.restore), "--local", str(backup), "--skip-stack-up"],
            capture_output=True,
            text=True,
            env=self.environment(**environment),
            check=False,
        )

    def make_backup_with_databases(self, stamp: str) -> Path:
        """A backup set whose dump names two databases — one of them a
        name that needs quoting — and whose ClickHouse part holds one
        database."""
        backup = self.root / "backups" / stamp
        (backup / "clickhouse" / stamp / "default").mkdir(parents=True)
        (backup / "postgres.dump").write_text(
            "CREATE DATABASE lakehouse WITH TEMPLATE = template0;\n"
            'CREATE DATABASE "other db" WITH TEMPLATE = template0;\n',
            encoding="utf-8",
        )
        parts = []
        for path in sorted(backup.rglob("*")):
            if path.is_file():
                data = path.read_bytes()
                parts.append(
                    {
                        "name": str(path.relative_to(backup)),
                        "sizeBytes": len(data),
                        "sha256": hashlib.sha256(data).hexdigest(),
                    }
                )
        (backup / "manifest.json").write_text(
            json.dumps(
                {
                    "version": "0.1.0",
                    "createdAt": stamp,
                    "composeProject": "rel2",
                    "parts": parts,
                }
            ),
            encoding="utf-8",
        )
        return backup

    def test_the_restore_works_on_the_deployment_its_own_directory(self) -> None:
        """REL-2 S4 decision D15."""
        backup = self.make_backup()
        result = self.run_restore(backup, LAKEHOUSE_DIR=str(self.deployment))
        self.assertEqual(result.returncode, 0, result.stderr)
        compose = [call for call in self.docker_calls() if "compose" in call]
        self.assertTrue(compose, f"the script never ran compose: {self.docker_calls()}")
        self.assertIn(f"-f {self.deployment}/docker-compose.yml", compose[0])
        self.assertIn(f"--env-file {self.deployment}/.env", compose[0])

    def test_the_developers_path_names_no_compose_file(self) -> None:
        """Without `LAKEHOUSE_DIR`, the restore runs the command it
        always ran: compose finds the Compose file — and a developer's
        own `docker-compose.override.yml` — by itself.

        Naming a Compose file would stop it from doing so (REL-2 S4
        review round 1 SHOULD-FIX 3).
        """
        backup = self.make_backup()
        self.clear_docker_log()
        result = self.run_restore(backup)
        self.assertEqual(result.returncode, 0, result.stderr)
        compose = [call for call in self.docker_calls() if "compose" in call]
        self.assertTrue(compose, f"the script never ran compose: {self.docker_calls()}")
        self.assertNotIn("-f ", compose[0])
        self.assertNotIn("--env-file", compose[0])
        self.assertNotIn("override.yml", compose[0])

    def test_the_restore_puts_the_release_and_the_state_back(self) -> None:
        backup = self.make_backup()
        # The deployment's own files change after the backup was taken.
        (self.deployment / "release.json").write_text('{"version": "0.3.0"}\n', encoding="utf-8")
        (self.deployment / "state.json").write_text(
            '{"schemaVersion": 1, "currentVersion": "0.3.0"}\n', encoding="utf-8"
        )

        result = self.run_restore(backup, LAKEHOUSE_DIR=str(self.deployment))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            "0.2.0",
            (self.deployment / "release.json").read_text(encoding="utf-8"),
            "the release was not put back",
        )
        self.assertIn(
            "0.2.0",
            (self.deployment / "state.json").read_text(encoding="utf-8"),
            "the state was not put back",
        )

    def test_the_restore_keeps_the_files_it_replaces(self) -> None:
        """A restore never destroys the only copy."""
        backup = self.make_backup()
        (self.deployment / "release.json").write_text('{"version": "0.3.0"}\n', encoding="utf-8")

        result = self.run_restore(backup, LAKEHOUSE_DIR=str(self.deployment))
        self.assertEqual(result.returncode, 0, result.stderr)
        kept = [
            path.name
            for path in self.deployment.iterdir()
            if path.name.startswith("release.json.before-restore-")
        ]
        self.assertEqual(len(kept), 1, f"the files kept are {kept}")

    def test_the_restore_replaces_the_data_it_restores(self) -> None:
        """A local restore REPLACES the data the backup holds.

        The rollback of a failed update restores onto the data the
        failed update left behind, and a restore that ADDED to that
        data would bring back a mixture that is in neither the backup
        nor the instance (REL-2 S4b review round 6 BLOCKER 1, decision
        D24). So the restore drops every database the dump names,
        drops and creates the `postgres` database again, and drops the
        ClickHouse database before it restores it.
        """
        stamp = "20261102T090000Z"
        backup = self.make_backup_with_databases(stamp)
        result = self.run_restore(backup, LAKEHOUSE_DIR=str(self.deployment))
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.docker_calls()

        def index_of(needle: str) -> int:
            for index, call in enumerate(calls):
                if needle in call:
                    return index
            self.fail(f"the restore never ran {needle!r}: {calls}")

        # Every database the dump names is dropped before the dump runs.
        restore = index_of("-v ON_ERROR_STOP=0")
        self.assertLess(
            index_of('DROP DATABASE IF EXISTS "lakehouse" WITH (FORCE);'),
            restore,
            "the database was dropped after the dump was restored",
        )
        self.assertLess(
            index_of('DROP DATABASE IF EXISTS "other db" WITH (FORCE);'),
            restore,
            "the database was dropped after the dump was restored",
        )

        # The `postgres` database is in the dump — `pg_dumpall` dumps
        # every database — and the dump never carries a CREATE DATABASE
        # postgres line, so the restore creates it again.
        index_of('DROP DATABASE IF EXISTS "postgres" WITH (FORCE);')
        index_of('CREATE DATABASE "postgres" WITH TEMPLATE = template0;')

        # The ClickHouse database is dropped before it is restored:
        # `RESTORE DATABASE` refuses a table that holds rows.
        self.assertLess(
            index_of("DROP DATABASE IF EXISTS `default` SYNC;"),
            index_of(
                f"RESTORE DATABASE `default` FROM Disk('backups', 'clickhouse/{stamp}/default')"
            ),
            "the database was dropped after it was restored",
        )


class ErrorTrapTest(unittest.TestCase):
    """The trap that says what failed (REL-2 S4b review round 8
    BLOCKER 2).

    Under `set -e` alone a failing command stops a script in silence:
    the rollback of a failed update was told only the exit status, and
    not one word about what failed. The trap names the line and the
    command.
    """

    def test_a_failing_command_says_the_line_and_the_command(self) -> None:
        """A script with the trap says which line failed and what that
        line ran, on its error stream."""
        with tempfile.TemporaryDirectory() as directory:
            script = Path(directory) / "the-script.sh"
            script.write_text(
                "#!/usr/bin/env bash\n"
                "set -euo pipefail\n"
                "set -E\n"
                f"{ERROR_TRAP}\n"
                "false\n",
                encoding="utf-8",
            )
            result = subprocess.run(
                ["bash", str(script)],
                capture_output=True,
                text=True,
                check=False,
            )

        self.assertNotEqual(result.returncode, 0, "the script was supposed to fail")
        self.assertIn(
            "the-script.sh line 5",
            result.stderr,
            f"the failure was not named: {result.stderr!r}",
        )
        self.assertIn(
            "`false`",
            result.stderr,
            f"the command was not named: {result.stderr!r}",
        )

    def test_both_scripts_carry_the_trap(self) -> None:
        """The backup and the restore both say what failed."""
        for script in (BACKUP_SCRIPT, RESTORE_SCRIPT):
            text = script.read_text(encoding="utf-8")
            self.assertIn(
                "set -E",
                text,
                f"{script.name} does not pass the ERR trap on to its functions: {text}",
            )
            self.assertIn(
                ERROR_TRAP,
                text,
                f"{script.name} does not carry the trap: {text}",
            )

    def test_the_trap_sits_beside_the_set_and_the_exit_trap_stays(self) -> None:
        """The trap is set right after `set -euo pipefail` — with the
        comment that says why, and nothing else — and the restore keeps
        its own `EXIT` trap."""
        for script in (BACKUP_SCRIPT, RESTORE_SCRIPT):
            lines = script.read_text(encoding="utf-8").splitlines()
            index = lines.index("set -euo pipefail") + 1
            # The comment that cites the finding sits between the two.
            comment = []
            while lines[index].startswith("#"):
                comment.append(lines[index].lstrip("#").strip())
                index += 1
            self.assertIn(
                "REL-2 S4b review round 8 BLOCKER 2",
                " ".join(comment),
                f"{script.name} does not cite the finding at the trap: {comment}",
            )
            self.assertEqual(
                lines[index],
                "set -E",
                f"{script.name} does not set -E right after `set -euo pipefail`",
            )
            self.assertEqual(
                lines[index + 1],
                ERROR_TRAP,
                f"{script.name} does not set the trap right after `set -euo pipefail`",
            )
        self.assertIn(
            "trap cleanup EXIT",
            RESTORE_SCRIPT.read_text(encoding="utf-8"),
            "the restore lost its EXIT trap",
        )


if __name__ == "__main__":
    unittest.main()
