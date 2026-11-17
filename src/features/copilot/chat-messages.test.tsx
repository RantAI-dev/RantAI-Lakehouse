import { fireEvent, render, within } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"
import * as React from "react"

import { ChatMessages, useStickToBottom } from "./chat-messages"
import { chatHistory, CopilotStreamProvider, type CopilotStreamValue, type Msg } from "./use-copilot"

/**
 * Renders the list with a stream value (AI-24 D6: the list reads the
 * streaming part of the Copilot from its own context).
 */
function renderMessages(
  props: React.ComponentProps<typeof ChatMessages>,
  stream: Partial<CopilotStreamValue> = {},
) {
  return render(
    <CopilotStreamProvider value={{ draft: "", liveReasoning: "", progress: null, ...stream }}>
      <ChatMessages {...props} />
    </CopilotStreamProvider>,
  )
}

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
    const { container } = renderMessages({ messages: conversation, busy: false })
    const view = within(container)
    // The avatar and the "Copilot" name beside each answer were removed on
    // purpose (QA feedback); the user's bubble already says who speaks.
    // AI-23: the rename must not bring the label back, under either name.
    expect(view.queryByText("Copilot")).toBeNull()
    expect(view.queryByText("AI Copilot")).toBeNull()
    expect(view.getByText("Thought for 2s")).toBeDefined()
    expect(view.queryByText("Use list_datasets.")).toBeNull()
    expect(view.getByText("Search")).toBeDefined()
    expect(view.getByText("“SELECT 1”")).toBeDefined()
    expect(container.textContent).toContain("answered in 5.1s")
  })

  it("offers edit on the user's message and regenerate on the last answer, and resends an edit", () => {
    const onEdit = mock(() => {})
    const onRetry = mock(() => {})
    const { container } = renderMessages({
      messages: conversation, busy: false, onEdit, onRetry,
    })
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
    const { container } = renderMessages(
      { messages: conversation.slice(0, 1), busy: true },
      {
        liveReasoning: "Checking the catalog",
        progress: { phase: "tool", tool: "list_datasets", startedAt: Date.now(), steps: ["list_datasets"] },
      },
    )
    const view = within(container)
    expect(view.getByText("Thinking…")).toBeDefined()
    expect(view.getByText("Checking the catalog")).toBeDefined()
    expect(view.getByText("Running Search datasets…")).toBeDefined()
  })

  it('shows a stopped answer as "Response stopped.", not the line the model is sent (BI-41 D3)', () => {
    const stopped: Msg[] = [
      { id: "u1", role: "user", content: "How many events?" },
      { id: "a1", role: "assistant", content: "", stopped: true },
    ]
    // AI-24 D6 moved the streaming part onto its own context, so the list no
    // longer mounts without it; this case still renders the list alone.
    const { container } = renderMessages({ messages: stopped, busy: false })
    const view = within(container)
    expect(view.getByText("Response stopped.")).toBeDefined()
    expect(container.textContent).not.toContain("The person stopped this answer")
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
    const { container } = renderMessages({ messages: asking, busy: false, onAnswerAsk: () => {} })
    const view = within(container)
    expect(view.getByRole("button", { name: "Ordered in the last 30 days" })).toBeDefined()
    expect(view.getByRole("button", { name: "Ordered in the last 90 days" })).toBeDefined()
    // The other tool step is still a card; only the ask is not.
    expect(view.getByText("Search")).toBeDefined()
    expect(view.queryByText("Ask")).toBeNull()
  })

  it("passes the term, the picked option and the user's previous message when an option is clicked", () => {
    const onAnswerAsk = mock(() => {})
    const { container } = renderMessages({ messages: asking, busy: false, onAnswerAsk })
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
    const { container } = renderMessages({ messages: followed, busy: false, onAnswerAsk: () => {} })
    const view = within(container)
    expect(view.queryByRole("button", { name: "Ordered in the last 30 days" })).toBeNull()
    expect(view.queryByRole("button", { name: "Ordered in the last 90 days" })).toBeNull()
    expect(container.textContent).toContain("Ordered in the last 90 days")
  })

  it("disables the option buttons while an answer is on its way", () => {
    const { container } = renderMessages({ messages: asking, busy: true, onAnswerAsk: () => {} })
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
    const { container } = renderMessages({ messages: broken, busy: false, onAnswerAsk: () => {} })
    expect(within(container).queryByRole("button", { name: "only one" })).toBeNull()
  })

  it("shows the notice line when the answer was not remembered", () => {
    const { container } = renderMessages({
      messages: asking, busy: false, onAnswerAsk: () => {}, askNotice: "Your answer was not remembered.",
    })
    expect(within(container).getByText("Your answer was not remembered.")).toBeDefined()
  })
})

