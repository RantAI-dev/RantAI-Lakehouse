import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"

import { PRIVATE_HINT, PRIVATE_TIP, PrivateBadge } from "./private-badge"
import { ShareBadges } from "./dashboard-list-columns"

afterEach(cleanup)

describe("PrivateBadge (BI-18 part C review fix R11)", () => {
  it("shows the lock and Private, and carries the sentence the tip says", () => {
    render(<PrivateBadge />)
    expect(screen.getByText("Private")).toBeTruthy()
    expect(PRIVATE_TIP).toBe("Only you and administrators can see it. Move it to a folder to share it.")
  })

  it("is the badge the list card shows for a board with an owner, and only for one", () => {
    render(<ShareBadges board={{ id: "b", name: "n", ownerId: "u" }} />)
    expect(screen.getByText("Private")).toBeTruthy()
    cleanup()
    render(<ShareBadges board={{ id: "b", name: "n" }} />)
    expect(screen.queryByText("Private")).toBeNull()
  })

  it("has the hint the New dashboard dialog shows under the personal space", () => {
    expect(PRIVATE_HINT).toContain("cannot be shared until you move it to a folder")
  })
})
