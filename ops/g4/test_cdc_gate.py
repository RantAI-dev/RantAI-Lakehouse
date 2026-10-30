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


if __name__ == "__main__":
    unittest.main()
