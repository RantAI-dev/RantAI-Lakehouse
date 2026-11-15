import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { Avatar } from "./library-parts"

afterEach(cleanup)

// BI-59 owner QA 2026-11-14: the initials sat low in the circle on the row's tall line height.
describe("Avatar", () => {
  it("centres the initials: leading-none, also with the gallery's smaller size", () => {
    const { container } = render(
      <>
        <Avatar owner="Budi Ajeng" />
        <Avatar owner="Budi Ajeng" className="size-4 text-[8px]" />
      </>,
    )
    const spans = [...container.querySelectorAll("span")]
    expect(spans.map((s) => s.textContent)).toEqual(["BA", "BA"])
    for (const s of spans) {
      expect(s.className).toContain("leading-none")
      expect(s.className).toContain("place-items-center")
    }
  })

  it("keeps the built-in box centred too", () => {
    const { container } = render(<Avatar owner="" builtin />)
    expect(container.querySelector("span")?.className).toContain("leading-none")
  })
})
