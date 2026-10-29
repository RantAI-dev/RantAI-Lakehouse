"""Unit tests for the pure functions of `measure_cdc.py` (SRC-4 slice 0). No network.

Run with: python3 -m pytest ops/src4/test_measure_cdc.py -q
"""

from __future__ import annotations

import os
import sys

import pytest

sys.path.insert(0, os.path.dirname(__file__))
import measure_cdc as m  # noqa: E402


def test_percentile_is_the_nearest_rank_and_an_observed_value() -> None:
    values = [float(i) for i in range(1, 31)]
    assert m.percentile(values, 50) == 15.0
    assert m.percentile(values, 95) == 29.0  # ceil(0.95 * 30) = 29th value
    assert m.percentile(values, 100) == 30.0
    assert m.percentile([4.0], 95) == 4.0
    with pytest.raises(ValueError):
        m.percentile([], 95)


def test_latency_stats_name_the_count_and_round_to_hundredths() -> None:
    assert m.latency_stats([1.0, 2.0, 3.0]) == {"n": 3, "p50_s": 2.0, "p95_s": 3.0, "max_s": 3.0}


def test_memory_usage_is_read_in_bytes_from_either_unit_family() -> None:
    assert m.parse_mem_usage("1GiB / 15.6GiB") == 1024**3
    assert m.parse_mem_usage("512MiB / 15.6GiB") == 512 * 1024**2
    assert m.parse_mem_usage("1.5GB / 16GB") == 1_500_000_000
    with pytest.raises(ValueError):
        m.parse_mem_usage("--")


def test_the_freshness_events_are_thirty_of_each_kind_on_ids_the_bulk_step_leaves_alone() -> None:
    events = m.freshness_events()
    assert len(events) == 90
    for kind in m.KINDS:
        assert sum(1 for e in events if e.kind == kind) == 30
    ids = [e.row_id for e in events]
    assert len(set(ids)) == 90
    assert not set(ids) & set(m.BULK_UPDATE_IDS)
    assert not set(ids) & set(m.BULK_DELETE_SEED_IDS)


def test_the_freshness_statements_are_what_the_event_says() -> None:
    insert, update, delete = m.freshness_events()[:3]
    assert insert.sql("orders").startswith("INSERT INTO src4.orders") and "'fi0'" in insert.sql("orders")
    assert update.sql("orders") .startswith("UPDATE src4.orders SET customer = 'fu0'")
    assert delete.sql("orders_history") == "DELETE FROM src4.orders_history WHERE id = 9001"


def test_an_event_is_visible_in_current_mode_only_when_bronze_shows_its_effect() -> None:
    insert, update, delete = m.freshness_events()[:3]
    assert not m.visible_in_current(insert, {})
    assert m.visible_in_current(insert, {1_000_000: "fi0"})
    assert not m.visible_in_current(update, {1: "seed_1"})
    assert m.visible_in_current(update, {1: "fu0"})
    assert not m.visible_in_current(delete, {9001: "seed_9001"})
    assert m.visible_in_current(delete, {})


def test_an_event_is_visible_in_history_mode_only_with_its_own_operation() -> None:
    insert, update, delete = m.freshness_events()[:3]
    assert not m.visible_in_history(update, {(1, "r", "seed_1")})
    assert m.visible_in_history(update, {(1, "r", "seed_1"), (1, "u", "fu0")})
    assert not m.visible_in_history(delete, {(9001, "r", "seed_9001")})
    assert m.visible_in_history(delete, {(9001, "d", "seed_9001")})
    assert m.visible_in_history(insert, {(1_000_000, "c", "fi0")})


def test_the_bulk_step_is_exactly_108000_changes_in_the_agreed_mix() -> None:
    assert (m.BULK_UPDATES, m.BULK_INSERTS, m.BULK_DELETES) == (75_600, 21_600, 10_800)
    assert m.BULK_TOTAL == 108_000
    assert round(m.BULK_UPDATES / m.BULK_TOTAL, 2) == 0.70
    assert round(m.BULK_INSERTS / m.BULK_TOTAL, 2) == 0.20
    assert round(m.BULK_DELETES / m.BULK_TOTAL, 2) == 0.10


def test_the_bulk_updates_add_up_from_the_passes_over_the_key_ranges() -> None:
    keys = len(m.BULK_UPDATE_IDS)
    assert keys == 3_940
    passes, remainder = divmod(m.BULK_UPDATES, keys)
    stmts = m.bulk_statements("orders")
    update_passes = [s for s in stmts if s.startswith("UPDATE") and "'bu" in s and "bur_" not in s]
    assert len(update_passes) == passes == 19
    assert any(f"BETWEEN 31 AND {30 + remainder}" in s for s in stmts)
    assert remainder == 740 and 30 + remainder < 3_000  # the partial pass stays inside the first key range


def test_the_bulk_deletes_are_seed_rows_and_a_part_of_the_new_rows() -> None:
    assert len(m.BULK_DELETE_SEED_IDS) + m.BULK_DELETE_NEW == m.BULK_DELETES
    assert m.BULK_DELETE_NEW <= m.BULK_INSERTS


