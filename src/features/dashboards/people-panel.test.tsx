import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { MoveEffectNote } from "./folder-dialogs"
import { NOBODY_MANAGES_NOTE, PeoplePanel, RESTRICTION_NOTE } from "./people-panel"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

type Call = { url: string; method: string; body: unknown }

const ACCESS = {
  kind: "board",
  id: "b_1",
  yourLevel: "manage",
  restricted: false,
  rules: [{ kind: "user", id: "u1", name: "Dewi", email: "dewi@company.example", level: "view" }],
  inherited: [{ kind: "role", id: "r1", name: "Analyst", level: "view", folderId: "f1", folderName: "Finance" }],
  restrictedAbove: [] as { folderId: string; folderName: string }[],
}

function stub(access: unknown, status = 200): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    calls.push({ url, method, body: init?.body ? JSON.parse(String(init.body)) : undefined })
    if (url.startsWith("/api/dashboard/access")) return json(access, status)
    if (url.startsWith("/api/reports/recipients")) {
      return json({ items: [{ id: "u2", name: "Budi", email: "budi@company.example" }], roles: [{ id: "r2", name: "Viewer" }] })
    }
    return json({ ok: true })
  }) as unknown as typeof fetch
  return calls
}

describe("PeoplePanel (BI-19 part B1)", () => {
  it("shows who has access, what the folders above give, and the sentence about public links", async () => {
    stub(ACCESS)
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByText("Dewi")
    expect(screen.getByText(/Analyst \(role\): can view, from “Finance”/)).toBeTruthy()
    expect(screen.getByText(RESTRICTION_NOTE)).toBeTruthy()
  })

  it("adds a person from the picker and saves the whole list, with the restriction", async () => {
    const calls = stub(ACCESS)
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByText("Dewi")
    fireEvent.change(screen.getByLabelText("Add a person or role"), { target: { value: "bu" } })
    fireEvent.click(await screen.findByText("Budi"))
    fireEvent.click(screen.getByLabelText("Only the people listed"))
    fireEvent.click(screen.getByText("Save people"))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")!
    expect(put.body).toEqual({
      kind: "board",
      id: "b_1",
      restricted: true,
      rules: [{ userId: "u1", level: "view" }, { userId: "u2", level: "view" }],
    })
  })

  it("adds a role by its id, not as a person", async () => {
    const calls = stub(ACCESS)
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByText("Dewi")
    fireEvent.change(screen.getByLabelText("Add a person or role"), { target: { value: "vi" } })
    fireEvent.click(await screen.findByText("Viewer"))
    fireEvent.click(screen.getByText("Save people"))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")!
    expect((put.body as { rules: unknown[] }).rules).toContainEqual({ roleId: "r2", level: "view" })
  })

  it("changes a person to edit and saves that level (BI-19 part B2)", async () => {
    const calls = stub(ACCESS)
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByText("Dewi")
    fireEvent.change(screen.getByLabelText("Level of Dewi"), { target: { value: "edit" } })
    fireEvent.click(screen.getByText("Save people"))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")!
    expect((put.body as { rules: unknown[] }).rules).toEqual([{ userId: "u1", level: "edit" }])
  })

  it("offers no level above the caller's own (feature B6)", async () => {
    stub({ ...ACCESS, yourLevel: "edit" })
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByText("Dewi")
    const options = Array.from(screen.getByLabelText("Level of Dewi").querySelectorAll("option")).map((o) => o.getAttribute("value"))
    expect(options).toEqual(["view", "edit"])
  })

  it("says why when the caller does not manage the item", async () => {
    stub({ error: "invalid_request", message: "permission_denied: manage on this item" }, 403)
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByRole("alert")
  })

  it("says the folders above already limit it to the people listed", async () => {
    stub({ ...ACCESS, restrictedAbove: [{ folderId: "f1", folderName: "Finance" }] })
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByText(/Already limited to the people listed by the folder “Finance”/)
  })
})

describe("PeoplePanel warning (BI-19 B2 review note)", () => {
  it("warns when it is restricted and nobody listed can manage, and not when somebody can", async () => {
    stub({ ...ACCESS, restricted: true })
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByText("Dewi")
    expect(screen.getByText(NOBODY_MANAGES_NOTE)).toBeTruthy()
    cleanup()
    stub({ ...ACCESS, restricted: true, rules: [{ ...ACCESS.rules[0], level: "manage" }] })
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByText("Dewi")
    expect(screen.queryByText(NOBODY_MANAGES_NOTE)).toBeNull()
  })

  it("does not warn while the item is not restricted", async () => {
    stub(ACCESS)
    render(<PeoplePanel kind="board" id="b_1" />)
    await screen.findByText("Dewi")
    expect(screen.queryByText(NOBODY_MANAGES_NOTE)).toBeNull()
  })
})

describe("MoveEffectNote (feature B7)", () => {
  it("names who gains and who loses access, and what a restricted destination does to everyone else", () => {
    render(
      <MoveEffectNote
        effect={{
          gains: [{ kind: "user", id: "u1", name: "Dewi" }],
          loses: [{ kind: "role", id: "r1", name: "Analyst" }],
          startingPointLoses: true,
          startingPointGains: false,
        }}
      />,
    )
    expect(screen.getByText("Will gain access: Dewi.")).toBeTruthy()
    expect(screen.getByText("Will lose access: Analyst (role).")).toBeTruthy()
    expect(screen.getByText(/shows only the people listed/)).toBeTruthy()
  })

  it("says nobody changes when nothing does", () => {
    render(<MoveEffectNote effect={{ gains: [], loses: [], startingPointLoses: false, startingPointGains: false }} />)
    expect(screen.getByText("Nobody gains or loses access.")).toBeTruthy()
  })
})
