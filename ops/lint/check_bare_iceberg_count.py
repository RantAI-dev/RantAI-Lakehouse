#!/usr/bin/env python3
"""CI lint for R10/R11 (`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §5): fail
the build on a bare, unqualified `count()` / `count(*)` / `count(<col>)`
against a Bronze **Iceberg** table.

# The defect this guards against (measured, ClickHouse 26.3 — see
`docs/plans/P5-RESULT.md`)

An unqualified `count()`/`count(*)`/`count(<col>)` against an Iceberg table
carrying merge-on-read *equality delete* files takes a metadata-only fast
path that does not subtract deleted rows — P5 measured 8 where 6 was
correct. Adding any `WHERE` or `GROUP BY` forces the row-scan path and is
correct. This returns a **wrong number, not an error**: no test fails, no
log line appears, a dashboard is just quietly wrong. See
`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §5, row R11.

# What this script does

Scans Python/Rust/TypeScript sources for SQL statements that both:

  1. call `count(...)` (any argument, including none / `*` / a column), and
  2. target something that looks like an Iceberg/Bronze catalog table —
     ClickHouse's `DataLakeCatalog` engine, `allow_database_iceberg`, a
     database named `icecat*`, or a backtick/plain table reference whose
     name starts with the literal `bronze.` (NOT `bronze_meta.` — see
     below),

and does NOT also contain a `WHERE` or `GROUP BY` in the same statement
window. Statements matching all of that fail the check.

# What this deliberately does NOT cover (false-positive / false-negative
boundary — read this before trusting a clean run)

- **Scope is statement-local text matching, not a SQL parser.** A
  "statement" is approximated as the text from the nearest preceding
  `SELECT` (case-insensitive) up to the nearest following `;`, a `FORMAT`
  clause, or a blank line — whichever comes first, capped at 800
  characters either direction. This is intentionally the same shape of
  heuristic already used by this codebase's own SQL-adjacent tooling
  (`ops/g4/g4_test.py`'s own count-audit rule, applied by convention); it
  is not a substitute for an actual SQL AST.
- **`bronze_meta.*` / `bronze_meta_sec.*` (the console's MergeTree
  registry) are explicitly NOT flagged.** They are ordinary MergeTree
  tables, not Iceberg, and a bare `count()` against them is correct and
  used throughout `lakehouse-api` (`routes/catalog.rs`, `routes/overview.rs`,
  `routes/storage.rs`, ...). The marker used to detect "this is the
  dangerous Iceberg path" is the literal substring `bronze.` (a dot
  directly after `bronze`); `bronze_meta.` never matches that substring
  because of the intervening `_meta`, so the two cannot be confused by
  this heuristic. `silver.*`, `serving.*`, and `system.*` are likewise
  never flagged — they carry no Iceberg/`icecat`/`DataLakeCatalog` marker.
- **A `count()` call physically far (>800 chars) from its `SELECT`/`WHERE`
  in unusually formatted code could be missed** (false negative) or a
  `WHERE`/`GROUP BY` belonging to a different, adjacent statement within
  that same 800-char window could hide a real violation (false negative)
  or spuriously clear one (impossible in practice here — clearing
  requires the window to ALSO contain an Iceberg marker, which a same-file
  unrelated statement housing a stray "WHERE" would not need to fake to
  slip past, but is a theoretical gap worth naming).
- **Comments/docstrings that merely discuss the rule** (e.g. this file's
  own docstring, or `ops/g4/g4_test.py`'s explanatory comment) can in
  principle contain the words "count(" and "bronze." close enough together
  to trip the heuristic. None do today (verified by running this script
  over the repo); if a future doc comment does, the fix is to reword the
  comment, not to weaken the check.
- **Dynamically constructed SQL where `count(` and the Iceberg marker are
  assembled from variables with no literal text in common** (e.g. a table
  name passed through several layers of indirection before ever appearing
  next to `count(`) is invisible to a text-based lint by construction. The
  sanctioned mitigation for that class is a shared, safe-by-default helper
  — out of scope for this script, tracked in
  `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` R11 as a possible follow-up.

# Directories scanned

`ops/`, `dagster/`, `rust/crates/` (`*.py`, `*.rs`, `*.ts`, `*.tsx`),
excluding `target/`, `node_modules/`, `.next/`, and any path containing
`/fmtcheck/` (an untracked worktree swept in by `tsconfig.json`, not real
source — see the task brief). `demo/` is intentionally excluded: it is
read-only for this build and does not define any Iceberg-backed count
query today (verified).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

SCAN_ROOTS = ["ops", "dagster", "rust/crates"]
SCAN_EXTS = {".py", ".rs", ".ts", ".tsx"}
EXCLUDE_SEGMENTS = {"target", "node_modules", ".next", "fmtcheck"}

# This lint script's own path — it is allowed to talk *about* the pattern
# in prose without tripping over itself.
SELF_PATH = Path(__file__).resolve()

COUNT_RE = re.compile(r"(?<![A-Za-z0-9_])count\s*\(", re.IGNORECASE)
SELECT_RE = re.compile(r"\bSELECT\b", re.IGNORECASE)
WHERE_RE = re.compile(r"\bWHERE\b", re.IGNORECASE)
GROUP_BY_RE = re.compile(r"\bGROUP\s+BY\b", re.IGNORECASE)
FORMAT_RE = re.compile(r"\bFORMAT\b", re.IGNORECASE)

# Any of these appearing in the statement window means the target is an
# Iceberg/Bronze catalog table (the dangerous case), not the MergeTree
# `bronze_meta.*` registry (safe) or any other engine.
ICEBERG_MARKER_RE = re.compile(r"icecat|DataLakeCatalog|allow_database_iceberg|bronze\.", re.IGNORECASE)

WINDOW = 800


def iter_source_files() -> list[Path]:
    files: list[Path] = []
    for root_name in SCAN_ROOTS:
        root = REPO_ROOT / root_name
        if not root.exists():
            continue
        for path in root.rglob("*"):
            if not path.is_file() or path.suffix not in SCAN_EXTS:
                continue
            if any(part in EXCLUDE_SEGMENTS for part in path.parts):
                continue
            files.append(path)
    return sorted(files)


def statement_window(text: str, count_start: int, count_end: int) -> tuple[int, int]:
    """Approximate the enclosing SQL statement around a `count(` match."""
    left_bound = max(0, count_start - WINDOW)
    right_bound = min(len(text), count_end + WINDOW)

    # Left edge: nearest preceding SELECT within the window, else the raw
    # window bound.
    left_slice = text[left_bound:count_start]
    select_matches = list(SELECT_RE.finditer(left_slice))
    if select_matches:
        left = left_bound + select_matches[-1].start()
    else:
        left = left_bound

    # Right edge: nearest following statement terminator (`;`, FORMAT, or a
    # blank line), else the raw window bound.
    right_slice = text[count_end:right_bound]
    candidates = []
    semi = right_slice.find(";")
    if semi != -1:
        candidates.append(semi)
    fmt = FORMAT_RE.search(right_slice)
    if fmt:
        candidates.append(fmt.start())
    blank = right_slice.find("\n\n")
    if blank != -1:
        candidates.append(blank)
    right = count_end + (min(candidates) if candidates else len(right_slice))

    return left, right


def find_violations(path: Path) -> list[tuple[int, str]]:
    text = path.read_text(encoding="utf-8", errors="replace")
    violations: list[tuple[int, str]] = []
    for m in COUNT_RE.finditer(text):
        left, right = statement_window(text, m.start(), m.end())
        window = text[left:right]
        if not ICEBERG_MARKER_RE.search(window):
            continue
        if WHERE_RE.search(window) or GROUP_BY_RE.search(window):
            continue
        line_no = text.count("\n", 0, m.start()) + 1
        snippet = " ".join(window.split())
        if len(snippet) > 160:
            snippet = snippet[:160] + "..."
        violations.append((line_no, snippet))
    return violations


def main() -> int:
    had_violation = False
    for path in iter_source_files():
        if path.resolve() == SELF_PATH:
            continue
        for line_no, snippet in find_violations(path):
            had_violation = True
            rel = path.relative_to(REPO_ROOT)
            print(f"{rel}:{line_no}: bare count() against an Iceberg/Bronze table (R11)")
            print(f"    {snippet}")
            print(
                "    fix: add a WHERE predicate (e.g. `WHERE 1` / `WHERE id > 0`) "
                "or a GROUP BY — see docs/plans/P5-RESULT.md"
            )
    if had_violation:
        print()
        print(
            "R11 guard failed: bare count()/count(*)/count(<col>) against a "
            "Bronze Iceberg table takes ClickHouse 26.3's metadata-only fast "
            "path and silently overcounts (measured 8 where 6 is correct; "
            "see docs/plans/P5-RESULT.md). Add a WHERE or GROUP BY."
        )
        return 1
    print("R11 guard: no bare count() against a Bronze Iceberg table found.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
