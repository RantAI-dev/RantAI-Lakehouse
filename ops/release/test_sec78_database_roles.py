#!/usr/bin/env python3
"""The database roles of the deployment template (`SEC-78`).

The console connects to Postgres as `lakehouse_app`, a role that owns no
table and is not a superuser, and the migrations run with the OWNER's
credentials in a separate service (`lakehouse-db-migrate`). Nothing in
the template itself pinned that wiring: a change could point the
migrations at a role that cannot run them, or hand the seeded
connector the owner's password, and no test would notice.

The API service's WHOLE environment map is asserted here, because the
security claim of the change is about what that container holds: no
owner credential, neither the owner's password nor the owner's name.

These tests read the REAL template (`docker-compose.yml`) and assert on
the LITERAL `${…}` expressions of its `environment` maps — a stable
text assertion, not on a rendered file, so a value that is unset in the
reader's own environment cannot change the answer.

No network, no containers: the template is a file.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))

import render_compose  # noqa: E402  (path set just above)

import yaml  # noqa: E402


class TemplateDatabaseRolesTest(unittest.TestCase):
    """Which credentials each service of the template is given."""

    @classmethod
    def setUpClass(cls) -> None:
        document = yaml.safe_load(
            render_compose.COMPOSE_PATH.read_text(encoding="utf-8")
        )
        cls.services = document["services"]

    def environment_of(self, service: str) -> dict[str, str]:
        return self.services[service]["environment"]

    def test_the_migrate_service_connects_with_the_owner_password_and_never_with_the_runtime_roles_password(self) -> None:
        url = self.environment_of("lakehouse-db-migrate")["DATABASE_URL"]
        self.assertIn("POSTGRES_PASSWORD", url)
        self.assertNotIn("LAKEHOUSE_DB_APP_PASSWORD", url)
        self.assertNotIn("lakehouse_app", url)

    def test_the_migrate_services_owner_url_cannot_be_overridden(self) -> None:
        # The migrations need DDL, and only the owner has it: an ambient
        # `DATABASE_URL` — the one the API may be pointed at — would make
        # them run as a role that cannot run them.
        url = self.environment_of("lakehouse-db-migrate")["DATABASE_URL"]
        self.assertNotIn("${DATABASE_URL", url)

    def test_the_api_service_connects_as_the_runtime_role(self) -> None:
        url = self.environment_of("lakehouse-api")["DATABASE_URL"]
        self.assertIn("lakehouse_app", url)
        self.assertIn("LAKEHOUSE_DB_APP_PASSWORD", url)

    def test_the_api_service_holds_no_owner_credential(self) -> None:
        # SEC-78 decision 4: after this, the API container holds
        # no `POSTGRES_PASSWORD`. The claim is about the WHOLE
        # environment map and not about one variable: a value added
        # anywhere in it hands the owner's password to the container.
        environment = self.environment_of("lakehouse-api")
        for name, value in environment.items():
            self.assertNotIn(
                "POSTGRES_PASSWORD",
                value,
                f"the API service's {name} carries the owner's password",
            )
            self.assertNotIn(
                "POSTGRES_USER",
                value,
                f"the API service's {name} names the owner",
            )
        self.assertIn(
            "lakehouse_app",
            environment["DATABASE_URL"],
            "the API connects as the runtime role",
        )

    def test_the_seeded_connector_dials_with_the_source_readers_password_and_never_with_the_owners(self) -> None:
        # The seeded `conn-pg-lakehouse` connector resolves its password
        # through `env:CONNECTOR_PG_PASSWORD`, so this value IS the
        # credential the seeded connector authenticates with. Since SEC-84
        # the seeded row dials as `lakehouse_source_reader`, so the value is
        # that role's password, not the console's runtime role's.
        password = self.environment_of("lakehouse-api")["CONNECTOR_PG_PASSWORD"]
        self.assertIn("LAKEHOUSE_DB_SOURCE_READER_PASSWORD", password)
        self.assertNotIn("LAKEHOUSE_DB_APP_PASSWORD", password)
        self.assertNotIn("POSTGRES_PASSWORD", password)

    def test_the_dagster_code_location_reads_the_database_as_the_source_reader_role(self) -> None:
        environment = self.environment_of("dagster-code-location")
        # The source reader is the DEFAULT, not a fixed value: an operator
        # may point the bronze source at another role (one with
        # `REPLICATION`, which the reader deliberately lacks). The SEC-84
        # reason for the change: the code location runs user pipeline code,
        # so it holds the read-only role, never the console's runtime role
        # that can write the audit trail.
        user = environment["BRONZE_SOURCE_DB_USER"]
        self.assertEqual(user, "${BRONZE_SOURCE_DB_USER:-lakehouse_source_reader}")
        self.assertNotIn("POSTGRES_USER", user)
        self.assertIn("LAKEHOUSE_DB_SOURCE_READER_PASSWORD", environment["BRONZE_SOURCE_DB_PASSWORD"])
        self.assertNotIn("POSTGRES_PASSWORD", environment["BRONZE_SOURCE_DB_PASSWORD"])
        self.assertIn("LAKEHOUSE_DB_SOURCE_READER_PASSWORD", environment["CONNECTOR_PG_PASSWORD"])

    def test_the_dagster_code_location_holds_no_console_runtime_role_password(self) -> None:
        # SEC-84: pipeline code runs in this container. The console's runtime
        # role can INSERT into `public.audit_event`, so its password must not
        # reach the container under any variable name or in any value.
        environment = self.environment_of("dagster-code-location")
        for name, value in environment.items():
            self.assertNotIn(
                "LAKEHOUSE_DB_APP_PASSWORD",
                str(name),
                f"dagster-code-location names {name}",
            )
            self.assertNotIn(
                "LAKEHOUSE_DB_APP_PASSWORD",
                str(value),
                f"dagster-code-location.{name} carries the console runtime role's password",
            )


    def test_the_dagster_services_reach_their_database_as_the_dagster_role_and_never_with_the_owners_password(self) -> None:
        """Dagster's own instance database has its own role (SEC-78, second
        change): the code location, the webserver and the daemon never see
        the owner's password."""
        for service in ("dagster-code-location", "dagster-webserver", "dagster-daemon"):
            environment = self.environment_of(service)
            self.assertEqual(environment["DAGSTER_PG_USER"], "lakehouse_dagster", service)
            self.assertIn("LAKEHOUSE_DB_DAGSTER_PASSWORD", environment["DAGSTER_PG_PASSWORD"], service)
            for name, value in environment.items():
                self.assertNotIn("POSTGRES_PASSWORD", str(value), f"{service}.{name}")
                self.assertNotIn("POSTGRES_USER", str(value), f"{service}.{name}")

    def test_the_dagster_webserver_and_daemon_wait_for_the_role_to_be_set_up(self) -> None:
        """The role and the database must exist before a Dagster service
        connects to them, so both services wait for the init service to
        complete (the code location already does)."""
        for service in ("dagster-webserver", "dagster-daemon"):
            depends_on = self.services[service]["depends_on"]
            self.assertEqual(
                depends_on["dagster-db-init"]["condition"],
                "service_completed_successfully",
                service,
            )

    def test_the_dagster_init_sets_the_role_password_without_logging_the_statement(self) -> None:
        """The password statement is a literal, so it runs with statement
        logging off (the same guard as `lakehouse-api migrate`)."""
        command = "\n".join(self.services["dagster-db-init"]["command"])
        self.assertIn("SET LOCAL log_statement = 'none'", command)
        self.assertIn("SET LOCAL log_min_error_statement = 'panic'", command)
        self.assertIn("PASSWORD :'pw'", command)
        self.assertIn("\\getenv pw DAGSTER_ROLE_PASSWORD", command)
        self.assertNotIn("-v pw=", command)
        self.assertIn("d.deptype IN ('a', 'i')", command)
        self.assertIn("ALTER DATABASE", command)
        self.assertNotIn("REASSIGN OWNED", command)


if __name__ == "__main__":
    unittest.main()
