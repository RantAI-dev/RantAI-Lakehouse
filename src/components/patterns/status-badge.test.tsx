import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { CertificationBadge, OutcomeBadge } from "./status-badge"

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

describe("OutcomeBadge", () => {
  it("reads Pending for a decision that is still waiting, in the info tone", () => {
    render(
      <>
        <OutcomeBadge outcome="pending" />
        <OutcomeBadge outcome="denied" />
      </>
    )
    const pending = screen.getByText("Pending")
    // The `info` tone: a decision still open is neither a warning nor a
    // success (GOV-5).
    expect(pending.className).toContain("text-primary")
    expect(pending.className).not.toContain("amber")
    expect(pending.className).not.toContain("emerald")
    // A real refusal keeps the warning tone.
    expect(screen.getByText("Denied").className).toContain("amber")
  })
})
