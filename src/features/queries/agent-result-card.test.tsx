import { render, within } from "@testing-library/react"
import { describe, expect, it } from "bun:test"
import type { NlAnswer } from "@/services/contracts/queries"
import { AgentResultCard } from "./agent-result-card"
import { NaturalLanguagePanel } from "./nl-panel"

// The chat writes Markdown and wraps a number it could not match to a tool
// result in <span data-unverified="true">. The box must show both the way
// the Copilot does, not as literal source text.
const ANSWER = 'Revenue was **bold** and <span data-unverified="true">999999</span>.'

function answer(overrides: Partial<NlAnswer> = {}): NlAnswer {
  return { answer: ANSWER, columns: [], rows: [], rowCount: 0, steps: [], ...overrides }
}

describe("AgentResultCard answer rendering", () => {
  it("renders Markdown and the unverified marker instead of printing their source", () => {
    const { container } = render(<AgentResultCard result={answer({ sql: "SELECT 1" })} />)
    expect(container.textContent).not.toContain("**")
    expect(container.textContent).not.toContain("<span")
    expect(within(container).getByText("bold").tagName).toBe("STRONG")
    const marked = within(container).getByText("999999")
    expect(marked.getAttribute("data-unverified")).toBe("true")
  })

  it("shows no row count when the chat ran no query", () => {
    const { container } = render(<AgentResultCard result={answer({ sql: undefined })} />)
    expect(container.textContent).not.toContain("0 rows")
    expect(container.textContent).not.toContain("the final SQL is loaded")
  })

  it("shows the row count and the SQL hint when a query ran", () => {
    const { container } = render(
      <AgentResultCard result={answer({ sql: "SELECT 1", rowCount: 3 })} />,
    )
    expect(container.textContent).toContain("3 rows")
    expect(container.textContent).toContain("the final SQL is loaded")
  })
})

describe("NaturalLanguagePanel explanation rendering", () => {
  it("renders the generated SQL's explanation as Markdown", () => {
    // Only the fields the panel reads; the rest of the studio is irrelevant here.
    const studio = {
      question: "q",
      setQuestion: () => {},
      generate: async () => {},
      generateAct: {
        status: "success",
        data: { sql: "SELECT 1", explanation: ANSWER },
      },
      agent: { busy: false, error: null, result: null, ask: async () => {} },
    } as unknown as Parameters<typeof NaturalLanguagePanel>[0]["studio"]
    const { container } = render(<NaturalLanguagePanel studio={studio} />)
    expect(container.textContent).not.toContain("**")
    expect(container.textContent).not.toContain("<span")
    expect(within(container).getByText("bold").tagName).toBe("STRONG")
  })
})
