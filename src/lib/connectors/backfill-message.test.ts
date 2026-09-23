import { describe, expect, it } from "bun:test"
import { backfillTriggerMessage } from "@/lib/connectors/backfill-message"

describe("backfillTriggerMessage", () => {
  it("names Debezium's own snapshot.mode=initial for every supported CDC driver, not just postgres", () => {
    for (const driver of ["postgres", "mysql", "mssql", "mongodb"]) {
      const msg = backfillTriggerMessage(driver)
      expect(msg).toContain("snapshot.mode=initial")
      expect(msg).toContain(driver)
      expect(msg).not.toContain("signal table")
    }
  })

  it("cites ADR 0008 rather than an untracked plan document", () => {
    const msg = backfillTriggerMessage("postgres")
    expect(msg).toContain("docs/adr/0008-initial-snapshot-backfill.md")
  })

  it("says Oracle CDC is unsupported instead of implying the generic Debezium story applies to it", () => {
    const msg = backfillTriggerMessage("oracle")
    expect(msg).not.toContain("snapshot.mode=initial")
    expect(msg.toLowerCase()).toContain("not supported")
    expect(msg).toContain("docs/adr/0008-initial-snapshot-backfill.md")
  })

  it("is case-insensitive when recognizing the Oracle driver", () => {
    const msg = backfillTriggerMessage("Oracle")
    expect(msg.toLowerCase()).toContain("not supported")
  })
})
