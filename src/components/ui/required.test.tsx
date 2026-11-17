import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "bun:test"

import { FieldLabel } from "./field"
import { Label } from "./label"
import { RequiredMarker } from "./required"
import { RequiredLegend } from "@/components/patterns/required-legend"

// The star is a CSS ::before on an empty aria-hidden span (UX-1). aria-hidden
// would already keep a text glyph out of a browser's accessible name; the CSS
// is chosen because testing-library matches labels on textContent, so a
// literal `*` would break existing exact getByLabelText queries. These tests
// assert that textContent/data-required invariant, not the glyph itself
// (happy-dom does not apply ::before).
const marker = (container: HTMLElement) => container.querySelector("span[aria-hidden='true'][class*=\"before:content-\"]")

describe("required-field marker (UX-1)", () => {
  it("renders a decorative star only when a Label is explicitly required", () => {
    const { container } = render(
      <>
        <Label required htmlFor="x">Name</Label>
        <input id="x" />
      </>
    )
    const label = screen.getByText("Name")
    expect(label.getAttribute("data-required")).toBe("")
    expect(marker(container)).not.toBeNull()
    // `required` is consumed, never forwarded as an invalid <label> attribute.
    expect(label.hasAttribute("required")).toBe(false)
    // The label's text is unchanged, so getByLabelText("Name") still works.
    expect(screen.getByLabelText("Name")).toBeDefined()
  })

  it("leaves optional labels unmarked", () => {
    const { container } = render(<Label>Description</Label>)
    expect(marker(container)).toBeNull()
    expect(screen.getByText("Description").hasAttribute("data-required")).toBe(false)
  })

  it("carries the required state through the shared FieldLabel wrapper", () => {
    const { container } = render(<FieldLabel required>Email</FieldLabel>)
    const label = screen.getByText("Email")
    expect(label.getAttribute("data-slot")).toBe("field-label")
    expect(label.getAttribute("data-required")).toBe("")
    expect(marker(container)).not.toBeNull()
  })

  it("explains the convention once with a hidden star and a spoken word", () => {
    const { container } = render(<RequiredLegend />)
    expect(screen.getByText(/Required/)).toBeDefined()
    expect(marker(container)).not.toBeNull()
  })

  it("exposes the marker as a standalone opt-in for custom controls", () => {
    const { container } = render(<span id="sql-label">SQL <RequiredMarker /></span>)
    expect(marker(container)).not.toBeNull()
  })
})
