import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import { ErrorWithReference } from "./error-reference"
import { TileBody } from "@/features/dashboards/tile-body"
import { chatErrorText } from "@/features/copilot/use-copilot"

// SEC-11: a failed tile shows the server's fixed sentence and, small and
// selectable, the reference the raw error was logged under. A message the
// product wrote itself carries no reference, and none is invented.
afterEach(cleanup)

const spec = { id: "c1", kind: "bar", title: "T" } as never

describe("ErrorWithReference", () => {
  it("shows the message and the reference when there is one", () => {
    const { container } = render(
      <ErrorWithReference message="This chart could not be loaded." errorId="ab12cd34ef" />,
    )
    expect(container.textContent).toContain("This chart could not be loaded.")
    expect(container.textContent).toContain("Reference: ab12cd34ef")
    expect(container.querySelector(".select-text")).not.toBeNull()
  })

  it("shows no reference line when the server gave none", () => {
    const { container } = render(<ErrorWithReference message="the SQL source no longer exists" />)
    expect(container.textContent).toBe("the SQL source no longer exists")
    expect(container.textContent).not.toContain("Reference")
  })
})

describe("TileBody on a failed tile", () => {
  it("shows the fixed message and the reference", () => {
    const { container } = render(
      <TileBody
        spec={spec}
        cell={{ error: "This chart could not be loaded.", errorId: "ab12cd34ef" }}
        dark={false}
        loading={false}
      />,
    )
    expect(container.textContent).toContain("This chart could not be loaded.")
    expect(container.textContent).toContain("Reference: ab12cd34ef")
  })

  it("keeps showing our own message unchanged when it has no reference", () => {
    const { container } = render(
      <TileBody
        spec={spec}
        cell={{ error: "this chart's definition is invalid" }}
        dark={false}
        loading={false}
      />,
    )
    expect(container.textContent).toBe("this chart's definition is invalid")
  })
})

describe("chatErrorText", () => {
  it("appends the reference the server logged the provider's error under", () => {
    expect(
      chatErrorText({ error: "AI Copilot is unavailable", detail: "Try again later.", errorId: "ab12" }),
    ).toBe("Try again later. Reference: ab12")
  })

  it("is unchanged without a reference", () => {
    expect(chatErrorText({ detail: "Try again later." })).toBe("Try again later.")
    expect(chatErrorText(null)).toBe("Copilot couldn't answer. Try again.")
  })
})
