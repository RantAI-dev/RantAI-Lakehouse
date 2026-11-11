import { act, cleanup, renderHook } from "@testing-library/react"
import type { Table } from "@tanstack/react-table"
import { NuqsTestingAdapter } from "nuqs/adapters/testing"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import type { ReactNode } from "react"
import { useDataTableFilters } from "./use-data-table-filters"

// QA LH-DATA-11-08, LH-DATA-12-03, LH-DATA-12-13: over plain http (staging
// opened by IP) `crypto.randomUUID` does not exist, and every option of the
// "+ Filter" menu threw. The hook must still make an 8-character `filterId`.

const realCrypto = Object.getOwnPropertyDescriptor(globalThis, "crypto")

beforeEach(() => {
  // An insecure context: `crypto` is there, `randomUUID` is not.
  Object.defineProperty(globalThis, "crypto", {
    value: { getRandomValues: (a: Uint8Array) => a.fill(7) },
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

describe("useDataTableFilters without crypto.randomUUID", () => {
  it("adds a filter with an 8-character filterId", async () => {
    const table = fakeTable()
    const { result } = renderHook(() => useDataTableFilters(table, { debounceMs: 0 }), { wrapper })
    let added: { filterId: string } | undefined
    act(() => {
      added = result.current.addColumnFilter(result.current.columns[0]) as { filterId: string }
    })
    expect(added?.filterId).toHaveLength(8)
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20))
    })
    expect(result.current.filters).toHaveLength(1)
    expect(result.current.filters[0].filterId).toHaveLength(8)
  })

  it("adds a rule through onFilterAdd with an 8-character filterId", async () => {
    const table = fakeTable()
    const { result } = renderHook(() => useDataTableFilters(table, { debounceMs: 0 }), { wrapper })
    act(() => {
      result.current.onFilterAdd()
    })
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50))
    })
    expect(result.current.filters).toHaveLength(1)
    expect(result.current.filters[0].filterId).toHaveLength(8)
  })
})
