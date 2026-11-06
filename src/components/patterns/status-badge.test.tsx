import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { CertificationBadge, OutcomeBadge, StatusBadge } from "./status-badge"

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

describe("StatusBadge", () => {
  it("reads Stopped for a stopped run, with the neutral tone and its description", () => {
    render(<StatusBadge status="stopped" />)
    const badge = screen.getByText("Stopped")
    expect(badge.getAttribute("title")).toBe("Stopped by the person who ran it.")
    expect(badge.className).toContain("text-muted-foreground")
  })

  it("shows a page's own label for a status and keeps the shared description", () => {
    render(<StatusBadge status="blocked" label="Refused" />)
    const badge = screen.getByText("Refused")
    expect(badge.getAttribute("title")).toBe("Stopped by policy, quota, or approval gate.")
    expect(screen.queryByText("Blocked")).toBeNull()
  })
})
