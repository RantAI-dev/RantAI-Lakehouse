// The route loading fallback draws the page's own placeholder (owner QA 2026-11-15): the
// dashboard's on a board, the list's on the Dashboards page, nothing on a public path
// (`/login` has no console frame), the generic one elsewhere. `next/navigation` is stubbed
// before any import resolves (`mock.module` is hoisted).
import { afterEach, describe, expect, it, mock } from "bun:test"

let pathname = "/"
mock.module("next/navigation", () => ({
  usePathname: () => pathname,
}))

import { cleanup, render, screen } from "@testing-library/react"
import Loading from "./loading"

afterEach(cleanup)

function at(path: string) {
  pathname = path
  return render(<Loading />)
}

describe("route loading fallback", () => {
  it("draws the dashboard placeholder on a board", () => {
    at("/dashboards/b1")
    expect(screen.getByRole("status", { name: "Loading dashboard" })).toBeTruthy()
    expect(screen.queryByRole("status", { name: "Loading" })).toBeNull()
  })

  it("draws the list placeholder on the Dashboards page", () => {
    at("/dashboards/browse")
    expect(screen.getByRole("status", { name: "Loading dashboards" })).toBeTruthy()
    expect(screen.queryByRole("status", { name: "Loading dashboard" })).toBeNull()
  })

  it("draws Home's placeholder on /", () => {
    at("/")
    expect(screen.getByRole("status", { name: "Loading Home" })).toBeTruthy()
    expect(screen.queryByRole("status", { name: "Loading" })).toBeNull()
  })

  it("draws nothing on /login", () => {
    const { container } = at("/login")
    expect(container.innerHTML).toBe("")
  })

  it("draws the generic placeholder on another page", () => {
    at("/alerts")
    expect(screen.getByRole("status", { name: "Loading" })).toBeTruthy()
    expect(screen.queryByRole("status", { name: "Loading dashboard" })).toBeNull()
  })
})
