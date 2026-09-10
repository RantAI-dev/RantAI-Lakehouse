import { describe, expect, it } from "bun:test"
import { fmtMeasured } from "./measured"

describe("fmtMeasured", () => {
  it("renders an em dash for a value the backend did not measure", () => {
    expect(fmtMeasured(null)).toBe("—")
  })

  it("renders zero as zero, because a measured zero is not the same as unmeasured", () => {
    expect(fmtMeasured(0)).toBe("0")
  })

  it("applies the caller's formatter to a measured value", () => {
    expect(fmtMeasured(1536, (n) => `${n / 1024} KiB`)).toBe("1.5 KiB")
  })

  it("never calls the formatter for a null, so formatters need not be null-safe", () => {
    let calls = 0
    fmtMeasured(null, (n) => {
      calls += 1
      return String(n)
    })
    expect(calls).toBe(0)
  })
})
