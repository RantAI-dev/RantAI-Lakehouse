import { afterEach, describe, expect, it } from "bun:test"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { UserMenu } from "./app-navbar"
import { CONSOLE_VERSION } from "@/features/version/console-version"

/**
 * The account menu says which build the console is (REL-2 S1-T5): a
 * person reporting a problem, or an operator checking what a host runs,
 * needs the number without opening a console page.
 */
describe("the account menu", () => {
  afterEach(() => cleanup())

  it("shows the console's version", () => {
    render(<UserMenu userName="Ada" userEmail="ada@example.com" onLogout={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /Account menu/ }))
    expect(screen.getByText(`Version ${CONSOLE_VERSION}`)).toBeTruthy()
  })
})
