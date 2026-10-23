"""Unit tests for the SaaS gate's marker spec helper (`g6_saas_test._markers`).

No network, no docker. Run from the repository root:

    python3 -m unittest discover -s ops/g6 -p 'test_*.py'

The gate module imports `requests` (as the gate itself does), so this test
needs that one package; the `repo-lints` CI job installs no third-party
packages and therefore does not discover `ops/g6` (see the handoff in
`docs/superpowers/plans/2026-10-19-data-phase-4-connectors.md`).

The regression it guards is PR #216 final review BLOCKER 19 (round 2): a
`"list_marker": "/pages"` string made `any(m in line for m in list_marker)`
iterate the string's characters, so every request carrying a `/` counted as
a page read and the read-count checks passed on nothing.
"""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
# The gate imports `gate_summary`, mounted at /opt/gates in its container.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "gates"))

import g6_saas_test as gate  # noqa: E402 -- the path is set up above


class MarkersTests(unittest.TestCase):
    def test_an_absent_key_is_an_empty_tuple(self) -> None:
        self.assertEqual(gate._markers({"label": "X"}, "list_marker"), ())

    def test_a_tuple_is_returned_as_it_is(self) -> None:
        self.assertEqual(gate._markers({"label": "X", "list_marker": ("/pages",)}, "list_marker"), ("/pages",))

    def test_a_bare_string_is_refused_rather_than_iterated_character_by_character(self) -> None:
        # The defect itself: as a string, "/pages" matches every line with a `/`.
        with self.assertRaises(gate.SaasGateFailure) as caught:
            gate._markers({"label": "X", "list_marker": "/pages"}, "list_marker")
        self.assertIn("bare string", str(caught.exception))
        self.assertIn("BLOCKER 19", str(caught.exception))

    def test_a_one_character_string_is_refused_too(self) -> None:
        with self.assertRaises(gate.SaasGateFailure):
            gate._markers({"label": "X", "reads_marker": "/"}, "reads_marker")


class VendorSpecTests(unittest.TestCase):
    def test_every_vendors_marker_keys_are_tuples(self) -> None:
        for product_key, spec in gate.PRODUCT_VENDORS.items():
            for key in ("list_marker", "reads_marker", "full_read_marker"):
                if key in spec:
                    with self.subTest(vendor=product_key, key=key):
                        self.assertIsInstance(spec[key], tuple)
                        self.assertTrue(all(isinstance(m, str) for m in spec[key]))

    def test_both_confluence_entries_count_page_requests_only(self) -> None:
        for product_key in ("confluence", "confluence-browser"):
            with self.subTest(vendor=product_key):
                self.assertIn("/pages", gate.PRODUCT_VENDORS[product_key]["list_marker"])
                markers = gate._markers(gate.PRODUCT_VENDORS[product_key], "list_marker")
                # A request that is not a page read must not match; a page read must.
                self.assertFalse(any(m in "POST /api/v1/connectors/4/test" for m in markers))
                self.assertTrue(any(m in "GET /wiki/api/v2/pages?limit=2&cursor=x" for m in markers))


if __name__ == "__main__":
    unittest.main()