def test_the_sentinel_is_the_last_statement_and_a_key_the_bulk_step_never_deletes() -> None:
    assert m.SENTINEL_ID in m.BULK_UPDATE_IDS
    assert m.SENTINEL_ID not in m.BULK_DELETE_SEED_IDS
    assert m.sentinel_statement("orders").endswith("WHERE id = 9999")


ROW = ("1", "a", "1.50", "seed")


def test_equal_tables_compare_equal() -> None:
    result = m.compare_current([ROW, ("2", "b", "2.00", "x")], [("2", "b", "2.00", "x"), ROW])
    assert result["equal"] and result["missing"] == result["extra"] == result["duplicated_keys"] == 0


def test_a_missing_extra_duplicated_or_different_row_is_never_equal() -> None:
    assert m.compare_current([ROW], [])["missing"] == 1
    assert m.compare_current([], [ROW])["extra"] == 1
    dup = m.compare_current([ROW], [ROW, ROW])
    assert dup["duplicated_keys"] == 1 and not dup["equal"]
    diff = m.compare_current([ROW], [("1", "a", "9.99", "seed")])
    assert diff["different"] == 1 and not diff["equal"]


def test_history_in_source_order_is_equal() -> None:
    expected = [("1", "r", "a", "s"), ("1", "u", "b", "s"), ("2", "r", "x", "s"), ("1", "d", "b", "s")]
    bronze = [("1", "r", "a", "s", "0"), ("2", "r", "x", "s", "1"), ("1", "u", "b", "s", "2"), ("1", "d", "b", "s", "3")]
    assert m.check_history(expected, bronze)["equal"]


def test_history_with_a_lost_a_repeated_or_reordered_event_is_never_equal() -> None:
    expected = [("1", "r", "a", "s"), ("1", "u", "b", "s")]
    lost = m.check_history(expected, [("1", "r", "a", "s", "0")])
    assert lost["missing"] == 1 and not lost["equal"]
    twice = m.check_history(expected, [("1", "r", "a", "s", "0"), ("1", "u", "b", "s", "1"), ("1", "u", "b", "s", "2")])
    assert twice["extra_or_duplicated"] == 1 and not twice["equal"]
    swapped = m.check_history(expected, [("1", "u", "b", "s", "0"), ("1", "r", "a", "s", "1")])
    assert swapped["keys_out_of_order"] == 1 and not swapped["equal"]


def test_the_connector_flattens_rewrites_deletes_and_adds_the_two_fields_the_applier_reads() -> None:
    cfg = m.connector_config("pw")
    assert cfg["transforms.unwrap.type"].endswith("ExtractNewRecordState")
    assert cfg["transforms.unwrap.delete.handling.mode"] == "rewrite"
    assert cfg["transforms.unwrap.add.fields"] == "op,source.ts_ms"
    assert cfg["value.converter.schemas.enable"] == "false"
    assert cfg["slot.name"] == "src4_slot" and cfg["publication.name"] == "src4_pub"
    assert not cfg["slot.name"].startswith("p5cdc")
    assert cfg["topic.prefix"] == m.TOPIC_PREFIX


def test_batch_lines_are_read_from_run_logs_and_other_lines_ignored() -> None:
    lines = ["noise", "CDC_BATCH target=current events=10000 seconds=3.25", "CDC_APPLY_DONE target=current events=10000"]
    assert m.parse_batch_lines(lines) == [{"target": "current", "events": 10000, "seconds": 3.25}]
    summary = m.batch_summary([{"target": "current", "events": 1000, "seconds": 2.0}, {"target": "history", "events": 5, "seconds": 1.0}], "current")
    assert summary["batches"] == 1 and summary["seconds_per_1000_events"] == 2.0
    assert m.batch_summary([], "current") == {"batches": 0}


def _phase(p95: float, equal: bool = True) -> dict:
    fresh = {k: {"n": 30, "p50_s": 1.0, "p95_s": p95, "max_s": p95, "not_visible": 0} for k in m.KINDS}
    return {"freshness": fresh, "correctness": {"check": {"equal": equal}}, "catch_up": {"check": {"equal": equal}}}


def test_every_target_is_met_only_from_measured_numbers() -> None:
    ok = m.verdicts({"modes": {"current": _phase(60.0), "history": _phase(119.9)}})
    assert m.all_met(ok)
    slow = m.verdicts({"modes": {"current": _phase(120.1), "history": _phase(10.0)}})
    assert slow["freshness_p95_within_120s"] == {"current": "MISSED", "history": "met"} and not m.all_met(slow)
    wrong = m.verdicts({"modes": {"current": _phase(10.0, equal=False), "history": _phase(10.0)}})
    assert wrong["correctness"]["current"] == "MISSED" and wrong["catch_up_correctness"]["current"] == "MISSED"


def test_a_target_with_no_number_is_not_measured_and_never_met() -> None:
    nothing = m.verdicts({"modes": {}})
    assert not m.all_met(nothing)
    assert set(nothing["freshness_p95_within_120s"].values()) == {"not measured"}
    hidden = _phase(5.0)
    hidden["freshness"]["delete"] = {"not_visible": 3}
    assert m.verdicts({"modes": {"current": hidden, "history": _phase(5.0)}})["freshness_p95_within_120s"]["current"] == "MISSED"
