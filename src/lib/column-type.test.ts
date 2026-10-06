import { describe, expect, it } from "bun:test"
import { typeFamily } from "./column-type"

describe("typeFamily", () => {
  it("reads the engine's text types", () => {
    for (const t of ["String", "FixedString(16)", "UUID", "Enum8('a' = 1, 'b' = 2)", "Enum16('x' = 1)", "Enum('y' = 1)"]) {
      expect(typeFamily(t)).toBe("text")
    }
  })

  it("reads the engine's number types", () => {
    for (const t of [
      "Int8", "Int16", "Int32", "Int64", "Int128", "Int256", "UInt8", "UInt64", "UInt256",
      "Float32", "Float64", "BFloat16", "Decimal(12, 2)", "Decimal32(4)", "Decimal128(10)",
    ]) {
      expect(typeFamily(t)).toBe("number")
    }
  })

  it("reads the engine's date and time types", () => {
    for (const t of ["Date", "Date32", "DateTime", "DateTime('Asia/Jakarta')", "DateTime64(3)", "DateTime64(3, 'UTC')"]) {
      expect(typeFamily(t)).toBe("time")
    }
  })

  it("reads the engine's boolean, and its nested types", () => {
    expect(typeFamily("Bool")).toBe("boolean")
    for (const t of ["Array(String)", "Map(String, Int64)", "Tuple(Int32, String)", "Nested(a String, b Int32)", "JSON"]) {
      expect(typeFamily(t)).toBe("nested")
    }
  })

  it("reads Iceberg's types", () => {
    expect(typeFamily("string")).toBe("text")
    expect(typeFamily("uuid")).toBe("text")
    for (const t of ["int", "long", "float", "double", "decimal(12, 2)"]) expect(typeFamily(t)).toBe("number")
    for (const t of ["date", "time", "timestamp", "timestamptz"]) expect(typeFamily(t)).toBe("time")
    expect(typeFamily("boolean")).toBe("boolean")
    for (const t of ["list<string>", "map<string, long>", "struct<a: int, b: string>"]) {
      expect(typeFamily(t)).toBe("nested")
    }
  })

  it("looks through Nullable and LowCardinality, in any nesting and any case", () => {
    expect(typeFamily("Nullable(String)")).toBe("text")
    expect(typeFamily("Nullable(Decimal(12, 2))")).toBe("number")
    expect(typeFamily("LowCardinality(String)")).toBe("text")
    expect(typeFamily("LowCardinality(Nullable(String))")).toBe("text")
    expect(typeFamily("Nullable(DateTime64(3, 'UTC'))")).toBe("time")
    expect(typeFamily("  nullable( int64 )  ")).toBe("number")
  })

  it("calls a nested type nested whatever it holds, wrapped or not", () => {
    expect(typeFamily("Array(Nullable(String))")).toBe("nested")
    expect(typeFamily("Nullable(Tuple(Int32, String))")).toBe("nested")
    expect(typeFamily("list<struct<a: int>>")).toBe("nested")
  })

  it("says other for a type it does not know, instead of guessing", () => {
    for (const t of ["binary", "fixed[16]", "IPv4", "Point", "Interval", "Variant(String, Int64)", "timestamp_foo", "", "   "]) {
      expect(typeFamily(t)).toBe("other")
    }
  })

  it("does not take a longer name for a type it starts with", () => {
    expect(typeFamily("integer_code")).toBe("other")
    expect(typeFamily("datetime_utc")).toBe("other")
    expect(typeFamily("stringly")).toBe("other")
  })
})
