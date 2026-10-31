#!/usr/bin/env python3
"""CI lint for SRC-9 plan task 8 (decision 6 of
`docs/core/features/connector-gates.md`): every connector type the product
lists as supported has a gate, and the gates' own registry says so.

# What it compares

1. The types that are `supported` after all migrations, derived by replaying
   every statement that touches the `connector_type` table, in file order:
   the `INSERT INTO connector_type ... VALUES` rows of `0035` and the
   `UPDATE connector_type SET supported = ...` statements of `0043`, `0062`
   and anything later.
2. `ops/gates/coverage.json`, the registry the connector gates read and write
   their result table from (`ops/gates/gate_summary.py`).

It fails when

- (a) a supported type has no registry entry;
- (b) a registry entry names a type that is not supported (a gate for
  something the product no longer offers claims coverage of nothing);
- (c) a supported type has `gate: null` or any step `missing`, unless the
  entry carries an explicit waiver `{"until": "<backlog id>", "reason": ...}`.
  A waived type is listed in the output as waived, never silently passed, and
  a waiver on a type that has nothing left to waive is itself a failure, so
  waivers cannot outlive the gap they were written for.

# The parser is deliberately narrow

It is not a SQL parser. It understands exactly the statement shapes the
migrations use on `connector_type` and FAILS, naming the file and the
statement, on any other statement that mentions the table, so a new migration
that changes which types are supported in a way this script cannot replay can
not pass unnoticed. Understood: the `CREATE TABLE`, `ALTER TABLE ... ADD COLUMN
| ADD CONSTRAINT | DROP CONSTRAINT` (they change no row), the `INSERT` with the
four columns `(name, adapter, supported, docs_url)` and literal rows, and the
`UPDATE` with `SET supported / adapter / unsupported_reason = <literal>` and a
`WHERE` of `name = '<literal>'` plus optional `adapter` and `supported`
equalities joined by `AND`. When a migration needs another shape, extend this
script in the same commit, with a test.

Exit code 0 when nothing is wrong (waivers are printed), 1 otherwise.
"""
from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "ops" / "gates"))
import gate_summary  # noqa: E402  (path set just above)

DEFAULT_MIGRATIONS = REPO_ROOT / "rust" / "migrations"
DEFAULT_REGISTRY = REPO_ROOT / "ops" / "gates" / "coverage.json"

TABLE = re.compile(r"\bconnector_type\b", re.IGNORECASE)
_STRING = r"'(?:[^']|'')*'"


class MigrationShapeError(Exception):
    """A migration touches `connector_type` in a shape this script cannot
    replay. Raised, never skipped: see the module docstring."""


@dataclass
class TypeState:
    adapter: str | None
    supported: bool
    # The `unsupported_reason` text, tracked so a later UPDATE may name the exact
    # text it replaces (`WHERE unsupported_reason = '...'`, `SRC-4` slice 3, `0121`).
    reason: str | None = None


def _strip_comments(sql: str) -> str:
    """Remove `--` line comments and `/* */` block comments, leaving string
    literals alone (a `--` inside a quoted reason is text)."""
    out: list[str] = []
    i, n = 0, len(sql)
    while i < n:
        ch = sql[i]
        if ch == "'":
            j = i + 1
            while j < n:
                if sql[j] == "'":
                    if j + 1 < n and sql[j + 1] == "'":
                        j += 2
                        continue
                    break
                j += 1
            out.append(sql[i : j + 1])
            i = j + 1
        elif sql.startswith("--", i):
            j = sql.find("\n", i)
            i = n if j == -1 else j
        elif sql.startswith("/*", i):
            j = sql.find("*/", i + 2)
            i = n if j == -1 else j + 2
        else:
            out.append(ch)
            i += 1
    return "".join(out)


def _split_statements(sql: str) -> list[str]:
    """Split on `;` outside string literals. Dollar-quoted bodies are not
    understood; `_statements_touching` refuses a file that has both one and
    the table, so a `DO $$ ... $$` cannot hide a change."""
    statements: list[str] = []
    current: list[str] = []
    i, n = 0, len(sql)
    while i < n:
        ch = sql[i]
        if ch == "'":
            j = i + 1
            while j < n:
                if sql[j] == "'":
                    if j + 1 < n and sql[j + 1] == "'":
                        j += 2
                        continue
                    break
                j += 1
            current.append(sql[i : j + 1])
            i = j + 1
        elif ch == ";":
            statements.append("".join(current).strip())
            current = []
            i += 1
        else:
            current.append(ch)
            i += 1
    tail = "".join(current).strip()
    if tail:
        statements.append(tail)
    return [s for s in statements if s]


