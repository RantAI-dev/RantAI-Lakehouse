import { fireEvent, render, within } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"

import { ChatMessages } from "./chat-messages"
import type { Msg } from "./use-copilot"

// Queries stay inside each render's own container: the test process shares
// one global document across files (see mini-markdown.test.tsx).

const conversation: Msg[] = [
  { id: "u1", role: "user", content: "How many datasets?", at: "2026-09-25T07:00:00Z" },
  {
    id: "a1",
    role: "assistant",
    content: "There are **6 datasets**.",
    at: "2026-09-25T07:00:05Z",
    elapsedMs: 5100,
    reasoning: "Use list_datasets.",
    reasoningMs: 1800,
    tools: [
      { tool: "list_datasets", args: {}, ok: true, result: { datasets: [] } },
      { tool: "run_sql", args: { sql: "SELECT 1" }, ok: false, result: { error: "boom" } },
    ],
  },
]

describe("ChatMessages (RantAI-Agents layout)", () => {
  it("shows the answer with a closed reasoning box, one row per tool and the answer time, and no name or avatar", () => {
    const { container } = render(<ChatMessages messages={conversation} busy={false} />)
    const view = within(container)
    // The avatar and the "Copilot" name beside each answer were removed on
    // purpose (QA feedback); the user's bubble already says who speaks.
    expect(view.queryByText("Copilot")).toBeNull()
    expect(view.getByText("Thought for 2s")).toBeDefined()
    expect(view.queryByText("Use list_datasets.")).toBeNull()
    expect(view.getByText("Search")).toBeDefined()
    expect(view.getByText("“SELECT 1”")).toBeDefined()
    expect(container.textContent).toContain("answered in 5.1s")
  })

  it("offers edit on the user's message and regenerate on the last answer, and resends an edit", () => {
    const onEdit = mock(() => {})
    const onRetry = mock(() => {})
    const { container } = render(
      <ChatMessages messages={conversation} busy={false} onEdit={onEdit} onRetry={onRetry} />,
    )
    const view = within(container)
    fireEvent.click(view.getByLabelText("Regenerate response"))
    expect(onRetry).toHaveBeenCalledTimes(1)

    fireEvent.click(view.getByLabelText("Edit message"))
    fireEvent.change(view.getByLabelText("Edit message", { selector: "textarea" }), {
      target: { value: "How many marts?" },
    })
    fireEvent.click(view.getByText("Save & Resend"))
    expect(onEdit).toHaveBeenCalledWith(0, "How many marts?")
  })

  it("while busy shows the live reasoning open, the tool being run, and the status pill", () => {
    const { container } = render(
      <ChatMessages
        messages={conversation.slice(0, 1)}
        busy
        liveReasoning="Checking the catalog"
        progress={{ phase: "tool", tool: "list_datasets", startedAt: Date.now(), steps: ["list_datasets"] }}
      />,
    )
    const view = within(container)
    expect(view.getByText("Thinking…")).toBeDefined()
    expect(view.getByText("Checking the catalog")).toBeDefined()
    expect(view.getByText("Running Search datasets…")).toBeDefined()
  })
})
