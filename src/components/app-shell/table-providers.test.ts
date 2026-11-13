import { describe, expect, it } from "bun:test"
import { ServiceError } from "@/services/errors"
import { retryTableQuery } from "./table-providers"

describe("retryTableQuery", () => {
  // QA LH-DATA-12-09 F2 (WEB-11): the Catalog asked a 403 four times while the
  // page showed its skeleton; a refusal answers the same every time.
  it("a refusal is not asked again", () => {
    expect(retryTableQuery(0, new ServiceError("permission_denied", "x", 403))).toBe(false)
  })

  it("any other failure is asked three more times, as before", () => {
    const down = new ServiceError("unavailable", "down")
    for (const count of [0, 1, 2]) expect(retryTableQuery(count, down)).toBe(true)
    expect(retryTableQuery(3, down)).toBe(false)
    expect(retryTableQuery(0, new Error("network"))).toBe(true)
  })
})
