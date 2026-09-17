"""Python port of `rust/crates/lakehouse-api/src/transform_grammar.rs`'s
injection-safe transform vocabulary (WS4 item E2, grand plan §6).
Re-validates every transform string BEFORE `authored_factory.py` ever
builds SQL from it -- defense in depth against a hypothetical future
Postgres row written by an API version with a looser grammar than the one
running today.

This is a port, not a redesign: same five verbs (`dedupe`, `filter`,
`rename`, `cast`, `select`), same `ALLOWED_CAST_TYPES`/
`ALLOWED_FILTER_OPERATORS` values, same refusal classes (stacked
statements, subqueries, function calls, SQL comments, disallowed
operators/casts, invalid identifiers, and a raw `'` anywhere in a filter
literal body -- banned outright, never escaped, which is what defeats the
backslash-quote bypass: `\\' OR '1'='1'` contains a raw `'` in its literal
body and is rejected before it ever reaches a renderer). Kept in lockstep
with `transform_grammar.rs` via `transform_test_vectors.json`, the shared
accept/reject vector file both `test_authored_transforms.py` (this
module's test) and `transform_grammar.rs`'s own `mod tests` load, so a
one-sided change to either grammar fails a test immediately instead of
silently drifting.

Every [`TransformError`] message names the offending FIELD, never the
caller's payload -- the payload is exactly the untrusted text this module
exists to keep out of an error message that might itself be logged or
echoed back to a caller.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Union

from dispar_orchestrate.bronze_catalog import _sql_string_literal

# The only `cast(col,type)` targets accepted -- `ClickHouse` type names a
# dedicated Bronze/Silver transform legitimately needs, nothing else (no
# `AggregateFunction`, no parametrized/nested types, which would reopen an
# injection surface through the type-name slot itself). Mirrors
# transform_grammar.rs's ALLOWED_CAST_TYPES verbatim.
ALLOWED_CAST_TYPES: tuple[str, ...] = (
    "String", "Int32", "Int64", "Float64", "Boolean", "Date", "DateTime", "UUID",
)

# `filter(expr)`'s only permitted comparison operators, longest first so a
# naive substring search never mis-splits `<=`/`>=`/`!=` on their leading
# `<`/`>`/`!` character. No `<>` (use `!=`), no `LIKE`/`IN`/boolean
# connectives -- those would reopen the subquery/function-call injection
# surface the grammar exists to close. Mirrors
# transform_grammar.rs's ALLOWED_FILTER_OPERATORS (there ordered
# `!=, <=, >=, =, <, >`; the two extra single-character operators are
# tried last here for the same reason).
ALLOWED_FILTER_OPERATORS: tuple[str, ...] = ("!=", "<=", ">=", "=", "<", ">")


class TransformError(ValueError):
    """Raised for any transform string outside the fixed grammar. The
    message names the offending field only -- see the module docstring."""


def _is_valid_identifier(name: str) -> bool:
    """Mirrors `Ident::new` in `rust/crates/lakehouse-core/src/ident.rs`:
    non-empty, ASCII alphanumeric plus `_`, not starting with a digit."""
    if not name:
        return False
    if name[0].isdigit():
        return False
    return all(c.isascii() and (c.isalnum() or c == "_") for c in name)


def _parse_ident(raw: str) -> str:
    raw = raw.strip()
    if not _is_valid_identifier(raw):
        raise TransformError("invalid identifier")
    return raw


@dataclass(frozen=True)
class Dedupe:
    key: str


@dataclass(frozen=True)
class Filter:
    column: str
    operator: str
    literal_body: str


@dataclass(frozen=True)
class Rename:
    from_col: str
    to_col: str


@dataclass(frozen=True)
class Cast:
    column: str
    target_type: str


@dataclass(frozen=True)
class Select:
    columns: tuple[str, ...]


Transform = Union[Dedupe, Filter, Rename, Cast, Select]


def parse_transform(text: str) -> Transform:
    """Parse one transform-vocabulary string. See the module docstring.

    Raises `TransformError` on any string outside the fixed grammar -- an
    unknown verb, a malformed argument list, an identifier that fails
    `_is_valid_identifier`, a filter expression outside
    `<ident> <op> '<literal>'` with `op` in `ALLOWED_FILTER_OPERATORS`, or
    a cast type outside `ALLOWED_CAST_TYPES`.
    """
    text = text.strip()
    if "(" not in text or not text.endswith(")"):
        raise TransformError("malformed transform")
    verb, _, rest = text.partition("(")
    args = rest[:-1]

    if verb == "dedupe":
        return Dedupe(key=_parse_ident(args))
    if verb == "filter":
        return _parse_filter(args)
    if verb == "rename":
        if "," not in args:
            raise TransformError("malformed transform")
        a, b = args.split(",", 1)
        return Rename(from_col=_parse_ident(a), to_col=_parse_ident(b))
    if verb == "cast":
        if "," not in args:
            raise TransformError("malformed transform")
        col, ty = args.split(",", 1)
        ty = ty.strip()
        if ty not in ALLOWED_CAST_TYPES:
            raise TransformError("cast type not allowed")
        return Cast(column=_parse_ident(col), target_type=ty)
    if verb == "select":
        columns = tuple(_parse_ident(c) for c in args.split(","))
        if not columns:
            raise TransformError("malformed transform")
        return Select(columns=columns)
    raise TransformError("unknown transform verb")


def _parse_filter(expr: str) -> Filter:
    """`filter(expr)`'s grammar: EXACTLY `<identifier> <op> '<literal>'` --
    one column, one operator from `ALLOWED_FILTER_OPERATORS`, one
    single-quoted literal. No boolean connectives, no subqueries, no
    function calls, no comments -- mirrors
    transform_grammar.rs's `parse_filter` exactly, including the ban on a
    raw `'` anywhere in the literal body (rather than escaping it), which
    is what defeats the classic backslash-quote bypass: a payload like
    `\\' OR '1'='1'` contains a raw `'` in its literal body and is
    rejected here before it ever reaches a renderer, so the bypass never
    gets the chance to become a tautology.
    """
    expr = expr.strip()
    for op in ALLOWED_FILTER_OPERATORS:
        if op not in expr:
            continue
        col, _, val = expr.partition(op)
        col = col.strip()
        val = val.strip()
        if not (val.startswith("'") and val.endswith("'") and len(val) >= 2):
            continue
        literal_body = val[1:-1]
        # Reject anything a single-quoted literal has no business
        # containing: another quote (would need doubling, which this
        # minimal grammar does not support -- reject rather than guess), a
        # semicolon, parentheses (blocks function-call/subquery injection
        # outright, since a bare literal never needs them), or SQL comment
        # markers. This ban is absolute, not an escape: transform_grammar.rs
        # takes the same stance (ident.rs's SqlLiteral only ever gets a
        # literal_body this check has already passed), so a raw quote can
        # never even reach either language's literal-rendering helper.
        if any(c in literal_body for c in ("'", ";", "(", ")")) or "--" in literal_body or "/*" in literal_body:
            raise TransformError("invalid filter expression")
        column = _parse_ident(col)
        return Filter(column=column, operator=op, literal_body=literal_body)
    raise TransformError("invalid filter expression")


def render_clickhouse(transform: Transform) -> str:
    """Render as a `ClickHouse` SQL fragment. Every dynamic piece goes
    through `_sql_string_literal` (reused from `bronze_catalog.py` per
    AGENTS.md rule 4) for a filter literal, or a value already validated
    by `_parse_ident` for an identifier position -- never raw
    interpolation of caller text.

    Note: this renders literals through `_sql_string_literal`'s
    backslash-escaping encoding, which is NOT byte-identical to
    `transform_grammar.rs`'s `SqlLiteral` (which doubles both backslashes
    and quotes) -- see the module docstring for why that divergence is
    deliberate and safe.
    """
    if isinstance(transform, Dedupe):
        return f"ORDER BY {transform.key}"
    if isinstance(transform, Filter):
        return f"{transform.column} {transform.operator} {_sql_string_literal(transform.literal_body)}"
    if isinstance(transform, Rename):
        return f"{transform.from_col} AS {transform.to_col}"
    if isinstance(transform, Cast):
        return f"CAST({transform.column} AS {transform.target_type})"
    if isinstance(transform, Select):
        return ", ".join(transform.columns)
    raise TransformError(f"unrenderable transform: {type(transform)!r}")  # unreachable -- Transform is a closed union