describe("the chat follows the shown text while it arrives (AI-24 D7)", () => {
  function ScrollProbe({ tail, instant }: { tail: string; instant: boolean }) {
    const ref = React.useRef<HTMLDivElement>(null)
    useStickToBottom(ref, tail, instant)
    return <div ref={ref} />
  }

  it("scrolls instantly while text arrives, and smoothly only for a new message", () => {
    const scroller = document.createElement("div")
    scroller.style.overflowY = "auto"
    document.body.appendChild(scroller)
    const behaviours: ScrollBehavior[] = []
    Object.defineProperty(scroller, "scrollTo", {
      configurable: true,
      writable: true,
      value: (options: ScrollToOptions) => {
        behaviours.push(options.behavior ?? "auto")
      },
    })

    const view = render(<ScrollProbe tail="0" instant />, { container: scroller })
    behaviours.length = 0
    // One tail change per revealed frame while the answer arrives.
    view.rerender(<ScrollProbe tail="1" instant />)
    view.rerender(<ScrollProbe tail="2" instant />)
    expect(behaviours).toEqual(["auto", "auto"])

    // A new message (or jump to latest) still glides.
    view.rerender(<ScrollProbe tail="3" instant={false} />)
    expect(behaviours.at(-1)).toBe("smooth")

    view.unmount()
    scroller.remove()
  })
})

/** One answer that holds the given tool steps; the model wrote its text before any confirm. */
function draftReply(tools: NonNullable<Msg["tools"]>): Msg[] {
  return [
    { id: "u1", role: "user", content: "Add a chart", at: "2026-11-05T07:00:00Z" },
    {
      id: "a1",
      role: "assistant",
      content: "The draft is ready. Confirm it in the console to save it.",
      at: "2026-11-05T07:00:05Z",
      tools,
    },
  ]
}

const SAVED_CHART = {
  tool: "create_chart",
  args: { title: "Revenue by month" },
  ok: true,
  confirmed: true,
  result: { created: true, id: "c1", title: "Revenue by month" },
}

describe("ChatMessages status line after a confirm", () => {
  it("says what was saved, under the reply text", () => {
    const { container } = renderMessages({ messages: draftReply([SAVED_CHART]), busy: false })
    const text = container.textContent ?? ""
    expect(text).toContain("Saved: Revenue by month")
    // The model's sentence stays as written; the console's line follows it.
    expect(text.indexOf("Saved: Revenue by month")).toBeGreaterThan(text.indexOf("Confirm it in the console"))
  })

  it("says a confirmed step was not saved, with its error", () => {
    const failed = {
      tool: "create_chart",
      args: { title: "Revenue by month" },
      ok: false,
      confirmed: false,
      result: { error: "mart sales_daily does not exist" },
    }
    const { container } = renderMessages({ messages: draftReply([failed]), busy: false })
    expect(container.textContent).toContain("Not saved: mart sales_daily does not exist")
    expect(container.textContent).not.toContain("Saved: Revenue")
  })

  it("shows no status line for a draft still waiting for the person", () => {
    const pending = {
      tool: "create_board",
      args: { name: "Overview" },
      ok: true,
      result: { needs_confirmation: true, tool: "create_board", summary: "Create the board Overview." },
    }
    const { container } = renderMessages({ messages: draftReply([pending]), busy: false })
    expect(container.textContent).toContain("Create the board Overview.")
    expect(container.textContent).not.toContain("Saved:")
    expect(container.textContent).not.toContain("Not saved")
  })

  it("shows no status line for a cancelled draft", () => {
    const cancelled = {
      tool: "create_chart",
      args: { title: "Revenue by month" },
      ok: false,
      result: { cancelled: true },
    }
    const { container } = renderMessages({ messages: draftReply([cancelled]), busy: false })
    expect(container.textContent).not.toContain("Saved:")
    expect(container.textContent).not.toContain("Not saved")
  })

  it("gives one line per confirmed step, in step order", () => {
    const board = {
      tool: "create_board",
      args: { name: "Overview" },
      ok: true,
      confirmed: true,
      result: { created: true, id: "b1", name: "Overview" },
    }
    const { container } = renderMessages({ messages: draftReply([board, SAVED_CHART]), busy: false })
    const lines = Array.from(container.querySelectorAll('[role="status"]')).map((el) => el.textContent)
    expect(lines).toEqual(["Saved: Overview", "Saved: Revenue by month"])
  })

  it("falls back to the tool's label when the result carries no title", () => {
    const bare = { tool: "rename_board", args: {}, ok: true, confirmed: true, result: { updated: true } }
    const { container } = renderMessages({ messages: draftReply([bare]), busy: false })
    expect(container.textContent).toContain("Saved: Rename board")
  })

  it("keeps the line out of the history the console sends next", () => {
    const messages = draftReply([SAVED_CHART])
    expect(chatHistory(messages)).toEqual([
      { role: "user", content: "Add a chart" },
      { role: "assistant", content: "The draft is ready. Confirm it in the console to save it." },
    ])
  })
})
