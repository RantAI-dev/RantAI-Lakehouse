// A dashboard's SQL source cannot hold a `{{name}}` placeholder yet (`QS-5`,
// decision 15): the server refuses it, and the page shows the server's
// sentence as it does for every other refusal. The console does not check
// the SQL itself. `WEB-7` T5 (D5): the statement comes from the editor in
// `sessionStorage`, and a page opened without it says so.

let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/query-studio/save-source",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: (href: string) => {
      pushed.push(href)
    },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { dashboardService, settingsService } from "@/services"
import { ServiceError } from "@/services/errors"
import { SaveSqlSourcePage } from "./save-sql-source-page"
import { readSaveSourceDraft, writeSaveSourceDraft } from "./save-draft"

afterEach(() => {
  cleanup()
  mock.restore()
  window.sessionStorage.clear()
  pushed = []
})

const REFUSAL =
  "Parameters in a dashboard's SQL source come later: give the value in the SQL for now."

describe("SaveSqlSourcePage", () => {
  it("sends the SQL with its placeholder as it is, and shows the sentence the server refuses it with", async () => {
    // The limits sentence reads whichever limits apply; a failure leaves it
    // without a number, which is the honest default and not this test's point.
    spyOn(settingsService, "getLimits").mockRejectedValue(
      new ServiceError("unavailable", "no limits")
    )
    writeSaveSourceDraft({ sql: "SELECT * FROM serving.t WHERE r = {{region}}" })
    const create = spyOn(dashboardService, "createSqlSource").mockRejectedValue(
      new ServiceError("invalid_request", REFUSAL)
    )
    render(<SaveSqlSourcePage />)

    fireEvent.click(await screen.findByRole("button", { name: "Save source" }))

    await waitFor(() => expect(create).toHaveBeenCalledTimes(1))
    expect(create.mock.calls[0]?.[0].sql).toBe("SELECT * FROM serving.t WHERE r = {{region}}")
    expect((await screen.findByText(REFUSAL)).textContent).toBe(REFUSAL)
    // A refused save keeps the draft: the statement is not lost.
    expect(readSaveSourceDraft()?.sql).toContain("{{region}}")
  })

  it("names the source from the statement and deletes the draft once it is saved", async () => {
    spyOn(settingsService, "getLimits").mockRejectedValue(
      new ServiceError("unavailable", "no limits")
    )
    writeSaveSourceDraft({ sql: "SELECT * FROM serving.orders" })
    const create = spyOn(dashboardService, "createSqlSource").mockResolvedValue({
      id: "s1",
      title: "SELECT * FROM serving.orders",
      sql: "SELECT * FROM serving.orders",
      columns: [],
      folderId: "",
      createdBy: "u1",
      updatedAt: "2026-11-11T10:00:00.000Z",
    })
    render(<SaveSqlSourcePage />)

    expect(((await screen.findByLabelText("Title")) as HTMLInputElement).value).toBe(
      "SELECT * FROM serving.orders"
    )
    fireEvent.click(screen.getByRole("button", { name: "Save source" }))

    await waitFor(() => expect(pushed).toEqual(["/query-studio"]))
    expect(create).toHaveBeenCalledTimes(1)
    expect(readSaveSourceDraft()).toBeNull()
  })

  it("says to open it from Query Studio when no statement was left for it", async () => {
    render(<SaveSqlSourcePage />)

    expect(await screen.findByText("Open this from Query Studio")).toBeDefined()
    // The repo's Button renders a link with `role="button"`, so the href is
    // read off the anchor itself.
    expect(screen.getByText("Open Query Studio").closest("a")?.getAttribute("href")).toBe(
      "/query-studio"
    )
  })
})