def _statements_touching(path: Path) -> list[str]:
    text = _strip_comments(path.read_text(encoding="utf-8"))
    if TABLE.search(text) and "$$" in text:
        raise MigrationShapeError(
            f"{path.name}: mentions connector_type and contains a dollar-quoted body ($$), "
            "which this check cannot read; extend ops/lint/check_connector_gate_coverage.py"
        )
    return [s for s in _split_statements(text) if TABLE.search(s)]


def _literal(token: str) -> str | None:
    """The value of `NULL` or a quoted string literal (unescaped)."""
    token = token.strip()
    if token.upper() == "NULL":
        return None
    if re.fullmatch(_STRING, token):
        return token[1:-1].replace("''", "'")
    raise ValueError(token)


def _split_outside_strings(text: str, separator: str) -> list[str]:
    """Split on `separator` (a regex) only where it is outside a quoted string."""
    pieces: list[str] = []
    last = 0
    for string_or_sep in re.finditer(f"{_STRING}|({separator})", text, re.IGNORECASE):
        if string_or_sep.group(1):
            pieces.append(text[last : string_or_sep.start()])
            last = string_or_sep.end()
    pieces.append(text[last:])
    return [p.strip() for p in pieces]


def _bool(token: str) -> bool:
    lowered = token.strip().lower()
    if lowered not in ("true", "false"):
        raise ValueError(token)
    return lowered == "true"


_INSERT = re.compile(
    r"INSERT\s+INTO\s+connector_type\s*\(\s*name\s*,\s*adapter\s*,\s*supported\s*,\s*docs_url\s*\)\s*VALUES\s*"
    r"(?P<rows>.+?)\s*(?P<conflict>ON\s+CONFLICT\s+DO\s+NOTHING)?\s*$",
    re.IGNORECASE | re.DOTALL,
)
_ROW = re.compile(
    rf"\(\s*(?P<name>{_STRING})\s*,\s*(?P<adapter>NULL|{_STRING})\s*,\s*(?P<supported>true|false)\s*,\s*NULL\s*\)",
    re.IGNORECASE,
)
_UPDATE = re.compile(
    r"UPDATE\s+connector_type\s+SET\s+(?P<set>.+?)\s+WHERE\s+(?P<where>.+?)\s*$",
    re.IGNORECASE | re.DOTALL,
)
_ALTER = re.compile(
    r"ALTER\s+TABLE\s+connector_type\s+(ADD\s+COLUMN|ADD\s+CONSTRAINT|DROP\s+CONSTRAINT)\b",
    re.IGNORECASE,
)
_CREATE = re.compile(r"CREATE\s+TABLE\s+connector_type\s*\(", re.IGNORECASE)


def _apply_insert(statement: str, types: dict[str, TypeState]) -> None:
    match = _INSERT.fullmatch(statement)
    if not match:
        raise ValueError("an INSERT INTO connector_type that is not (name, adapter, supported, docs_url) VALUES (...)")
    rows_text = match.group("rows")
    rows = list(_ROW.finditer(rows_text))
    leftover = _ROW.sub("", rows_text)
    if not rows or re.sub(r"[\s,]", "", leftover):
        raise ValueError("VALUES rows that are not ('name', 'adapter'|NULL, true|false, NULL)")
    for row in rows:
        name = _literal(row.group("name"))
        assert name is not None  # the row pattern admits only a quoted string here
        if name in types:
            if match.group("conflict"):
                continue  # ON CONFLICT DO NOTHING keeps the existing row
            raise ValueError(f"INSERT of {name!r}, which already exists, without ON CONFLICT DO NOTHING")
        types[name] = TypeState(_literal(row.group("adapter")), _bool(row.group("supported")))


def _apply_update(statement: str, types: dict[str, TypeState]) -> None:
    match = _UPDATE.fullmatch(statement)
    if not match:
        raise ValueError("an UPDATE connector_type that is not UPDATE ... SET ... WHERE ...")
    assignments: dict[str, str] = {}
    for part in _split_outside_strings(match.group("set"), ","):
        column, eq, value = part.partition("=")
        column = column.strip().lower()
        if not eq or column not in ("supported", "adapter", "unsupported_reason") or column in assignments:
            raise ValueError(f"SET of {part!r}: only supported, adapter and unsupported_reason, once each, are understood")
        assignments[column] = value.strip()
    conditions: dict[str, str] = {}
    for part in _split_outside_strings(match.group("where"), r"\s+AND\s+"):
        is_null = re.fullmatch(r"(adapter)\s+IS\s+NULL", part, re.IGNORECASE)
        if is_null:
            column, value = "adapter", "NULL"
        else:
            column, eq, value = (x.strip() for x in part.partition("="))
            column = column.lower()
            if not eq or column not in ("name", "adapter", "supported", "unsupported_reason") or column in conditions:
                raise ValueError(
                    f"WHERE of {part!r}: only name = '...', adapter = '...' | IS NULL, supported = true|false "
                    "and unsupported_reason = '...' are understood"
                )
        conditions[column] = value
    if "name" not in conditions:
        raise ValueError("a WHERE without name = '...'")
    name = _literal(conditions["name"])
    state = types.get(name or "")
    if state is None:
        return  # no such row: the UPDATE changes nothing, as in SQL
    if "adapter" in conditions and _literal(conditions["adapter"]) != state.adapter:
        return
    if "supported" in conditions and _bool(conditions["supported"]) != state.supported:
        return
    if "unsupported_reason" in conditions and _literal(conditions["unsupported_reason"]) != state.reason:
        return
    if "supported" in assignments:
        state.supported = _bool(assignments["supported"])
    if "adapter" in assignments:
        state.adapter = _literal(assignments["adapter"])
    if "unsupported_reason" in assignments:
        state.reason = _literal(assignments["unsupported_reason"])


