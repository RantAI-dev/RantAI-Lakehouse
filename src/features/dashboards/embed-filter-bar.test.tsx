import * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { SurfaceThemeProvider } from "@/components/ui/surface-theme"
import { EmbedFilterBar } from "./embed-filter-bar"

const originalFetch = global.fetch
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

/** The embed's value list is portalled to <body>, outside the render container. */
function portaledList(): HTMLElement {
  const popup = document.body.querySelector('[data-slot="select-content"]')
  if (!popup) throw new Error("The embed filter's portaled list was not opened")
  return popup as HTMLElement
}

function serveValues(values: string[]) {
  global.fetch = mock(async () => new Response(JSON.stringify({ values }), {
    status: 200, headers: { "Content-Type": "application/json" },
  })) as unknown as typeof fetch
}

const FILTERS = [{ column: "Region" }]

/** Open the list and wait for the fetched values, then return the portaled list. */
async function openFilterList() {
  fireEvent.click(screen.getByRole("combobox", { name: "Region" }))
  await screen.findByRole("option", { name: "North" })
  return portaledList()
}

describe("WEB-10: the embed filter bar's dropdown follows its dashboard's theme", () => {
  it("copies theme-light onto its portaled list on a light embed", async () => {
    serveValues(["North", "South"])
    render(
      <SurfaceThemeProvider themeClass="theme-light">
        <EmbedFilterBar jwt="t" filters={FILTERS} onChange={() => {}} />
      </SurfaceThemeProvider>
    )
    const list = await openFilterList()
    expect(list.className.split(/\s+/)).toContain("theme-light")
  })

  it("keeps the console's theme outside a light surface", async () => {
    serveValues(["North", "South"])
    render(<EmbedFilterBar jwt="t" filters={FILTERS} onChange={() => {}} />)
    const list = await openFilterList()
    expect(list.className.split(/\s+/)).not.toContain("theme-light")
  })
})
