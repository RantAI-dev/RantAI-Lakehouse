import { afterEach, describe, expect, it, mock } from "bun:test"

import { createBoard, deleteBoard, deleteChart, duplicateBoard, renameBoard, saveBoardLayout } from "./board-writes"

const originalFetch = global.fetch
afterEach(() => { global.fetch = originalFetch })

function answer(status: number, body: unknown) {
  global.fetch = mock(async () => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })) as unknown as typeof fetch
}

const REFUSED = { error: "permission_denied: dashboard:write" }
const SENTENCE = "This account does not have permission. Ask a workspace admin."

describe("a refused write is an error, not a success (BI-18 part C review fix (SHOULD-FIX) R19)", () => {
  const calls: [string, () => Promise<unknown>][] = [
    ["create", () => createBoard("New dashboard")],
    ["duplicate", () => duplicateBoard("b1")],
    ["rename", () => renameBoard("b1", "Renamed")],
    ["delete a dashboard", () => deleteBoard("b1")],
    ["delete a chart", () => deleteChart("c1")],
    ["save a layout", () => saveBoardLayout("b1", {})],
  ]
  for (const [name, call] of calls) {
    it(`${name} throws the server's sentence on a 403, without the code`, async () => {
      answer(403, REFUSED)
      await expect(call()).rejects.toThrow(SENTENCE)
    })
  }

  it("says the status when the refusal has no sentence", async () => {
    answer(500, {})
    await expect(deleteBoard("b1")).rejects.toThrow("The change was refused (500).")
  })

  it("returns the answer of a write that went through", async () => {
    answer(200, { board: { id: "b9" } })
    expect((await duplicateBoard("b1")).board?.id).toBe("b9")
    answer(200, { ok: true })
    await deleteChart("c1")
  })
})
