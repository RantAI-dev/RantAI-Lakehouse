import { describe, expect, it } from "bun:test"
import { devOrigins } from "./dev-origins"

// BI-1 QA fix: the PDF renderer opens the dev console by 127.0.0.1.
describe("devOrigins", () => {
  it("always allows 127.0.0.1 so the PDF renderer's page can hydrate under next dev", () => {
    expect(devOrigins(undefined)).toEqual(["127.0.0.1"])
    expect(devOrigins("")).toEqual(["127.0.0.1"])
  })
  it("adds the configured hosts, trimmed, without duplicates or blanks", () => {
    expect(devOrigins(" 10.0.0.5, *.lan.test,,127.0.0.1 ")).toEqual(["127.0.0.1", "10.0.0.5", "*.lan.test"])
  })
})
