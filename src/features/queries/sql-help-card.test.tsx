import { cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ExplainCard, FixCard, FixWithAiButton } from "./sql-help-card"

afterEach(() => {
  cleanup()
})

const EDITOR = "SELECT amont FROM t\nWHERE x = 1"
const PROPOSED = "SELECT amount FROM t\nWHERE x = 1"

describe("FixCard for a proposal", () => {
  it("shows the reason, marks the changed line with a sign and a colour, and offers Accept and Reject", () => {
    const onAccept = mock(() => {})
    const onReject = mock(() => {})
    render(
      <FixCard
        outcome={{ sql: PROPOSED, reason: "The column is named amount." }}
        editorSql={EDITOR}
        onAccept={onAccept}
        onReject={onReject}
      />,
    )

    expect(screen.getByText("The column is named amount.")).toBeTruthy()
    const diff = screen.getByRole("list", { name: "Changes to the query" })
    const removed = within(diff).getByText("SELECT amont FROM t").closest("li") as HTMLElement
    const added = within(diff).getByText("SELECT amount FROM t").closest("li") as HTMLElement
    // The sign and the colour both tell the two apart: colour alone would not.
    expect(removed.textContent).toContain("−")
    expect(removed.className).toContain("text-red-")
    expect(added.textContent).toContain("+")
    expect(added.className).toContain("text-emerald-")
    // The unchanged line is shown without a change mark.
    const kept = within(diff).getByText("WHERE x = 1").closest("li") as HTMLElement
    expect(kept.className).not.toContain("text-red-")
    expect(kept.className).not.toContain("text-emerald-")

    fireEvent.click(screen.getByRole("button", { name: /Accept/ }))
    fireEvent.click(screen.getByRole("button", { name: /Reject/ }))
    expect(onAccept).toHaveBeenCalledTimes(1)
    expect(onReject).toHaveBeenCalledTimes(1)
  })
})

describe("FixCard with no proposal", () => {
  it("shows the assistant's text and says no fix was proposed, with no Accept or Reject", () => {
    render(
      <FixCard
        outcome={{ answer: "I need the table's columns to fix this." }}
        editorSql={EDITOR}
        onAccept={() => {}}
        onReject={() => {}}
      />,
    )
    expect(screen.getByText("I need the table's columns to fix this.")).toBeTruthy()
    expect(screen.getByText("No fix to apply was proposed.")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /Accept/ })).toBeNull()
    expect(screen.queryByRole("button", { name: /Reject/ })).toBeNull()
  })
})

describe("FixWithAiButton", () => {
  it("is disabled for Trino with the reason as its title, and calls nothing", () => {
    const onFix = mock(() => {})
    render(<FixWithAiButton engine="trino" busy={false} onFix={onFix} />)
    const button = screen.getByRole("button", { name: /Fix with AI/ }) as HTMLButtonElement
    expect(button.disabled).toBe(true)
    expect(button.title).toBe("fixing a Trino query is not supported yet")
    fireEvent.click(button)
    expect(onFix).not.toHaveBeenCalled()
  })

  it("is enabled for ClickHouse and calls the fix when pressed", () => {
    const onFix = mock(() => {})
    render(<FixWithAiButton engine="clickhouse" busy={false} onFix={onFix} />)
    const button = screen.getByRole("button", { name: /Fix with AI/ }) as HTMLButtonElement
    expect(button.disabled).toBe(false)
    fireEvent.click(button)
    expect(onFix).toHaveBeenCalledTimes(1)
  })
})

describe("ExplainCard", () => {
  it("renders the explanation as Markdown, not as its source text", () => {
    const { container } = render(<ExplainCard text="It counts **rows** of `t`." />)
    expect(container.textContent).not.toContain("**")
    expect(within(container).getByText("rows").tagName).toBe("STRONG")
  })
})
