"""Unit tests of the pure parts of `cdc_gate.py` (no network). Run: `python -m unittest test_cdc_gate`
from `ops/g4` with `PYTHONPATH=../gates`."""
from __future__ import annotations

import unittest

import cdc_gate

TOPIC = "cdc.conn-x1.schema-history"

# The shape kafka-python 3.0.11's `describe_configs(..., config_filter="all")` returns, written from
# its source (`kafka/admin/_configs.py::_describe_configs_process_responses`): resource type name in
# lower case, then the resource name, then each config key with the fields of the response struct
# (`name` is popped into the key, `config_source` becomes a name).
RECORDED = {
    "topic": {
        TOPIC: {
            "cleanup.policy": {"value": "delete", "read_only": False, "is_default": False, "is_sensitive": False, "config_source": "DYNAMIC_TOPIC_CONFIG"},
            "retention.ms": {"value": "-1", "read_only": False, "is_default": False, "is_sensitive": False, "config_source": "DYNAMIC_TOPIC_CONFIG"},
            "retention.bytes": {"value": "-1", "read_only": False, "is_default": False, "is_sensitive": False, "config_source": "DYNAMIC_TOPIC_CONFIG"},
        }
    }
}


class TopicConfigValueTests(unittest.TestCase):
    def test_reads_the_value_of_a_key_in_the_recorded_answer(self) -> None:
        self.assertEqual(cdc_gate.topic_config_value(RECORDED, TOPIC, "retention.ms"), "-1")
        self.assertEqual(cdc_gate.topic_config_value(RECORDED, TOPIC, "retention.bytes"), "-1")

    def test_a_key_the_answer_lacks_is_none_not_a_default(self) -> None:
        self.assertIsNone(cdc_gate.topic_config_value(RECORDED, TOPIC, "segment.bytes"))

    def test_another_topic_or_an_empty_answer_is_none(self) -> None:
        self.assertIsNone(cdc_gate.topic_config_value(RECORDED, "cdc.other.schema-history", "retention.ms"))
        self.assertIsNone(cdc_gate.topic_config_value({}, TOPIC, "retention.ms"))

    def test_a_seven_day_retention_is_read_as_such(self) -> None:
        answer = {"topic": {TOPIC: {"retention.ms": {"value": "604800000"}, "retention.bytes": {"value": "-1"}}}}
        self.assertEqual(cdc_gate.topic_config_value(answer, TOPIC, "retention.ms"), "604800000")


class RetentionTests(unittest.TestCase):
    def test_minus_one_and_debeziums_long_max_value_are_unlimited(self) -> None:
        self.assertTrue(cdc_gate.retention_is_unlimited("-1"))
        self.assertTrue(cdc_gate.retention_is_unlimited("9223372036854775807"))

    def test_the_brokers_seven_day_default_and_junk_are_not(self) -> None:
        self.assertFalse(cdc_gate.retention_is_unlimited("604800000"))
        self.assertFalse(cdc_gate.retention_is_unlimited(None))
        self.assertFalse(cdc_gate.retention_is_unlimited("not a number"))


class _AgentSource(cdc_gate.Source):
    cdc_type = "SQL Server CDC"
    gate = "g4-mssql"

    def unchecked_is_expected(self, check: dict) -> bool:
        return check["setting"] == "SQL Server Agent"


class SetupCheckTests(unittest.TestCase):
    def setUp(self) -> None:
        cdc_gate.SOURCE = _AgentSource()

    def tearDown(self) -> None:
        cdc_gate.SOURCE = None

    @staticmethod
    def check(setting: str, status: str) -> dict:
        return {"setting": setting, "status": status, "message": "m"}

    def test_every_check_ok_or_the_expected_unchecked_one_passes(self) -> None:
        cdc_gate._expect_rest_ok([self.check("a", "ok"), self.check("SQL Server Agent", "unchecked")], None)

    def test_an_unchecked_check_the_login_could_read_is_a_failure(self) -> None:
        with self.assertRaises(cdc_gate.G4Failure):
            cdc_gate._expect_rest_ok([self.check("a", "unchecked")], None)

    def test_a_missing_check_is_a_failure_unless_it_is_the_one_expected(self) -> None:
        checks = [self.check("a", "ok"), self.check("b", "missing")]
        with self.assertRaises(cdc_gate.G4Failure):
            cdc_gate._expect_rest_ok(checks, None)
        cdc_gate._expect_rest_ok(checks, "b")

    def test_the_lag_line_names_each_table_and_its_seconds_only_when_behind(self) -> None:
        capture = {"lag": {"tables": [
            {"table": "g4_cdc.orders", "status": "behind", "seconds": 90},
            {"table": "g4_cdc.orders_history", "status": "up_to_date"},
        ]}}
        self.assertEqual(cdc_gate._lag_words(capture), "g4_cdc.orders behind 90s, g4_cdc.orders_history up_to_date")


if __name__ == "__main__":
    unittest.main()
