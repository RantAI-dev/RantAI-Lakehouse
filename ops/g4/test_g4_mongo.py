"""Unit tests of the pure parts of `g4_mongo_test.py` (no network). Run: `python -m unittest
test_g4_mongo` from `ops/g4` with `PYTHONPATH=../gates` (needs `pymongo`, as the gate runner has)."""
from __future__ import annotations

import unittest

import g4_mongo_test as gate


class ObjectIdTests(unittest.TestCase):
    def test_the_object_id_of_a_number_is_its_24_digit_lower_case_hex_and_back(self) -> None:
        for n in (1, 255, 20_000, 900_001, 105_000):
            text = str(gate.oid(n))
            self.assertEqual(text, f"{n:024x}")
            self.assertEqual(len(text), 24)
            self.assertEqual(text, text.lower())
            self.assertEqual(gate.number_of(text), n)

    def test_a_document_carries_the_object_id_and_the_integer_field_the_shared_steps_read(self) -> None:
        doc = gate.document(7, "seed_7", "seed", 7.5)
        self.assertEqual(str(doc["_id"]), f"{7:024x}")
        self.assertEqual(doc["id"], 7)
        self.assertEqual(set(doc), {"_id", "id", "customer", "amount", "status", "updated_at"})

    def test_the_clickhouse_check_of_the_id_text_is_the_same_formula(self) -> None:
        # `lower(leftPad(hex(id), 24, '0'))` in step_ids_are_the_object_id_text: hex() of an Int64 is
        # upper-case without leading zeros for the value; padded and lower-cased it is f"{n:024x}".
        for n in (1, 31, 20_000):
            self.assertEqual(format(n, "X").rjust(24, "0").lower(), f"{n:024x}")

    def test_the_expected_history_of_a_delete_has_no_customer(self) -> None:
        gate.EXPECTED.clear()
        gate._expect([5], "d", "")
        gate._expect(range(1, 3), "u", lambda i: f"u{i}")
        self.assertEqual(dict(gate.EXPECTED), {(5, "d", ""): 1, (1, "u", "u1"): 1, (2, "u", "u2"): 1})


if __name__ == "__main__":
    unittest.main()
