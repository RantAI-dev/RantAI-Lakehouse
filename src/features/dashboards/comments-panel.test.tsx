import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { CommentsPanel, insertMention, mentionedIds, pendingMention } from "./comments-panel"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

type Call = { url: string; method: string; body: unknown }

const COMMENT = {
  id: "c1",
  board: "b_1",
  author: { id: "u1", name: "Ann" },
  body: "<img src=x onerror=alert(1)> **not bold**",
  createdAt: "2030-01-01T10:00:00.000Z",
  editedAt: null,
  mentions: [],
  canEdit: true,
  canDelete: true,
}

function stub(
  comments: unknown[],
  onWrite: (c: Call) => Response = () => json({ ok: true }),
  truncated = false,
): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call = {
      url: String(input),
      method: init?.method ?? "GET",
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    }
    calls.push(call)
    if (call.url.includes("/comments/people")) return json({ items: [{ id: "u2", name: "Bob" }] })
    if (call.method === "GET") return json({ comments, truncated })
    return onWrite(call)
  }) as unknown as typeof fetch
  return calls
}

describe("mention helpers", () => {
  it("sees the @word being typed at the end only", () => {
    expect(pendingMention("hi @bo")).toBe("bo")
    expect(pendingMention("@")).toBe("")
    expect(pendingMention("hi @bob there")).toBeNull()
    expect(pendingMention("mail a@b.c")).toBeNull()
  })
  it("puts the chosen name where the word was", () => {
    expect(insertMention("hi @bo", { id: "u2", name: "Bob" })).toBe("hi @Bob ")
  })
  it("sends only picked people whose name is still in the text, once", () => {
    const bob = { id: "u2", name: "Bob" }
    const cy = { id: "u3", name: "Cy" }
    expect(mentionedIds("hi @Bob and @Bob", [bob, cy])).toEqual(["u2"])
    expect(mentionedIds("hi Bob", [bob])).toEqual([])
  })
})

describe("CommentsPanel", () => {
  it("shows a comment's text as text, never as markup", async () => {
    stub([COMMENT])
    const { container } = render(<CommentsPanel board="b_1" />)
    expect(await screen.findByText(COMMENT.body)).toBeTruthy()
    expect(container.querySelector("img")).toBeNull()
    expect(container.querySelector("strong")).toBeNull()
  })

  it("offers Edit and Delete only where the server allowed them", async () => {
    stub([{ ...COMMENT, canEdit: false, canDelete: false }])
    render(<CommentsPanel board="b_1" />)
    await screen.findByText(COMMENT.body)
    expect(screen.queryByText("Edit")).toBeNull()
    expect(screen.queryByText("Delete")).toBeNull()
  })

  it("posts the text with the person picked from the @ list", async () => {
    const calls = stub([], () => json({ ...COMMENT, body: "hi @Bob" }))
    render(<CommentsPanel board="b_1" />)
    const box = (await screen.findByLabelText("Write a comment")) as HTMLTextAreaElement
    fireEvent.change(box, { target: { value: "hi @bo" } })
    fireEvent.click(await screen.findByText("Bob"))
    expect(box.value).toBe("hi @Bob ")
    fireEvent.click(screen.getByText("Post"))
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true))
    const post = calls.find((c) => c.method === "POST")!
    expect(post.body).toEqual({ board: "b_1", body: "hi @Bob", mentions: ["u2"] })
    expect(calls.find((c) => c.url.includes("/people"))!.url).toContain("board=b_1")
  })

  it("shows the server's sentence when a mention is refused", async () => {
    stub([], () => json({ error: "One of the people mentioned cannot see this dashboard." }, 400))
    render(<CommentsPanel board="b_1" />)
    const box = await screen.findByLabelText("Write a comment")
    fireEvent.change(box, { target: { value: "hello" } })
    fireEvent.click(screen.getByText("Post"))
    expect(await screen.findByText("One of the people mentioned cannot see this dashboard.")).toBeTruthy()
  })

  it("will not post an empty or over-long comment", async () => {
    stub([])
    render(<CommentsPanel board="b_1" />)
    const box = await screen.findByLabelText("Write a comment")
    const post = screen.getByText("Post") as HTMLButtonElement
    expect(post.disabled).toBe(true)
    fireEvent.change(box, { target: { value: "x".repeat(2001) } })
    expect(post.disabled).toBe(true)
    fireEvent.change(box, { target: { value: "x".repeat(2000) } })
    expect(post.disabled).toBe(false)
  })

  it("asks the server once, after typing pauses (BI-19 E review E-3)", async () => {
    const calls = stub([])
    render(<CommentsPanel board="b_1" />)
    const box = await screen.findByLabelText("Write a comment")
    for (const value of ["@b", "@bo", "@bob"]) fireEvent.change(box, { target: { value } })
    expect(calls.filter((c) => c.url.includes("/people"))).toHaveLength(0)
    await waitFor(() => expect(calls.filter((c) => c.url.includes("/people"))).toHaveLength(1))
    expect(calls.find((c) => c.url.includes("/people"))!.url).toContain("q=bob")
  })

  it("lists who will be notified once a person is picked (BI-19 QA-4)", async () => {
    stub([])
    render(<CommentsPanel board="b_1" />)
    const box = (await screen.findByLabelText("Write a comment")) as HTMLTextAreaElement
    expect(screen.queryByText(/Will notify/)).toBeNull()
    fireEvent.change(box, { target: { value: "hi @bo" } })
    fireEvent.click(await screen.findByText("Bob"))
    expect(screen.getByText("Will notify: Bob")).toBeTruthy()
    // Deleting the name from the text drops the mention, and the line with it.
    fireEvent.change(box, { target: { value: "hi" } })
    expect(screen.queryByText(/Will notify/)).toBeNull()
  })

  it("says nobody matches an @word only after the lookup finished empty (BI-19 QA-4)", async () => {
    global.fetch = mock(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes("/comments/people")) return json({ items: [] })
      return json({ comments: [], truncated: false })
    }) as unknown as typeof fetch
    render(<CommentsPanel board="b_1" />)
    const box = await screen.findByLabelText("Write a comment")
    fireEvent.change(box, { target: { value: "hi @zed" } })
    // The debounced request has not run yet: no claim.
    expect(screen.queryByText(/Nobody who can see/)).toBeNull()
    expect(await screen.findByText("Nobody who can see this dashboard matches “zed”.")).toBeTruthy()
  })

  it("says when older comments are not shown (BI-19 E review E-6c)", async () => {
    stub([COMMENT], undefined, true)
    render(<CommentsPanel board="b_1" />)
    expect(await screen.findByText("Older comments are not shown.")).toBeTruthy()
  })

  it("does not say it when the list is complete", async () => {
    stub([COMMENT])
    render(<CommentsPanel board="b_1" />)
    await screen.findByText(COMMENT.body)
    expect(screen.queryByText("Older comments are not shown.")).toBeNull()
  })

  it("deletes through the service", async () => {
    const calls = stub([COMMENT])
    render(<CommentsPanel board="b_1" />)
    fireEvent.click(await screen.findByText("Delete"))
    await waitFor(() => expect(calls.some((c) => c.method === "DELETE")).toBe(true))
    expect(calls.find((c) => c.method === "DELETE")!.url).toContain("/api/dashboard/comments/c1")
  })
})
