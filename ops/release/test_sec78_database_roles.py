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

    def test_the_seeded_connector_dials_with_the_runtime_roles_password_and_never_with_the_owners(self) -> None:
        # The seeded `conn-pg-lakehouse` connector resolves its password
        # through `env:CONNECTOR_PG_PASSWORD`, so this value IS the
        # credential the seeded connector authenticates with.
        password = self.environment_of("lakehouse-api")["CONNECTOR_PG_PASSWORD"]
        self.assertIn("LAKEHOUSE_DB_APP_PASSWORD", password)
        self.assertNotIn("POSTGRES_PASSWORD", password)

    def test_the_dagster_code_location_reads_the_database_as_the_runtime_role(self) -> None:
        environment = self.environment_of("dagster-code-location")
        # The runtime role is the DEFAULT, not a fixed value: an operator
        # may point the bronze source at another role (one with
        # `REPLICATION`, which the runtime role deliberately lacks).
        user = environment["BRONZE_SOURCE_DB_USER"]
        self.assertEqual(user, "${BRONZE_SOURCE_DB_USER:-lakehouse_app}")
        self.assertNotIn("POSTGRES_USER", user)
        self.assertIn("LAKEHOUSE_DB_APP_PASSWORD", environment["BRONZE_SOURCE_DB_PASSWORD"])
        self.assertNotIn("POSTGRES_PASSWORD", environment["BRONZE_SOURCE_DB_PASSWORD"])


if __name__ == "__main__":
    unittest.main()
