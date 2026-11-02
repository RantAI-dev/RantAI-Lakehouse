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

  it("no longer sends anyone to bring up a Debezium compose service", () => {
    const msg = backfillTriggerMessage("postgres")
    expect(msg).not.toContain("compose service")
    expect(msg).toContain("docs/adr/0016-change-capture-per-connector-on-kafka-connect.md")
  })

  it("says Oracle change capture is unsupported instead of implying the generic story applies to it", () => {
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
