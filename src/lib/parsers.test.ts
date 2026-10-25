import { describe, expect, it } from "bun:test"
import { z } from "zod"

import { getFiltersStateParser } from "@/lib/parsers"

describe("the parsers module", () => {
  it("turns Zod's JIT probe off, so a parse draws no script-src violation", () => {
    // SEC-47 review round 3, BLOCKER 2: the module sets `z.config({ jitless:
    // true })` at load; without it the first parse probes `new Function("")`
    // and strict CSP reports the caught probe on every page.
    expect(z.config().jitless).toBe(true)
  })

  it("still parses a filter list", () => {
    const parser = getFiltersStateParser<{ status: string }>()
    const filter = {
      id: "status",
      value: "ok",
      variant: "text",
      operator: "iLike",
      filterId: "f1",
    } as const
    expect(parser.parse(JSON.stringify([filter]))).toEqual([filter])
  })
})
