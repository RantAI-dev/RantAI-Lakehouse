import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import type { Table } from "@tanstack/react-table"
import { NuqsTestingAdapter } from "nuqs/adapters/testing"
import type { ReactNode } from "react"

import { DataTableAdvancedFilterPanel } from "./data-table-add-filter"

// QA LH-DATA-11-08: over plain http (staging opened by IP) `crypto.randomUUID`
// does not exist. The panel seeds one rule and can duplicate it, and both ids
// must still be 8 characters long and different from each other.

const realCrypto = Object.getOwnPropertyDescriptor(globalThis, "crypto")

beforeEach(() => {
  // An insecure context: `crypto` is there, `randomUUID` is not. Every call gives
  // other bytes, so two ids cannot come out the same.
  let counter = 0
  Object.defineProperty(globalThis, "crypto", {
    value: {
      getRandomValues: (a: Uint8Array) => {
        for (let i = 0; i < a.length; i += 1) a[i] = (counter + i) % 256
        counter += 1
        return a
      },
    },
    configurable: true,
    writable: true,
  })
})

afterEach(() => {
  cleanup()
  if (realCrypto) Object.defineProperty(globalThis, "crypto", realCrypto)
})

type Row = { name: string }

function fakeTable(): Table<Row> {
  const column = {
    id: "name",
    columnDef: { enableColumnFilter: true, meta: { variant: "text" } },
  }
  return {
    getAllColumns: () => [column],
    options: { meta: {} },
  } as unknown as Table<Row>
}

const wrapper = ({ children }: { children: ReactNode }) => (
  <NuqsTestingAdapter>{children}</NuqsTestingAdapter>
)

/** The `filterId` of each rule: the panel puts it in the rule's element id. */
function filterIds(): string[] {
  return screen
    .getAllByRole("listitem")
    .map((item) => (item.id.match(/-filter-(.+)$/) ?? [])[1] ?? "")
}

describe("DataTableAdvancedFilterPanel without crypto.randomUUID", () => {
  it("seeds one rule with an 8-character filterId", async () => {
    render(<DataTableAdvancedFilterPanel table={fakeTable()} />, { wrapper })
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(1))
    expect(filterIds()[0]).toHaveLength(8)
  })

  it("duplicates a rule with its own 8-character filterId", async () => {
    render(<DataTableAdvancedFilterPanel table={fakeTable()} />, { wrapper })
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(1))
    // A base-ui menu opens on pointerdown (as the other tests of the console do).
    fireEvent.pointerDown(screen.getByRole("button", { name: "Filter rule actions" }), {
      button: 0,
      pointerType: "mouse",
    })
    fireEvent.click(await screen.findByRole("menuitem", { name: "Duplicate" }))
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(2))
    const ids = filterIds()
    expect(ids[0]).toHaveLength(8)
    expect(ids[1]).toHaveLength(8)
    // Two rules, two ids: the duplicate is not the rule it came from.
    expect(ids[0]).not.toBe(ids[1])
  })
})
