// plan D2: the audit page's person filter picks the principal from the
// trail's actors. This pins the picker's own contract — the options come
// from the actors route and are searched by name, a held id the list does
// not name is kept marked, and a failed read goes back to free text with
// the reason instead of an empty list that would read as "the trail has no
// people".
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { governanceService } from "@/services"
import type { AuditActor } from "@/services/contracts/governance"
import { AuditActorPicker, useAuditActors } from "./audit-actor-picker"

afterEach(() => {
  cleanup()
  mock.restore()
})

// cmdk scrolls the selected item into view; jsdom/happy-dom has no such method.
Element.prototype.scrollIntoView = Element.prototype.scrollIntoView ?? (() => {})

const ACTORS: AuditActor[] = [
  { actorId: "usr_admin", actor: "Platform Admin", actorKind: "user" },
  { actorId: "svc_loader", actor: "Ingest Loader", actorKind: "service" },
  { actorId: "agt_qa", actor: "QA Runner", actorKind: "agent" },
]

/** The hook's answer, rendered into a picker. */
function Picker({
  value = "",
  onChange = () => {},
}: {
  value?: string
  onChange?: (next: string) => void
}) {
  const { actors, note } = useAuditActors()
  return (
    <AuditActorPicker
      label="Person"
      value={value}
      onChange={onChange}
      actors={actors}
      note={note}
    />
  )
}

describe("AuditActorPicker", () => {
  it("offers the trail's actors by name with their kind, and picks by id", async () => {
    spyOn(governanceService, "listAuditActors").mockResolvedValue({ actors: ACTORS })
    const onChange = mock<(next: string) => void>(() => {})
    render(<Picker onChange={onChange} />)

    const trigger = await screen.findByRole("button", { name: "Person" })
    expect(trigger.textContent).toContain("Pick a person")
    fireEvent.click(trigger)
    const options = await screen.findAllByRole("option")
    expect(options.map((option) => option.textContent)).toEqual([
      "Platform Admin · User",
      "Ingest Loader · Service",
      "QA Runner · Agent",
    ])
    fireEvent.click(options[2] as HTMLElement)
    expect(onChange.mock.calls[0][0]).toBe("agt_qa")
  })

  it("narrows the options as a name or an id is typed", async () => {
    spyOn(governanceService, "listAuditActors").mockResolvedValue({ actors: ACTORS })
    render(<Picker />)
    fireEvent.click(await screen.findByRole("button", { name: "Person" }))
    expect((await screen.findAllByRole("option")).length).toBe(3)

    fireEvent.change(screen.getByLabelText("Search Person"), { target: { value: "runner" } })
    const byName = await screen.findAllByRole("option")
    expect(byName.map((option) => option.textContent)).toEqual(["QA Runner · Agent"])

    fireEvent.change(screen.getByLabelText("Search Person"), { target: { value: "svc_" } })
    const byId = await screen.findAllByRole("option")
    expect(byId.map((option) => option.textContent)).toEqual(["Ingest Loader · Service"])

    fireEvent.change(screen.getByLabelText("Search Person"), { target: { value: "nobody" } })
    expect(await screen.findByText("No person matches.")).toBeDefined()
    expect(screen.queryByRole("option")).toBeNull()
  })

  it("keeps a held id the trail's list does not name, marked, rather than dropping it", async () => {
    spyOn(governanceService, "listAuditActors").mockResolvedValue({ actors: ACTORS })
    render(<Picker value="usr_gone" />)

    const trigger = await screen.findByRole("button", { name: "Person" })
    expect(trigger.textContent).toContain("usr_gone (not in the trail)")
    fireEvent.click(trigger)
    expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toContain(
      "usr_gone (not in the trail)",
    )
  })

  it("is free text, with the reason, when the actors route cannot be read", async () => {
    spyOn(governanceService, "listAuditActors").mockRejectedValue(new Error("route unavailable"))
    render(<Picker />)
    const input = (await screen.findByRole("textbox", { name: "Person" })) as HTMLInputElement
    expect(input.value).toBe("")
    expect(screen.getByText(/Could not read the trail's people/)).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Person" })).toBeNull()
  })
})
