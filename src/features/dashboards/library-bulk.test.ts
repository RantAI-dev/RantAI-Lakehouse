// BI-61 T11 with BI-59 T5: moving ticked rows to the trash forgets the sidebar's remembered
// Dashboards row when it is one of them, as the single delete does, so the row never opens
// something in the trash.
import { afterEach, describe, expect, it, spyOn } from "bun:test"
import { dashboardReportService, dashboardService } from "@/services"
import { readRow, rememberRow } from "./dashboards-row"
import { trashItems } from "./library-bulk"
import type { LibraryItem } from "./library-items"

const item = (kind: "dashboard" | "report", id: string) =>
  ({ key: `${kind}:${id}`, kind, id, name: id, folderId: "", owner: "", updatedAt: "", count: 0, builtin: false, publicLink: false }) as unknown as LibraryItem

const spies: { mockRestore: () => void }[] = []
function stubDeletes(failBoard?: string) {
  spies.push(spyOn(dashboardService, "deleteBoard").mockImplementation((async (id: string) => {
    if (id === failBoard) throw new Error("not allowed")
  }) as unknown as typeof dashboardService.deleteBoard))
  spies.push(spyOn(dashboardReportService, "deleteReport").mockImplementation((async () => {}) as unknown as typeof dashboardReportService.deleteReport))
}

afterEach(() => {
  for (const s of spies.splice(0)) s.mockRestore()
  window.localStorage.clear()
})

describe("trashItems and the remembered Dashboards row", () => {
  it("forgets a remembered dashboard that is moved to the trash with others", async () => {
    stubDeletes()
    rememberRow("dashboard", "b1", "Revenue")
    const out = await trashItems([item("report", "r9"), item("dashboard", "b1")])
    expect(out.done).toBe(2)
    expect(readRow()).toBeNull()
  })

  it("forgets a remembered report that is moved to the trash", async () => {
    stubDeletes()
    rememberRow("report", "r1", "Q3 review")
    await trashItems([item("report", "r1")])
    expect(readRow()).toBeNull()
  })

  it("keeps the row when what it remembers is not among the deleted", async () => {
    stubDeletes()
    rememberRow("dashboard", "b1", "Revenue")
    // A report with the dashboard's id is another thing.
    await trashItems([item("dashboard", "b2"), item("report", "b1")])
    expect(readRow()?.id).toBe("b1")
  })

  it("keeps the row when its dashboard's delete is refused", async () => {
    stubDeletes("b1")
    rememberRow("dashboard", "b1", "Revenue")
    const out = await trashItems([item("dashboard", "b1")])
    expect(out.refused).toHaveLength(1)
    expect(readRow()?.id).toBe("b1")
  })
})
