// UX-9: the toolbar's menus and the command menu read one list of actions.
// These check the rules the list holds, without the page around it.
import { describe, expect, it } from "bun:test"
import { pickActions, studioActions, type StudioActionContext } from "./studio-actions"

/** A studio with one statement in a new tab; `over` changes a field. */
function studio(over: Record<string, unknown> = {}): StudioActionContext["studio"] {
  return {
    targetStatement: { text: "SELECT 1", from: 0, to: 8 },
    running: false,
    download: { busy: null },
    engine: "clickhouse",
    statements: [{ text: "SELECT 1", from: 0, to: 8 }],
    runnable: true,
    saveTarget: { mode: "create", label: "Save as new", savedId: null },
    tabs: [{ id: "tab-1" }],
    explain: { busy: false },
    tab: "sql",
    estimateSupported: true,
    estimatable: true,
    estimateAct: { status: "success" },
    ...over,
  } as unknown as StudioActionContext["studio"]
}

function context(over: Partial<StudioActionContext> = {}): StudioActionContext {
  const noop = () => {}
  return {
    studio: studio(),
    onSave: noop,
    onSaveAsNew: noop,
    onFormat: noop,
    onOpenFormatSettings: noop,
    onOpenTimeTravel: noop,
    onOpenShortcuts: noop,
    panelOpen: true,
    onShowPanel: noop,
    onHidePanel: noop,
    ...over,
  }
}

const ids = (ctx: StudioActionContext) => studioActions(ctx).map((action) => action.id)

describe("studioActions", () => {
  it("offers Save as new only beside a Save that changes a saved query", () => {
    expect(ids(context())).not.toContain("save-as-new")
    const fromSaved = context({
      studio: studio({ saveTarget: { mode: "update", label: "Save", savedId: "q1" } }),
    })
    expect(ids(fromSaved)).toContain("save-as-new")
  })

  it("offers Save as SQL source only when the page hands it a handler", () => {
    expect(ids(context())).not.toContain("save-source")
    expect(ids(context({ onSaveAsSource: () => {} }))).toContain("save-source")
  })

  it("is Stop instead of Run while a run or a download is going", () => {
    expect(ids(context({ studio: studio({ running: true }) }))).toContain("stop")
    expect(ids(context({ studio: studio({ download: { busy: "csv" } }) }))).toContain("stop")
    expect(ids(context({ studio: studio({ running: true }) }))).not.toContain("run")
  })

  it("disables the one-statement actions, with the reason, when there is no single statement", () => {
    const actions = studioActions(context({ studio: studio({ targetStatement: null, estimatable: false }) }))
    for (const action of pickActions(actions, ["save", "explain", "estimate"])) {
      expect(action.disabled).toBe(true)
      expect(action.reason).toBe("Select one statement")
    }
  })

  // PR #658 review 3: the guarantee that every ⋯ action is also in the
  // command menu is the page test's ("offers every ⋯ menu action in the
  // command menu too"), which reads both menus as the page draws them. Here
  // the list only has to hold the ids the page picks, in that order.
  it("holds every id the page's ⋯ menu picks, in the order it picks them", () => {
    const actions = studioActions(context({ onUnpin: () => {} }))
    const menu = ["format", "format-settings", "explain", "time-travel", "unpin", "shortcuts"] as const
    expect(pickActions(actions, menu).map((action) => action.id)).toEqual([...menu])
  })
})
