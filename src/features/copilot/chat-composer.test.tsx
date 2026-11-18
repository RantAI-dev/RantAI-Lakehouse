// The required-field presentation of the composer: UX-1-R1 (the animated
// example must keep its relative containing block) and UX-1-R2 (the compact
// pill must show its required label instead of hiding it in sr-only).
import { render, within } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"

import { ChatComposer } from "./chat-composer"

const base = {
  mode: "ask" as const,
  setMode: mock(),
  onSend: mock(),
  busy: false,
}

describe("ChatComposer required-field presentation (UX-1-R1, UX-1-R2)", () => {
  it("puts the animated example inside a relative wrapper below the legend and label", () => {
    const { container } = render(<ChatComposer {...base} examples={["one example"]} />)
    const view = within(container)
    const textarea = view.getByLabelText("Message AI Copilot")
    const wrapper = textarea.parentElement as HTMLElement

    // R1: the absolutely positioned overlay needs a positioned ancestor, and
    // that wrapper contains only the textarea and the overlay — not the label.
    expect(wrapper.className).toContain("relative")
    const overlay = wrapper.querySelector(".absolute")
    expect(overlay).not.toBeNull()
    expect(overlay?.parentElement).toBe(wrapper)
    const label = container.querySelector('label[for="copilot-message"]') as HTMLElement
    expect(wrapper.contains(label)).toBe(false)
    expect(label.getAttribute("data-required")).toBe("")
    // The convention is still explained once above the wrapper.
    expect(view.getByText(/Required/)).toBeDefined()
  })

  it("shows a visible associated required label in the compact pill", () => {
    const { container } = render(<ChatComposer {...base} compact />)
    const view = within(container)
    const input = view.getByLabelText("Message")
    expect(input.getAttribute("aria-required")).toBe("true")
    expect(input.getAttribute("aria-describedby")).toBe("copilot-compact-required-help")
    const label = container.querySelector('label[for="copilot-compact-message"]') as HTMLElement
    expect(label.getAttribute("data-required")).toBe("")
    expect(label.className).not.toContain("sr-only")
    // R2: the compact surface explains the convention without hiding the star.
    expect(view.getByText(/star marks required fields/i)).toBeDefined()
  })
})
