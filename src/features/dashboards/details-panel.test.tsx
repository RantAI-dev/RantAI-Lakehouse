import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { DetailsPanel } from "./details-panel"

afterEach(() => {
  cleanup()
  mock.restore()
})

const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } })

// BI-61 review round 1, SHOULD-FIX 11: the dashboard's last edit is said in Details,
// "Edited 5h ago by <the newest version's author>", with the full date as its tooltip.
describe("DetailsPanel, the last edit", () => {
  it("says when the dashboard was last edited and by whom, with the full date on hover", async () => {
    spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL) => {
      if (String(input).includes("/api/dashboard/versions")) {
        return Promise.resolve(json({ keep: 20, versions: [{ version: 3, author: "Rina Putri", at: "2026-11-14 07:00:00", summary: "Settings", charts: 3 }] }))
      }
      return Promise.resolve(json({}))
    }) as unknown as typeof fetch)
    const updatedAt = new Date(Date.now() - 5 * 3_600_000).toISOString()
    render(
      <DetailsPanel
        board="b_1" listed={{ id: "b_1", name: "Sales", updatedAt }} canManage={false} canShare={false} canWrite={false} canDownload={false}
        dataFrom="Data from 12:47" autoRefresh="Off" cacheRule={null}
        onShare={() => {}} onFreshness={() => {}} onSubscriptions={() => {}} onHistory={() => {}}
      />,
    )
    const value = await screen.findByText("5h ago by Rina Putri")
    expect(screen.getByText("Edited")).toBeTruthy()
    expect(value.getAttribute("title")).toMatch(/2026/)
  })
})
