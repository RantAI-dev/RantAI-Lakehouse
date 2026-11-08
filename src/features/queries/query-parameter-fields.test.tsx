import { cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import * as React from "react"
import { replaceDefinition } from "@/lib/query-param-names"
import type { QueryParamDef, QueryParamValue } from "@/services/contracts/queries"
import { QueryParameterFields } from "./query-parameter-fields"

afterEach(cleanup)

type Values = Record<string, QueryParamValue>

/** Chooses an option of a base-ui Select the way a pointer does (see `format-section.test.tsx`). */
function choose(trigger: HTMLElement, option: string) {
  fireEvent.click(trigger)
  const target = screen.getByRole("option", { name: option })
  fireEvent.focus(target)
  fireEvent.click(target)
}

/** The fields over spies, for the tests that look at what one control reports. */
function mountOver(definitions: QueryParamDef[], values: Values = {}) {
  const onValueChange = mock<(name: string, value: QueryParamValue | undefined) => void>(() => {})
  const onDefinitionChange = mock<(def: QueryParamDef) => void>(() => {})
  render(
    <QueryParameterFields
      definitions={definitions}
      values={values}
      onValueChange={onValueChange}
      onDefinitionChange={onDefinitionChange}
    />
  )
  return { onValueChange, onDefinitionChange }
}

/**
 * The fields over real state, the way the studio holds it: a definition
 * change goes through `replaceDefinition`, which also clears a value that no
 * longer fits.
 */
function Harness({
  initial,
  initialValues = {},
  onState,
}: {
  initial: QueryParamDef[]
  initialValues?: Values
  onState?: (state: { parameters: QueryParamDef[]; values: Values }) => void
}) {
  const [state, setState] = React.useState({ parameters: initial, values: initialValues })
  React.useEffect(() => onState?.(state), [state, onState])
  return (
    <QueryParameterFields
      definitions={state.parameters}
      values={state.values}
      onValueChange={(name, value) =>
        setState((previous) => {
          const next = { ...previous.values }
          if (value === undefined) delete next[name]
          else next[name] = value
          return { ...previous, values: next }
        })
      }
      onDefinitionChange={(def) =>
        setState((previous) => replaceDefinition(previous.parameters, previous.values, def))
      }
    />
  )
}

describe("QueryParameterFields", () => {
  it("renders nothing for a query with no parameter", () => {
    const { container } = render(
      <QueryParameterFields
        definitions={[]}
        values={{}}
        onValueChange={() => {}}
        onDefinitionChange={() => {}}
      />
    )
    expect(container.firstChild).toBeNull()
  })

  it("shows one labelled field per parameter, in the order given", () => {
    mountOver([
      { name: "region", type: "text" },
      { name: "limit", type: "number" },
    ])
    const labels = screen.getAllByText(/^(region|limit)$/).map((node) => node.textContent)
    expect(labels).toEqual(["region", "limit"])
    expect(screen.getByLabelText("region")).toBeTruthy()
    expect(screen.getByLabelText("limit")).toBeTruthy()
  })

  it("sets a text value as typed, and no value when the field is emptied", () => {
    const { onValueChange } = mountOver([{ name: "region", type: "text" }])
    fireEvent.change(screen.getByLabelText("region"), { target: { value: "EU" } })
    expect(onValueChange).toHaveBeenLastCalledWith("region", "EU")
    fireEvent.change(screen.getByLabelText("region"), { target: { value: "" } })
    expect(onValueChange).toHaveBeenLastCalledWith("region", undefined)
  })

  it("starts a field at its default when the tab has no value for it", () => {
    mountOver([{ name: "region", type: "text", default: "EU" }])
    expect((screen.getByLabelText("region") as HTMLInputElement).value).toBe("EU")
  })

  it("shows the tab's value over the default", () => {
    mountOver([{ name: "region", type: "text", default: "EU" }], { region: "US" })
    expect((screen.getByLabelText("region") as HTMLInputElement).value).toBe("US")
  })

  it("sets a number value as a number, and no value when the field is emptied", () => {
    const { onValueChange } = mountOver([{ name: "limit", type: "number" }])
    fireEvent.change(screen.getByLabelText("limit"), { target: { value: "12.5" } })
    expect(onValueChange).toHaveBeenLastCalledWith("limit", 12.5)
    fireEvent.change(screen.getByLabelText("limit"), { target: { value: "" } })
    expect(onValueChange).toHaveBeenLastCalledWith("limit", undefined)
  })

  it("sets a date value as YYYY-MM-DD", () => {
    const { onValueChange } = mountOver([{ name: "day", type: "date" }])
    const input = screen.getByLabelText("day") as HTMLInputElement
    expect(input.type).toBe("date")
    fireEvent.change(input, { target: { value: "2026-11-07" } })
    expect(onValueChange).toHaveBeenLastCalledWith("day", "2026-11-07")
  })

  it("sets a date range only when both ends are set, and no value otherwise", () => {
    const { onValueChange } = mountOver([{ name: "period", type: "dateRange" }])
    expect(screen.getByText("period")).toBeTruthy()
    fireEvent.change(screen.getByLabelText("Start"), { target: { value: "2026-11-01" } })
    expect(onValueChange).toHaveBeenLastCalledWith("period", undefined)
    fireEvent.change(screen.getByLabelText("End"), { target: { value: "2026-11-07" } })
    expect(onValueChange).toHaveBeenLastCalledWith("period", {
      start: "2026-11-01",
      end: "2026-11-07",
    })
  })

  it("shows the two ends of a range value", () => {
    mountOver([{ name: "period", type: "dateRange" }], {
      period: { start: "2026-11-01", end: "2026-11-07" },
    })
    expect((screen.getByLabelText("Start") as HTMLInputElement).value).toBe("2026-11-01")
    expect((screen.getByLabelText("End") as HTMLInputElement).value).toBe("2026-11-07")
  })

  it("picks a dropdown option and reports its value, a number staying a number", () => {
    const { onValueChange } = mountOver([
      {
        name: "region",
        type: "dropdown",
        options: [
          { value: "eu", label: "Europe" },
          { value: "us", label: "United States" },
        ],
      },
      {
        name: "tier",
        type: "dropdown",
        options: [
          { value: 1, label: "Gold" },
          { value: 2, label: "Silver" },
        ],
      },
    ])
    choose(screen.getByLabelText("region"), "United States")
    expect(onValueChange).toHaveBeenLastCalledWith("region", "us")
    choose(screen.getByLabelText("tier"), "Silver")
    expect(onValueChange).toHaveBeenLastCalledWith("tier", 2)
  })

  it("shows the label of a dropdown's default", () => {
    mountOver([
      {
        name: "region",
        type: "dropdown",
        default: "eu",
        options: [{ value: "eu", label: "Europe" }],
      },
    ])
    expect(within(screen.getByLabelText("region")).getByText("Europe")).toBeTruthy()
  })

  it("picks several options of a multi-select and reports them as a list", () => {
    const { onValueChange } = mountOver([
      {
        name: "regions",
        type: "multiSelect",
        options: [
          { value: "eu", label: "Europe" },
          { value: "us", label: "United States" },
        ],
      },
    ])
    fireEvent.click(screen.getByLabelText("regions"))
    fireEvent.click(screen.getByRole("option", { name: /Europe/ }))
    expect(onValueChange).toHaveBeenLastCalledWith("regions", ["eu"])
  })

  it("adds to a multi-select value, and reports no value when the last pick is removed", () => {
    const { onValueChange } = mountOver(
      [
        {
          name: "regions",
          type: "multiSelect",
          options: [
            { value: 1, label: "One" },
            { value: 2, label: "Two" },
          ],
        },
      ],
      { regions: [1] }
    )
    fireEvent.click(screen.getByLabelText("regions"))
    fireEvent.click(screen.getByRole("option", { name: /Two/ }))
    expect(onValueChange).toHaveBeenLastCalledWith("regions", [1, 2])
    fireEvent.click(screen.getByRole("option", { name: /One/ }))
    expect(onValueChange).toHaveBeenLastCalledWith("regions", undefined)
  })
})

describe("QueryParameterFields definition menu", () => {
  it("opens from a button that names the parameter", () => {
    mountOver([{ name: "region", type: "text" }])
    fireEvent.click(screen.getByRole("button", { name: "Edit definition of region" }))
    expect(screen.getByLabelText("Type")).toBeTruthy()
  })

  it("changes the type, and clears a value that no longer fits it", () => {
    let latest: { parameters: QueryParamDef[]; values: Values } | null = null
    render(
      <Harness
        initial={[{ name: "region", type: "text" }]}
        initialValues={{ region: "EU" }}
        onState={(state) => {
          latest = state
        }}
      />
    )
    fireEvent.click(screen.getByRole("button", { name: "Edit definition of region" }))
    choose(screen.getByLabelText("Type"), "Number")
    expect(latest!.parameters).toEqual([{ name: "region", type: "number" }])
    expect(latest!.values).toEqual({})
  })

  it("keeps a value that still fits the new type", () => {
    let latest: { parameters: QueryParamDef[]; values: Values } | null = null
    render(
      <Harness
        initial={[{ name: "tier", type: "dropdown", options: [{ value: "a", label: "A" }] }]}
        initialValues={{ tier: "a" }}
        onState={(state) => {
          latest = state
        }}
      />
    )
    fireEvent.click(screen.getByRole("button", { name: "Edit definition of tier" }))
    choose(screen.getByLabelText("Type"), "Text")
    expect(latest!.values).toEqual({ tier: "a" })
  })

  it("sets a default of the parameter's type", () => {
    const { onDefinitionChange } = mountOver([{ name: "limit", type: "number" }])
    fireEvent.click(screen.getByRole("button", { name: "Edit definition of limit" }))
    fireEvent.change(screen.getByLabelText("Default"), { target: { value: "50" } })
    expect(onDefinitionChange).toHaveBeenLastCalledWith({ name: "limit", type: "number", default: 50 })
  })

  it("drops the options and the default when the type is no longer a list", () => {
    let latest: { parameters: QueryParamDef[]; values: Values } | null = null
    render(
      <Harness
        initial={[
          {
            name: "region",
            type: "dropdown",
            default: "eu",
            options: [{ value: "eu", label: "Europe" }],
          },
        ]}
        onState={(state) => {
          latest = state
        }}
      />
    )
    fireEvent.click(screen.getByRole("button", { name: "Edit definition of region" }))
    choose(screen.getByLabelText("Type"), "Number")
    expect(latest!.parameters).toEqual([{ name: "region", type: "number" }])
  })

  it("adds, edits and removes an option of a dropdown", () => {
    let latest: { parameters: QueryParamDef[]; values: Values } | null = null
    render(
      <Harness
        initial={[{ name: "region", type: "dropdown" }]}
        onState={(state) => {
          latest = state
        }}
      />
    )
    fireEvent.click(screen.getByRole("button", { name: "Edit definition of region" }))
    fireEvent.click(screen.getByRole("button", { name: "Add option" }))
    fireEvent.change(screen.getByLabelText("Option 1 label"), { target: { value: "Europe" } })
    fireEvent.change(screen.getByLabelText("Option 1 value"), { target: { value: "eu" } })
    expect(latest!.parameters[0]?.options).toEqual([{ value: "eu", label: "Europe" }])
    fireEvent.click(screen.getByRole("button", { name: "Remove option 1" }))
    expect(latest!.parameters[0]?.options).toBeUndefined()
  })

  it("stops adding options at 200", () => {
    const options = Array.from({ length: 200 }, (_, index) => ({
      value: `v${index}`,
      label: `L${index}`,
    }))
    mountOver([{ name: "region", type: "dropdown", options }])
    fireEvent.click(screen.getByRole("button", { name: "Edit definition of region" }))
    expect((screen.getByRole("button", { name: "Add option" }) as HTMLButtonElement).disabled).toBe(
      true
    )
  })
})
