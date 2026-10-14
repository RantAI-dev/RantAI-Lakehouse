import { describe, expect, it } from "bun:test"

import { toolsFromCaps } from "./capabilities"

describe("toolsFromCaps", () => {
  it("offers ask_user with the Query Data capability in both modes", () => {
    expect(toolsFromCaps(new Set(["data"]), "ask")).toContain("ask_user")
    expect(toolsFromCaps(new Set(["data"]), "build")).toContain("ask_user")
  })

  it("does not offer ask_user when Query Data is off", () => {
    expect(toolsFromCaps(new Set(["dashboard"]), "build")).not.toContain("ask_user")
  })
})
