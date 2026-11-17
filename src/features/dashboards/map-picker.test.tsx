import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { MAP_CATALOGUE } from "./echarts-maps"
import { MapPicker } from "./map-picker"

// BI-16 C2a follow-up (planner, picker search).
afterEach(cleanup)

// cmdk scrolls the selected item into view; jsdom has no such method.
Element.prototype.scrollIntoView = Element.prototype.scrollIntoView ?? (() => {})

const open = async (value = "dki-jakarta", onChange: (id: string) => void = () => {}) => {
  render(<MapPicker maps={MAP_CATALOGUE} value={value} onChange={onChange} />)
  fireEvent.click(screen.getByRole("combobox", { name: "Map" }))
  return screen.findByLabelText("Search maps")
}

describe("the map picker", () => {
  it("shows the chosen map and lists Jakarta and Indonesia first", async () => {
    await open("id-provinces")
    expect(screen.getByRole("combobox", { name: "Map" }).textContent).toContain("Indonesia — provinces")
    const labels = screen.getAllByRole("option").map((o) => o.textContent)
    expect(labels.slice(0, 3)).toEqual(["Jakarta — cities", "Indonesia — provinces", "Indonesia — kabupaten/kota"])
    expect(labels[3]).toBe("World — countries")
    expect(labels.length).toBe(MAP_CATALOGUE.length)
  })

  it("narrows the list as the user types and says so when nothing matches", async () => {
    const search = await open()
    fireEvent.change(search, { target: { value: "brazil" } })
    await waitFor(() => expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["Brazil — states/provinces"]))
    fireEvent.change(search, { target: { value: "zzzz" } })
    await waitFor(() => expect(screen.getByText("No map matches.")).toBeTruthy())
  })

  it("picks a map and closes", async () => {
    const onChange = mock<(id: string) => void>()
    const search = await open("dki-jakarta", onChange)
    fireEvent.change(search, { target: { value: "united states" } })
    const option = await waitFor(() => within(screen.getByRole("listbox")).getByText("United States of America — states/provinces"))
    fireEvent.click(option)
    expect(onChange).toHaveBeenCalledWith("adm1-usa")
    await waitFor(() => expect(screen.queryByLabelText("Search maps")).toBeNull())
  })
})
