import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { JoinsSection, type JoinTableInfo } from "./joins-section"
import { emptyJoin, type JoinDraft } from "@/lib/chart-joins"

afterEach(cleanup)

const info: Record<string, JoinTableInfo> = {
  capacity: { columns: ["provinsi", "hotels", "rooms"], rightKey: ["provinsi"] },
  events: { columns: ["id", "provinsi"], rightKey: ["id"] },
}
const props = {
  baseMart: "visits", baseColumns: ["provinsi", "visitors"], marts: ["visits", "capacity", "events"],
  infoOf: (m: string) => info[m], disabled: false,
}

describe("JoinsSection", () => {
  it("adds a blank join, up to four, and removes one", () => {
    const onChange = mock<(next: JoinDraft[]) => void>(() => {})
    const { rerender } = render(<JoinsSection {...props} joins={[]} onChange={onChange} onPickTable={() => {}} />)
    fireEvent.click(screen.getByText("Join a table"))
    expect(onChange.mock.calls[0][0]).toEqual([emptyJoin()])
    const four = [emptyJoin(), emptyJoin(), emptyJoin(), emptyJoin()]
    rerender(<JoinsSection {...props} joins={four} onChange={onChange} onPickTable={() => {}} />)
    expect(screen.queryByText("Join a table")).toBeNull()
    fireEvent.click(screen.getByLabelText("Remove join 2"))
    expect(onChange.mock.calls[1][0]).toHaveLength(3)
  })

  it("shows the pairs of a picked table, adds and removes a pair, and no more than four", () => {
    const onChange = mock<(next: JoinDraft[]) => void>(() => {})
    const joins: JoinDraft[] = [{ mart: "capacity", type: "left", on: [{ left: "provinsi", op: "=", right: "provinsi" }] }]
    const { rerender } = render(<JoinsSection {...props} joins={joins} onChange={onChange} onPickTable={() => {}} />)
    expect(screen.getByLabelText("Join 1 pair 1 left")).toBeTruthy()
    expect((screen.getByLabelText("Remove join 1 pair 1") as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByText("Add a pair"))
    expect(onChange.mock.calls[0][0][0].on).toHaveLength(2)
    const four = { ...joins[0], on: Array.from({ length: 4 }, () => ({ left: "provinsi", op: "=", right: "provinsi" })) }
    rerender(<JoinsSection {...props} joins={[four]} onChange={onChange} onPickTable={() => {}} />)
    expect(screen.queryByText("Add a pair")).toBeNull()
    fireEvent.click(screen.getByLabelText("Remove join 1 pair 3"))
    expect(onChange.mock.calls[1][0][0].on).toHaveLength(3)
  })

  it("warns, without blocking, when the equality pairs do not cover the joined table's key", () => {
    const covered: JoinDraft[] = [{ mart: "capacity", type: "left", on: [{ left: "provinsi", op: "=", right: "provinsi" }] }]
    const { rerender } = render(<JoinsSection {...props} joins={covered} onChange={() => {}} onPickTable={() => {}} />)
    expect(screen.queryByRole("status")).toBeNull()
    const multiplying: JoinDraft[] = [{ mart: "events", type: "left", on: [{ left: "provinsi", op: "=", right: "provinsi" }] }]
    rerender(<JoinsSection {...props} joins={multiplying} onChange={() => {}} onPickTable={() => {}} />)
    expect(screen.getByRole("status").textContent).toMatch(/rows may multiply/)
  })

  it("shows nothing to match until a table is picked", () => {
    render(<JoinsSection {...props} joins={[emptyJoin()]} onChange={() => {}} onPickTable={() => {}} />)
    expect(screen.queryByLabelText("Join 1 pair 1 left")).toBeNull()
    expect(screen.getByLabelText("Join 1 table")).toBeTruthy()
  })
})
