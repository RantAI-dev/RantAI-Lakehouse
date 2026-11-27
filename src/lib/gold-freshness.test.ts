import { describe, expect, it } from "bun:test"
import { freshnessLine, publicationLabel, syncModeLabel } from "./gold-freshness"
import type { GoldExportResult, GoldPublication } from "@/services/contracts/gold"

// DATA-6 review R6: these three fields ship with the console. An optional
// property adds `undefined` to the indexed type, so the assertions below
// flip to `false` and typecheck fails if any of them becomes optional
// again. This is the regression guard the old contract had no test for.
type RequiredKey<T, K extends keyof T> = undefined extends T[K] ? false : true

const basePublication: GoldPublication = {
  mart: "sales_rollup",
  enabled: true,
  updatedAt: "2026-10-14T10:00:00Z",
  lastChangedAt: "2026-10-14T09:30:00Z",
  lastExportedAt: "2026-10-14T10:00:00Z",
  changedSinceExport: false,
  lastFailure: null,
  canEdit: true,
  trim: null,
  latestCopy: null,
  syncMode: "on_update",
  schedule: null,
  timeZone: null,
  nextSyncAt: null,
}

describe("gold-freshness", () => {
  it("reports Never published when lastExportedAt is null", () => {
    const pub: GoldPublication = {
      ...basePublication,
      lastExportedAt: null,
      changedSinceExport: null,
    }
    expect(freshnessLine(pub)).toBe("Never published")
    expect(publicationLabel(pub)).toBe("On · Never published")
  })

  it("reports Not measured when changedSinceExport is null", () => {
    const pub: GoldPublication = {
      ...basePublication,
      changedSinceExport: null,
    }
    expect(freshnessLine(pub)).toBe("Not measured")
    expect(publicationLabel(pub)).toBe("On · Not measured")
  })

  // QA LH-DATA-1-01 (DATA-49): a failed newest run is the first line, even when
  // the last successful publish matches the table (which would read "Up to date").
  it("reports Last publish failed when the newest run failed, even if the last success matches the table", () => {
    const pub: GoldPublication = {
      ...basePublication,
      changedSinceExport: false,
      lastFailure: { failedAt: "2026-10-14T11:00:00Z", reason: "The catalog request failed. Reference: abc" },
    }
    expect(freshnessLine(pub)).toBe("Last publish failed")
    expect(publicationLabel(pub)).toBe("On · Last publish failed")
  })

  it("reports Last publish failed for a mart whose only run failed", () => {
    const pub: GoldPublication = {
      ...basePublication,
      lastExportedAt: null,
      changedSinceExport: null,
      lastFailure: { failedAt: null, reason: "The publish failed. No reason was recorded." },
    }
    expect(freshnessLine(pub)).toBe("Last publish failed")
  })

  it("keeps the four other lines when lastFailure is null", () => {
    expect(freshnessLine({ ...basePublication, lastExportedAt: null, changedSinceExport: null })).toBe("Never published")
    expect(freshnessLine({ ...basePublication, changedSinceExport: null })).toBe("Not measured")
    expect(freshnessLine({ ...basePublication, changedSinceExport: true })).toBe("Out of date")
    expect(freshnessLine({ ...basePublication, changedSinceExport: false })).toBe("Up to date")
  })

  it("reports Up to date when changedSinceExport is false", () => {
    const pub: GoldPublication = {
      ...basePublication,
      changedSinceExport: false,
    }
    expect(freshnessLine(pub)).toBe("Up to date")
    expect(publicationLabel(pub)).toBe("On · Up to date")
  })

  it("reports Out of date when changedSinceExport is true", () => {
    const pub: GoldPublication = {
      ...basePublication,
      changedSinceExport: true,
    }
    expect(freshnessLine(pub)).toBe("Out of date")
    expect(publicationLabel(pub)).toBe("On · Out of date")
  })

  it("reports Off when enabled is false regardless of freshness", () => {
    const pub: GoldPublication = {
      ...basePublication,
      enabled: false,
      changedSinceExport: false,
    }
    expect(publicationLabel(pub)).toBe("Off")
  })
})

describe("the DATA-6 contract fields are required", () => {
  it("does not allow an older API to omit the latest-copy fields", () => {
    const publicationLatestCopy: RequiredKey<GoldPublication, "latestCopy"> = true
    const resultStamp: RequiredKey<GoldExportResult, "copyExportedAt"> = true
    const resultLatestCopy: RequiredKey<GoldExportResult, "latestCopy"> = true
    expect(publicationLatestCopy).toBe(true)
    expect(resultStamp).toBe(true)
    expect(resultLatestCopy).toBe(true)
  })

  // DATA-66: the one label map the card and the All published marts page
  // share; an unknown mode is shown as sent, never as a guessed default.
  it("labels each sync mode, and shows an unknown one as the API sent it", () => {
    expect(syncModeLabel("on_update")).toBe("On every update")
    expect(syncModeLabel("scheduled")).toBe("On a schedule")
    expect(syncModeLabel("manual")).toBe("Manual only")
    expect(syncModeLabel("eventually")).toBe("eventually")
    expect(syncModeLabel(null)).toBe("On every update")
  })
})
