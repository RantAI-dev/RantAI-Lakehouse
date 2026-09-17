"""Tests for authored_transforms.py's transform-vocabulary port (WS4 item
E2, grand plan §6). Driven by transform_test_vectors.json, the SAME file
rust/crates/lakehouse-api/src/transform_grammar.rs's tests load (once the
Rust side's own vector-driven test lands there) -- see that file and
docs/superpowers/plans/2026-09-11-ws4-pipelines-detail-source-logs.md's
Python port task for why the two languages compare accept/reject
decisions, not byte-identical rendered SQL: transform_grammar.rs's `SqlLiteral` escapes a
literal by doubling both backslashes and quotes
(`rust/crates/lakehouse-core/src/ident.rs:106-111`), while
`_sql_string_literal` here (reused from `bronze_catalog.py:284-289` per
AGENTS.md rule 4) backslash-escapes a quote instead. Both are safe
`ClickHouse` string-literal encodings; they are not the SAME encoding, so
only the accept/reject decision is checked identically across languages.

Run with: `~/.cache/rantai-dagster-venv/bin/python -m pytest
dagster/dispar_orchestrate/test_authored_transforms.py -v`
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

from dispar_orchestrate import authored_transforms

VECTORS = json.loads(
    (Path(__file__).parent / "transform_test_vectors.json").read_text()
)


class SharedVectorParityTest(unittest.TestCase):
    def test_every_shared_vector_is_accepted_or_rejected_as_labeled(self) -> None:
        for vector in VECTORS:
            with self.subTest(input=vector["input"]):
                try:
                    authored_transforms.parse_transform(vector["input"])
                    actually_accepted = True
                except authored_transforms.TransformError:
                    actually_accepted = False
                self.assertEqual(
                    actually_accepted, vector["accepted"],
                    f"{vector['input']!r}: expected accepted={vector['accepted']}, "
                    f"got {actually_accepted}",
                )


class RenderingTest(unittest.TestCase):
    def test_filter_renders_through_the_existing_sql_string_literal_helper(self) -> None:
        transform = authored_transforms.parse_transform("filter(status = 'active')")
        rendered = authored_transforms.render_clickhouse(transform)
        self.assertEqual(rendered, "status = 'active'")

    def test_a_bare_backslash_literal_is_escaped_not_passed_through_raw(self) -> None:
        transform = authored_transforms.parse_transform("filter(path = 'C:\\data')")
        rendered = authored_transforms.render_clickhouse(transform)
        # `_sql_string_literal`'s own encoding: backslash doubled, quote
        # backslash-escaped (not doubled) -- `bronze_catalog.py:284-289`.
        self.assertEqual(rendered, "path = 'C:\\\\data'")

    def test_disallowed_cast_type_is_rejected(self) -> None:
        with self.assertRaises(authored_transforms.TransformError):
            authored_transforms.parse_transform("cast(col,UDF_EVIL_TYPE)")

    def test_refusal_names_the_field_not_the_payload(self) -> None:
        # Mirrors transform_grammar.rs's refusal_names_the_field_not_the_payload
        # -- the error message must never echo the untrusted transform text.
        payload = "'; DROP TABLE pipeline_definition; --"
        try:
            authored_transforms.parse_transform(f"dedupe({payload})")
            self.fail("expected TransformError")
        except authored_transforms.TransformError as err:
            message = str(err)
            self.assertNotIn(payload, message)
