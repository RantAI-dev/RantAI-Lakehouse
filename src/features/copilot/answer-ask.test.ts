import { describe, expect, it } from "bun:test"

import type { SaveChatTermInput } from "@/services/contracts/chat-terms"

import { answerAsk } from "./answer-ask"

const ARGS = { term: "active customer", option: "Ordered in the last 30 days", question: "Show active customers" }

describe("answerAsk", () => {
  it("stores the answer with the term, the option and the user's question, and only then sends the option", async () => {
    const order: string[] = []
    const stored: unknown[] = []
    const remembered = await answerAsk(
      {
        saveTerm: async (input) => {
          order.push("store")
          stored.push(input)
        },
        send: async (text) => {
          order.push(`send:${text}`)
        },
      },
      ARGS,
    )
    expect(stored).toEqual([
      { term: "active customer", meaning: "Ordered in the last 30 days", question: "Show active customers" },
    ])
    expect(order).toEqual(["store", "send:Ordered in the last 30 days"])
    expect(remembered).toBe(true)
  })

  it("still sends the option when the store fails, and reports that the answer was not remembered", async () => {
    const sent: string[] = []
    const remembered = await answerAsk(
      {
        saveTerm: async () => {
          throw new Error("down")
        },
        send: async (text) => {
          sent.push(text)
        },
      },
      ARGS,
    )
    expect(sent).toEqual(["Ordered in the last 30 days"])
    expect(remembered).toBe(false)
  })

  // PR review fix (SHOULD-FIX): the store refuses a question over 500 characters.
  const storedQuestion = async (question: string): Promise<string> => {
    const stored: SaveChatTermInput[] = []
    const remembered = await answerAsk(
      {
        saveTerm: async (input) => {
          stored.push(input)
        },
        send: async () => {},
      },
      { ...ARGS, question },
    )
    expect(remembered).toBe(true)
    return stored[0].question ?? ""
  }

  it("cuts a long question to the store's 500-character limit before saving it", async () => {
    const sent = await storedQuestion(`  ${"a".repeat(600)}  `)
    expect(sent).toBe("a".repeat(500))
  })

  it("counts characters, not UTF-16 units, so an emoji is never split in half", async () => {
    // 400 emoji are 800 UTF-16 units but 400 characters: all of them stay.
    expect(Array.from(await storedQuestion("😀".repeat(400)))).toHaveLength(400)
    const cut = await storedQuestion("😀".repeat(600))
    expect(Array.from(cut)).toHaveLength(500)
    expect(cut).toBe("😀".repeat(500))
  })
})
