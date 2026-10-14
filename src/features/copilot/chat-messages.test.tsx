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

const ASK_STEP = {
  tool: "ask_user",
  args: { term: "active customer" },
  ok: true,
  result: {
    asked: true,
    term: "active customer",
    question: "What counts as an active customer?",
    options: ["Ordered in the last 30 days", "Ordered in the last 90 days"],
  },
}

const asking: Msg[] = [
  { id: "u1", role: "user", content: "Show active customers", at: "2026-10-07T07:00:00Z" },
  {
    id: "a1",
    role: "assistant",
    content: "What counts as an active customer?",
    at: "2026-10-07T07:00:05Z",
    tools: [{ tool: "list_datasets", args: {}, ok: true, result: { datasets: [] } }, ASK_STEP],
  },
]

describe("ChatMessages ask-back options", () => {
  it("shows the options of the newest ask as buttons and hides the ask_user step itself", () => {
    const { container } = render(<ChatMessages messages={asking} busy={false} onAnswerAsk={() => {}} />)
    const view = within(container)
    expect(view.getByRole("button", { name: "Ordered in the last 30 days" })).toBeDefined()
    expect(view.getByRole("button", { name: "Ordered in the last 90 days" })).toBeDefined()
    // The other tool step is still a card; only the ask is not.
    expect(view.getByText("Search")).toBeDefined()
    expect(view.queryByText("Ask")).toBeNull()
  })

  it("passes the term, the picked option and the user's previous message when an option is clicked", () => {
    const onAnswerAsk = mock(() => {})
    const { container } = render(<ChatMessages messages={asking} busy={false} onAnswerAsk={onAnswerAsk} />)
    fireEvent.click(within(container).getByRole("button", { name: "Ordered in the last 90 days" }))
    expect(onAnswerAsk).toHaveBeenCalledTimes(1)
    expect(onAnswerAsk).toHaveBeenCalledWith({
      term: "active customer",
      option: "Ordered in the last 90 days",
      question: "Show active customers",
    })
  })

  it("shows the options as plain text, not buttons, once another message follows the ask", () => {
    const followed: Msg[] = [
      ...asking,
      { id: "u2", role: "user", content: "Ordered in the last 30 days", at: "2026-10-07T07:01:00Z" },
    ]
    const { container } = render(<ChatMessages messages={followed} busy={false} onAnswerAsk={() => {}} />)
    const view = within(container)
    expect(view.queryByRole("button", { name: "Ordered in the last 30 days" })).toBeNull()
    expect(view.queryByRole("button", { name: "Ordered in the last 90 days" })).toBeNull()
    expect(container.textContent).toContain("Ordered in the last 90 days")
  })

  it("disables the option buttons while an answer is on its way", () => {
    const { container } = render(<ChatMessages messages={asking} busy onAnswerAsk={() => {}} />)
    const button = within(container).getByRole("button", { name: "Ordered in the last 30 days" }) as HTMLButtonElement
    expect(button.disabled).toBe(true)
  })

  it("shows no buttons for an ask_user step that failed or is malformed", () => {
    const broken: Msg[] = [
      asking[0]!,
      {
        ...asking[1]!,
        tools: [{ ...ASK_STEP, result: { asked: true, term: "t", question: "q", options: ["only one"] } }],
      },
    ]
    const { container } = render(<ChatMessages messages={broken} busy={false} onAnswerAsk={() => {}} />)
    expect(within(container).queryByRole("button", { name: "only one" })).toBeNull()
  })

  it("shows the notice line when the answer was not remembered", () => {
    const { container } = render(
      <ChatMessages messages={asking} busy={false} onAnswerAsk={() => {}} askNotice="Your answer was not remembered." />,
    )
    expect(within(container).getByText("Your answer was not remembered.")).toBeDefined()
  })
})
