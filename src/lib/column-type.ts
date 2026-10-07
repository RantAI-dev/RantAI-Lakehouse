/**
 * Which kind of data a column's type stands for, so the Schema tab can give
 * each column one glyph. The catalog shows two spellings of a type: the
 * engine's (`Nullable(String)`, `Decimal(12, 2)`, `DateTime64(3)`) for a
 * Silver or Gold table and the registry, and Iceberg's (`string`,
 * `decimal(12, 2)`, `timestamptz`, `list<string>`) for a raw table. Both are
 * read here.
 *
 * A type this does not recognise is `"other"`, never a guess: a glyph that
 * says "text" for a column that is not text would be wrong, and a neutral
 * one is not.
 */
export type TypeFamily = "text" | "number" | "time" | "boolean" | "nested" | "other"

/** `Nullable(...)` and `LowCardinality(...)` change how a column is stored, not what it holds. */
const WRAPPERS = ["nullable(", "lowcardinality("]

function unwrapped(dataType: string): string {
  let t = dataType.trim().toLowerCase()
  for (;;) {
    const wrapper = WRAPPERS.find((w) => t.startsWith(w) && t.endsWith(")"))
    if (!wrapper) return t
    t = t.slice(wrapper.length, -1).trim()
  }
}

// Each pattern is anchored at both ends, so `int` is a number but `interval`
// and `point` are not, and `time` is a time but `timestamp_foo` is not.
const NESTED = /^(array|map|tuple|nested)\(.*\)$|^json(\(.*\))?$|^(list|map|struct)<.*>$/
const TEXT = /^(string|uuid|fixedstring\(.*\)|enum(8|16)?(\(.*\))?)$/
const NUMBER =
  /^(u?int(8|16|32|64|128|256)?|float(32|64)?|bfloat16|long|double|decimal(32|64|128|256)?(\(.*\))?)$/
const TIME = /^(date(32)?|datetime(64)?(\(.*\))?|time(64)?(\(.*\))?|timestamp(tz)?(_ns)?)$/
const BOOLEAN = /^(bool|boolean)$/

/**
 * The family of a column's type, in either spelling the catalog shows.
 * `Nullable(...)` and `LowCardinality(...)` are looked through; a nested
 * type is `"nested"` whatever it holds.
 */
export function typeFamily(dataType: string): TypeFamily {
  const t = unwrapped(dataType)
  if (NESTED.test(t)) return "nested"
  if (TEXT.test(t)) return "text"
  if (NUMBER.test(t)) return "number"
  if (TIME.test(t)) return "time"
  if (BOOLEAN.test(t)) return "boolean"
  return "other"
}
