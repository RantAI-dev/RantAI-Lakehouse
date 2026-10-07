import { describe, expect, it } from "bun:test"

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
})
