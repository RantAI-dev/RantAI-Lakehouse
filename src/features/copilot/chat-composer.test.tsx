// The chat box is back to its pre-#568 presentation (UX-4 decision 4): no
// visible "Message" label, no "* Required" line, no tooltip, no hidden
// requirement text. What must survive is behaviour: the textarea/input keep
// `aria-required` (UX-4 decision 3), the animated example still sits inside
// a `relative` containing block with only the textarea and the overlay, and
// the send button stays disabled until something is typed.
import { render, within } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"

import { ChatComposer } from "./chat-composer"

const base = {
  mode: "ask" as const,
  setMode: mock(),
  onSend: mock(),
  busy: false,
}

describe("ChatComposer after the required marks were removed (UX-4)", () => {
  it("keeps the animated example inside a relative wrapper with the textarea", () => {
    const { container } = render(<ChatComposer {...base} examples={["one example"]} />)
    const view = within(container)
    const textarea = view.getByLabelText("Message AI Copilot")
    expect(textarea.getAttribute("aria-required")).toBe("true")
    const wrapper = textarea.parentElement as HTMLElement

    // The absolutely positioned overlay needs a positioned ancestor, and
    // that wrapper contains only the textarea and the overlay.
    expect(wrapper.className).toContain("relative")
    const overlay = wrapper.querySelector(".absolute")
    expect(overlay).not.toBeNull()
    expect(overlay?.parentElement).toBe(wrapper)
    expect(wrapper.querySelector("label")).toBeNull()
    // No visible label, no legend.
    expect(container.querySelector('label[for="copilot-message"]')).toBeNull()
    expect(view.queryByText(/Required/)).toBeNull()
  })

  it("keeps the compact pill label-free but still required", () => {
    const { container } = render(<ChatComposer {...base} compact />)
    const view = within(container)
    const input = view.getByLabelText("Message AI Copilot")
    expect(input.getAttribute("aria-required")).toBe("true")
    // No visible "Message" label, no tooltip, no hidden requirement text.
    expect(container.querySelector('label[for="copilot-compact-message"]')).toBeNull()
    expect(input.getAttribute("aria-describedby")).toBeNull()
    expect(view.queryByText(/message is required/i)).toBeNull()
    expect(view.queryByText(/Required/)).toBeNull()
  })
})
