"""Unit tests for the SaaS gate's saved-delta-link helper (`g6_saas_test._saved_delta_link`).

No network, no docker. Run from the repository root:

    python3 -m unittest discover -s ops/g6 -p 'test_*.py'

The gate module imports `requests` (as the gate itself does), so this test needs
that one package; the `repo-lints` CI job installs no third-party packages and
therefore does not discover `ops/g6` (see the handoff in
`docs/superpowers/plans/2026-10-19-data-phase-4-connectors.md`).

The regression it guards is PR #216 final review BLOCKER 19 (round 3):
SHOULD-FIX 24 (`d612215c`) stores SharePoint's link as
`{"link": ..., "object": ...}`, but the step still read it as a bare string and
died with `AttributeError: 'dict' object has no attribute 'endswith'` instead of
a named gate failure. The helper also keeps the step honest about the tie
SHOULD-FIX 24 added: a link saved for another list is refused.
"""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
# The gate imports `gate_summary`, mounted at /opt/gates in its container.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "gates"))

import g6_saas_test as gate  # noqa: E402 -- the path is set up above

OBJECT = f"{gate.SP_SITE}/55555555-5555-4555-8555-555555555555"
LINK = f"https://graph.microsoft.com/v1.0/sites/{gate.SP_SITE}/lists/55555555-5555-4555-8555-555555555555/items/delta?token=v5"
OTHER = f"{gate.SP_SITE}/66666666-6666-4666-8666-666666666666"


class SavedDeltaLinkTests(unittest.TestCase):
    def test_a_saved_link_for_this_object_is_returned(self) -> None:
        state = {"delta_link": {"link": LINK, "object": OBJECT}}
        self.assertEqual(gate._saved_delta_link(state, OBJECT), LINK)

    def test_the_pre_fix_bare_string_fails_named_rather_than_with_attribute_error(self) -> None:
        # The CI failure itself: a dict read as a string raised AttributeError, not a gate failure.
        with self.assertRaises(gate.SaasGateFailure) as caught:
            gate._saved_delta_link({"delta_link": LINK}, OBJECT)
        self.assertIn("BLOCKER 19", str(caught.exception))

    def test_a_missing_key_fails_named(self) -> None:
        with self.assertRaises(gate.SaasGateFailure):
            gate._saved_delta_link({}, OBJECT)

    def test_a_dict_without_a_link_string_fails_named(self) -> None:
        for saved in ({"object": OBJECT}, {"link": "", "object": OBJECT}, {"link": 5, "object": OBJECT}):
            with self.subTest(saved=saved):
                with self.assertRaises(gate.SaasGateFailure):
                    gate._saved_delta_link({"delta_link": saved}, OBJECT)

    def test_a_link_saved_for_another_list_is_refused(self) -> None:
        # SHOULD-FIX 24: the gate must not accept a link that was kept for a different list.
        with self.assertRaises(gate.SaasGateFailure) as caught:
            gate._saved_delta_link({"delta_link": {"link": LINK, "object": OTHER}}, OBJECT)
        self.assertIn("SHOULD-FIX 24", str(caught.exception))

    def test_the_sharepoint_steps_read_the_object_they_loaded(self) -> None:
        self.assertEqual(gate.SP_OBJECT, OBJECT)


if __name__ == "__main__":
    unittest.main()