def supported_types(migrations_dir: Path) -> list[str]:
    """The names of the types `supported = true` after replaying every
    migration, in registry-neutral (sorted) order.

    # Errors

    Raises `MigrationShapeError` for a statement on `connector_type` it does
    not understand, and `FileNotFoundError` if there are no migrations."""
    files = sorted(migrations_dir.glob("*.sql"))
    if not files:
        raise FileNotFoundError(f"no .sql files in {migrations_dir}")
    types: dict[str, TypeState] = {}
    for path in files:
        for statement in _statements_touching(path):
            try:
                if _CREATE.match(statement) or _ALTER.match(statement):
                    continue
                if re.match(r"INSERT\b", statement, re.IGNORECASE):
                    _apply_insert(statement, types)
                elif re.match(r"UPDATE\b", statement, re.IGNORECASE):
                    _apply_update(statement, types)
                else:
                    raise ValueError("a statement that is not CREATE TABLE, ALTER TABLE ADD COLUMN|ADD CONSTRAINT|DROP CONSTRAINT, INSERT or UPDATE")
            except ValueError as exc:
                one_line = " ".join(statement.split())
                raise MigrationShapeError(
                    f"{path.name}: connector_type is touched in a shape this check does not understand "
                    f"({exc}): {one_line[:200]}. Extend ops/lint/check_connector_gate_coverage.py in the same commit, with a test."
                ) from exc
    return sorted(name for name, state in types.items() if state.supported)


@dataclass
class Report:
    problems: list[str]
    waived: list[str]
    covered: list[str]


def compare(supported: list[str], registry: gate_summary.Registry) -> Report:
    problems: list[str] = []
    waived: list[str] = []
    covered: list[str] = []
    listed = set(registry.type_names())
    for name in supported:
        if name not in listed:
            problems.append(f"{name}: supported by the migrations and absent from ops/gates/coverage.json")
    for name in registry.type_names():
        if name not in supported:
            problems.append(f"{name}: in ops/gates/coverage.json but not a supported connector type in the migrations")
            continue
        gaps = []
        if registry.gate_of(name) is None:
            gaps.append("no gate")
        gaps += [f"{step} missing" for step in gate_summary.STEPS if registry.expectation(name, step) == "missing"]
        waiver = registry.waiver(name)
        if gaps and waiver is None:
            problems.append(f"{name}: supported but not covered ({', '.join(gaps)}) and not waived")
        elif gaps:
            waived.append(f"{name}: WAIVED until {waiver['until']} ({', '.join(gaps)}). {waiver['reason']}")
        elif waiver is not None:
            problems.append(f"{name}: carries a waiver (until {waiver['until']}) but is fully covered; remove the waiver")
        else:
            covered.append(name)
    return Report(problems, waived, covered)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--migrations", type=Path, default=DEFAULT_MIGRATIONS)
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    args = parser.parse_args(argv)
    try:
        supported = supported_types(args.migrations)
        registry = gate_summary.Registry.load(args.registry)
    except (MigrationShapeError, FileNotFoundError, gate_summary.RegistryError) as exc:
        print(f"[connector-gate-coverage] FAILED: {exc}", file=sys.stderr)
        return 1
    report = compare(supported, registry)
    print(f"[connector-gate-coverage] {len(supported)} supported connector types: {', '.join(supported)}")
    print(f"[connector-gate-coverage] fully covered ({len(report.covered)}): {', '.join(report.covered) or 'none'}")
    for line in report.waived:
        print(f"[connector-gate-coverage] {line}")
    if report.problems:
        for line in report.problems:
            print(f"[connector-gate-coverage] FAILED: {line}", file=sys.stderr)
        return 1
    print(
        f"[connector-gate-coverage] OK: every supported type is covered or explicitly waived "
        f"({len(report.waived)} waived)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
