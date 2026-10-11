import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { CertificationBadge } from "./status-badge"

afterEach(cleanup)

describe("CertificationBadge", () => {
  it("says Certified or Deprecated, with an explanation on hover", () => {
    render(
      <>
        <CertificationBadge certification="certified" />
        <CertificationBadge certification="deprecated" />
      </>
    )
    expect(screen.getByText("Certified").getAttribute("title")).toContain("vouched")
    expect(screen.getByText("Deprecated").getAttribute("title")).toContain("do not build new work")
  })

  it("renders nothing without a mark, or for a mark it does not know", () => {
    const { container } = render(
      <>
        <CertificationBadge certification={undefined} />
        <CertificationBadge certification={null} />
        <CertificationBadge certification="endorsed" />
      </>
    )
    expect(container.textContent).toBe("")
  })
})
