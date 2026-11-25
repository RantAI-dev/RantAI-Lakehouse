import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, describe, expect, it, mock } from "bun:test"

import * as realNavigation from "next/navigation"

const real = { ...realNavigation }
mock.module("next/navigation", () => ({ ...real, useSearchParams: () => new URLSearchParams() }))
afterAll(() => {
  mock.module("next/navigation", () => real)
})

import { LineagePage } from "./lineage-page"

const originalFetch = global.fetch
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

describe("the lineage focus field (UX-7)", () => {
  it("suggests the catalog's datasets from the console's own list, and keeps a name typed by hand", async () => {
    global.fetch = mock(async () => new Response(JSON.stringify([]), { status: 200, headers: { "Content-Type": "application/json" } })) as unknown as typeof fetch
    render(<LineagePage />)
    const field = screen.getByLabelText("Dataset or table") as HTMLInputElement
    // No `datalist` whatever the catalog answers: the suggestions are the console's.
    fireEvent.keyDown(field, { key: "ArrowDown" })
    expect(document.querySelector("datalist")).toBeNull()
    fireEvent.change(field, { target: { value: "serving.typed" } })
    await waitFor(() => expect(field.value).toBe("serving.typed"))
  })
})
