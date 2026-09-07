#!/usr/bin/env python3
"""Fail if a `dispar_orchestrate` module imports a name its sibling does not define.

Why this exists: unscheduling the Gold export job commented out
`gold_export_schedule` in `gold_export.py` but left `definitions.py`
importing it. That is an ImportError at code-location load time, which takes
down the ENTIRE Dagster code location — no jobs register at all, so
`GET /api/pipelines` silently falls back to the mock list and the G3a
acceptance test fails with a message ("'bronze_ingest_job' not in
GET /api/pipelines") that points nowhere near the actual cause.

`python -m py_compile`, which CI already ran, cannot catch this: it checks
each file's syntax in isolation and never resolves imports. Actually
importing the package would catch it, but that needs `dagster` installed —
a heavy dependency for a lint job. This is the cheap middle ground: parse
the package with `ast` and check every `from dispar_orchestrate.X import Y`
against the top-level names X actually defines.

Deliberately limited. It only resolves FIRST-PARTY intra-package imports;
third-party ones (`from dagster import ...`) are out of scope, since
checking those means importing them. It also only sees module-level
definitions, so a name injected at runtime (globals() manipulation,
conditional definition inside `if`/`try`) would read as missing — the
package does not do that today, and doing it would be worth a second look
anyway.
"""

from __future__ import annotations

import ast
import pathlib
import sys

PACKAGE = "dispar_orchestrate"


def toplevel_names(tree: ast.Module) -> set[str]:
    """Names bound at module level — defs, classes, assignments, imports."""
    names: set[str] = set()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            names.add(node.name)
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    names.add(target.id)
        elif isinstance(node, ast.AnnAssign):
            if isinstance(node.target, ast.Name):
                names.add(node.target.id)
        elif isinstance(node, (ast.Import, ast.ImportFrom)):
            # A re-exported name is legitimately importable from here.
            for alias in node.names:
                names.add(alias.asname or alias.name.split(".")[0])
    return names


def main() -> int:
    root = pathlib.Path(__file__).resolve().parents[2] / "dagster" / PACKAGE
    if not root.is_dir():
        print(f"{__file__}: no {root} — nothing to check", file=sys.stderr)
        return 0

    trees = {p: ast.parse(p.read_text(), filename=str(p)) for p in sorted(root.glob("*.py"))}
    defined = {f"{PACKAGE}.{p.stem}": toplevel_names(t) for p, t in trees.items()}

    problems: list[str] = []
    for path, tree in trees.items():
        for node in ast.walk(tree):
            if not isinstance(node, ast.ImportFrom) or node.module not in defined:
                continue
            for alias in node.names:
                if alias.name == "*":
                    continue
                if alias.name not in defined[node.module]:
                    problems.append(
                        f"{path.relative_to(root.parents[1])}:{node.lineno}: "
                        f"imports {alias.name!r} from {node.module}, "
                        f"which does not define it"
                    )

    if problems:
        print("Unresolvable intra-package imports:\n", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        print(
            "\nThis would be an ImportError at Dagster code-location load, "
            "taking down every job in the location.",
            file=sys.stderr,
        )
        return 1

    print(f"OK: all intra-package {PACKAGE} imports resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
