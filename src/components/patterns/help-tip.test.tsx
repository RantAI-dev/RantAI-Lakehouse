import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { HelpTip } from "./help-tip"

afterEach(cleanup)

describe("HelpTip", () => {
  it("has the accessible name 'More about <heading>' and hides the detail until asked", () => {
    render(<HelpTip heading="Public links">Nothing is deleted.</HelpTip>)
    expect(screen.getByRole("button", { name: "More about Public links" })).toBeDefined()
    expect(screen.queryByText("Nothing is deleted.")).toBeNull()
  })

  it("opens on a click, the way a tap on a phone does, and shows the text", async () => {
    render(<HelpTip heading="Public links">Nothing is deleted.</HelpTip>)
    fireEvent.click(screen.getByRole("button", { name: "More about Public links" }))
    expect(await screen.findByText("Nothing is deleted.")).toBeDefined()
  })

  it("opens from the keyboard: focus, then Enter", async () => {
    render(<HelpTip heading="Fonts">Only shipped fonts.</HelpTip>)
    const button = screen.getByRole("button", { name: "More about Fonts" })
    button.focus()
    expect(document.activeElement).toBe(button)
    // A focused button turns Enter into a click, which happy-dom does not do for us.
    fireEvent.keyDown(button, { key: "Enter" })
    fireEvent.click(button)
    expect(await screen.findByText("Only shipped fonts.")).toBeDefined()
  })
})
