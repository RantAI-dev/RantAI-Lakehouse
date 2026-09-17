import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "bun:test"
import { Button } from "./button"

describe("component test harness (judge amendment A2)", () => {
  it("renders a trivial existing component through happy-dom", () => {
    render(<Button>Click me</Button>)
    expect(screen.getByText("Click me")).toBeDefined()
  })
})
