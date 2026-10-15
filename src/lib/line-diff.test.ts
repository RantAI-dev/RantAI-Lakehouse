import { describe, expect, it } from "bun:test"
import { lineDiff } from "./line-diff"

describe("lineDiff", () => {
  it("marks every line as same when the two texts are identical", () => {
    expect(lineDiff("SELECT 1\nFROM t", "SELECT 1\nFROM t")).toStrictEqual([
      { kind: "same", text: "SELECT 1" },
      { kind: "same", text: "FROM t" },
    ])
  })

  it("shows one changed line as a removed line followed by an added line", () => {
    expect(lineDiff("SELECT a\nFROM t", "SELECT b\nFROM t")).toStrictEqual([
      { kind: "removed", text: "SELECT a" },
      { kind: "added", text: "SELECT b" },
      { kind: "same", text: "FROM t" },
    ])
  })

  it("marks a line that exists only in the proposal as added", () => {
    expect(lineDiff("SELECT 1", "SELECT 1\nWHERE x = 1")).toStrictEqual([
      { kind: "same", text: "SELECT 1" },
      { kind: "added", text: "WHERE x = 1" },
    ])
  })

  it("marks a line that exists only in the editor as removed", () => {
    expect(lineDiff("SELECT 1\nWHERE x = 1", "SELECT 1")).toStrictEqual([
      { kind: "same", text: "SELECT 1" },
      { kind: "removed", text: "WHERE x = 1" },
    ])
  })

  it("treats a text against the empty text as all added, and the reverse as all removed", () => {
    expect(lineDiff("", "SELECT 1\nFROM t")).toStrictEqual([
      { kind: "added", text: "SELECT 1" },
      { kind: "added", text: "FROM t" },
    ])
    expect(lineDiff("SELECT 1\nFROM t", "")).toStrictEqual([
      { kind: "removed", text: "SELECT 1" },
      { kind: "removed", text: "FROM t" },
    ])
  })

  it("counts lines that differ only in trailing spaces as changed", () => {
    expect(lineDiff("SELECT 1  ", "SELECT 1")).toStrictEqual([
      { kind: "removed", text: "SELECT 1  " },
      { kind: "added", text: "SELECT 1" },
    ])
  })
})
