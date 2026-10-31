import { describe, expect, it } from "bun:test"
import { version as packageVersion } from "../../../package.json"
import { CONSOLE_VERSION, versionMismatch } from "./console-version"

/**
 * The console's own version comes from `package.json` (REL-2 S1-T5), and
 * the banner only appears when the server runs another build.
 */
describe("the console's version", () => {
  it("is the version package.json carries", () => {
    expect(CONSOLE_VERSION).toBe(packageVersion)
    expect(CONSOLE_VERSION).toMatch(/^\d+\.\d+\.\d+/)
  })

  it("reports a mismatch only when the server runs another build", () => {
    expect(versionMismatch(CONSOLE_VERSION)).toBeNull()
    expect(versionMismatch("9.9.9")).toEqual({ server: "9.9.9", console: CONSOLE_VERSION })
  })

  it("says nothing when the server's version is unknown", () => {
    expect(versionMismatch(null)).toBeNull()
    expect(versionMismatch(undefined)).toBeNull()
    expect(versionMismatch("")).toBeNull()
  })
})
