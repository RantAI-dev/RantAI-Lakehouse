import { render, screen } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"
import { SqlDialForm } from "./sql-dial-form"
import { RestDialForm } from "./rest-dial-form"

describe("dial forms", () => {
  it("SqlDialForm renders host/port/database/user as plain inputs, no secret picker for user", () => {
    render(<SqlDialForm value={null} onChange={mock()} />)
    expect(screen.getByLabelText("Host")).toBeDefined()
    expect(screen.getByLabelText("User")).toHaveProperty("type", "text")
  })
  it("RestDialForm renders an auth-type selector with all four types", () => {
    render(<RestDialForm value={null} onChange={mock()} />)
    const select = screen.getByLabelText("Auth type") as HTMLSelectElement
    const options = Array.from(select.options).map((o) => o.value)
    expect(options).toEqual(["api_key", "bearer", "oauth2_client_credentials", "basic"])
  })
})
