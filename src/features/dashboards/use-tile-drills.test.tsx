import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { useTileDrills } from "./tile-drills"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
const GEO = { id: "h_1", sourceKind: "mart", sourceId: "m", name: "Geo", levels: ["provinsi", "kab_kota", "place"], createdBy: "" }
const chart = (def: Record<string, unknown>) => ({ id: "c_1", title: "t", kind: "bar", mart: "m", x: "provinsi", y: "visitors", source: "ui", def: { kind: "bar", dimension: "provinsi", ...def } }) as never

function stub() {
  const previews: unknown[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    if (url.includes("/api/dashboard/hierarchies")) return json({ hierarchies: [GEO] })
    previews.push(JSON.parse(String(init?.body)))
    return json({ spec: {}, result: { columns: ["kab_kota", "visitors"], rows: [{ kab_kota: "Bandung", visitors: 5 }] } })
  }) as unknown as typeof fetch
  return previews
}

describe("useTileDrills", () => {
  it("offers the next level once the source's hierarchies are known, and asks the server for it with the saved chart's id", async () => {
    const previews = stub()
    const { result } = renderHook(() => useTileDrills([chart({})], [], true))
    await waitFor(() => expect(result.current.offer(chart({}))).toEqual({ to: "kab_kota", hierarchy: GEO }))
    act(() => result.current.down(chart({}), GEO, "Jawa Barat"))
    await waitFor(() => expect(result.current.drills.c_1?.cell).toBeTruthy())
    expect(previews).toEqual([{ drillOf: "c_1", to: "kab_kota", fix: [{ column: "provinsi", value: "Jawa Barat" }], filters: [] }])
    expect(result.current.currentLevel(chart({}))).toBe("kab_kota")
    expect(result.current.fixed(chart({}))).toEqual([{ column: "provinsi", values: ["Jawa Barat"], op: "in" }])
    // One more level, then no more.
    expect(result.current.offer(chart({}))).toEqual({ to: "place", hierarchy: GEO })
    act(() => result.current.down(chart({}), GEO, "Kota Bandung"))
    await waitFor(() => expect(result.current.drills.c_1?.steps).toHaveLength(2))
    await waitFor(() => expect(result.current.offer(chart({}))).toEqual({ reason: "this is the last level of its hierarchy" }))
  })

  it("climbs back to the whole chart and forgets the drill", async () => {
    stub()
    const { result } = renderHook(() => useTileDrills([chart({})], [], true))
    await waitFor(() => expect(result.current.offer(chart({}))).toBeTruthy())
    act(() => result.current.down(chart({}), GEO, "Jawa Barat"))
    await waitFor(() => expect(result.current.drills.c_1?.cell).toBeTruthy())
    act(() => result.current.climb(chart({}), 0))
    await waitFor(() => expect(result.current.drills.c_1).toBeUndefined())
    expect(result.current.currentLevel(chart({}))).toBe("provinsi")
  })

  it("says why a chart cannot be drilled, and says nothing for a column in no hierarchy or a built-in tile", async () => {
    stub()
    const binned = chart({ bin: { mode: "auto" } })
    const { result } = renderHook(() => useTileDrills([binned, chart({ dimension: "category" })], [], true))
    await waitFor(() => expect(result.current.offer(binned)).toEqual({ reason: "a chart grouped in bins cannot be drilled down." }))
    expect(result.current.offer(chart({ dimension: "category" }))).toBeNull()
    expect(result.current.offer({ ...(chart({}) as object), source: "builtin" } as never)).toBeNull()
  })

  it("offers no drill when the list of hierarchies cannot be read", async () => {
    global.fetch = mock(async () => json({ error: "no" }, 500)) as unknown as typeof fetch
    const { result } = renderHook(() => useTileDrills([chart({})], [], true))
    await new Promise((r) => setTimeout(r, 30))
    expect(result.current.offer(chart({}))).toBeNull()
  })
})
