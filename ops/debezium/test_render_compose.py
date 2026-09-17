"""Tests for `render_compose.py` (WS3 Phase G).

Run directly: `python3 ops/debezium/test_render_compose.py` (no network, no
external test framework — every other gate script under `ops/` is a
standalone `unittest`/plain-assert module run the same way, e.g.
`ops/g4/g4_test.py`).
"""

from __future__ import annotations

import unittest

from render_compose import (
    DEBEZIUM_IMAGE_DIGEST,
    RenderComposeError,
    looks_like_a_resolved_secret,
    render_compose_fragment,
    sanitize_connector_id,
)


class LooksLikeAResolvedSecretTests(unittest.TestCase):
    """Independently-tested Python port of
    `lakehouse_store::connectors::looks_like_raw_secret`
    (`rust/crates/lakehouse-store/src/connectors.rs:386`) — same three
    checks, documented as intentionally duplicated logic since this script
    has no FFI into Rust."""

    def test_dsn_shape_is_flagged(self) -> None:
        self.assertTrue(
            looks_like_a_resolved_secret("postgres://user:pass@host:5432/db")
        )

    def test_pem_block_is_flagged(self) -> None:
        self.assertTrue(
            looks_like_a_resolved_secret(
                "-----BEGIN PRIVATE KEY-----\nMIIB...\n-----END PRIVATE KEY-----"
            )
        )

    def test_jwt_shape_is_flagged(self) -> None:
        self.assertTrue(
            looks_like_a_resolved_secret(
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"
            )
        )

    def test_long_unbroken_alnum_run_is_flagged(self) -> None:
        self.assertTrue(
            looks_like_a_resolved_secret("a" * 32 + "B1c2D3e4F5g6H7i8")
        )

    def test_an_env_var_reference_is_not_flagged(self) -> None:
        self.assertFalse(looks_like_a_resolved_secret("${CONNECTOR_MYSQL_PASSWORD}"))

    def test_a_properties_line_naming_a_reference_is_not_flagged(self) -> None:
        self.assertFalse(
            looks_like_a_resolved_secret(
                "debezium.source.database.password=${CONNECTOR_MYSQL_PASSWORD}\n"
            )
        )


class SanitizeConnectorIdTests(unittest.TestCase):
    def test_hyphens_become_underscores_mirroring_the_rust_slug_rule(self) -> None:
        # Same transform as `connector_slug_for_id`
        # (`rust/crates/lakehouse-api/src/routes/connectors.rs:615`):
        # `id.replace('-', '_')`, validated against `ConnectorSlug::new`'s
        # `^[a-z0-9][a-z0-9_]{0,62}$`.
        self.assertEqual(sanitize_connector_id("conn-orders-mysql"), "conn_orders_mysql")

    def test_an_id_that_would_not_pass_connectorslug_is_rejected(self) -> None:
        with self.assertRaises(RenderComposeError):
            sanitize_connector_id("Conn-Orders!")


class RenderComposeFragmentTests(unittest.TestCase):
    def test_fragment_carries_env_var_references_never_resolved_values(self) -> None:
        properties_response = {
            "properties": (
                "debezium.source.database.hostname=${CONNECTOR_MYSQL_HOST}\n"
                "debezium.source.database.password=${CONNECTOR_MYSQL_PASSWORD}\n"
            )
        }
        fragment = render_compose_fragment("conn-orders-mysql", properties_response)
        self.assertIn("${CONNECTOR_MYSQL_PASSWORD}", fragment)
        self.assertFalse(looks_like_a_resolved_secret(fragment))

    def test_fragment_pins_the_same_image_digest_as_docker_compose_yml(self) -> None:
        properties_response = {"properties": "debezium.source.database.hostname=${H}\n"}
        fragment = render_compose_fragment("conn-x", properties_response)
        self.assertIn(DEBEZIUM_IMAGE_DIGEST, fragment)

    def test_fragment_names_a_per_connector_service_and_data_volume(self) -> None:
        properties_response = {"properties": "debezium.source.database.hostname=${H}\n"}
        fragment = render_compose_fragment("conn-orders-mysql", properties_response)
        self.assertIn("debezium-conn_orders_mysql:", fragment)
        self.assertIn("lakehouse_debezium_data_conn_orders_mysql:", fragment)
        # Per-connector token subpath, distinct from the static demo
        # connector's `debezium.jwt` (WS3 plan review Z10).
        self.assertIn("debezium-conn_orders_mysql.jwt", fragment)

    def test_fragment_declares_the_same_depends_on_keys_as_the_static_service(
        self,
    ) -> None:
        properties_response = {"properties": "debezium.source.database.hostname=${H}\n"}
        fragment = render_compose_fragment("conn-x", properties_response)
        self.assertIn("g4-source-init:", fragment)
        self.assertIn("lakekeeper-authz-init:", fragment)
        self.assertIn("service_completed_successfully", fragment)

    def test_environment_block_declares_must_set_refs_never_a_default_token(
        self,
    ) -> None:
        # AGENTS.md compose rule: `${X:?}` for must-set, never a default
        # token -- these are credential-shaped env vars.
        properties_response = {
            "properties": "debezium.source.database.password=${CONNECTOR_MYSQL_PASSWORD}\n"
        }
        fragment = render_compose_fragment("conn-x", properties_response)
        self.assertIn("CONNECTOR_MYSQL_PASSWORD: ${CONNECTOR_MYSQL_PASSWORD:?}", fragment)
        self.assertNotIn(":-", fragment)

    def test_a_resolved_looking_credential_in_the_properties_response_is_rejected(
        self,
    ) -> None:
        # Defense in depth (mirrors `looks_like_raw_secret`'s own callers
        # in Rust): if the upstream API ever handed this generator a
        # resolved value instead of an `${ENV_VAR_NAME}` reference, fail
        # closed rather than writing the leak to disk.
        properties_response = {
            "properties": (
                "debezium.source.database.password=postgres://u:hunter2@host:5432/db\n"
            )
        }
        with self.assertRaises(RenderComposeError):
            render_compose_fragment("conn-x", properties_response)


if __name__ == "__main__":
    unittest.main()
