// DATA-23: the row menu leaves the page in two ways — a link to the Data
// Explorer and the clipboard — and both carry the namespace **id**, because
// that is the value the assets hold and the one the explorer's namespace
// column compares to. The display name stays on screen; it is not what a
// filter needs.
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { namespaceAssetsHref } from "@/lib/table-filter-link"
import type { CatalogNamespace } from "@/services/contracts/assets"
import { getCatalogNamespaceColumns } from "./catalog-namespace-columns"

const columns = getCatalogNamespaceColumns({ onSelect: () => {} })

/** A namespace whose display name differs from its id — the DATA-23 case. */
const NAMESPACE: CatalogNamespace = {
  id: "silver",
  name: "Silver Layer",
  description: "Curated tables.",
  assetCount: 12,
  owner: "Data Platform",
  residency: "id-jakarta",
  sourceEngine: "ClickHouse + Iceberg",
}

/** The row's action menu, as the table renders it. */
function actionsMenu() {
  const found = columns.find((c) => c.id === "actions")
  if (!found) throw new Error("no actions column")
  const renderCell = found.cell as (ctx: { row: { original: CatalogNamespace } }) => React.ReactNode
  return render(<>{renderCell({ row: { original: NAMESPACE } })}</>)
}

/** Opens the menu and hands back the item with that name. */
async function openItem(name: string) {
  actionsMenu()
  // The menu opens on the pointer press, not on the click that follows it.
  fireEvent.pointerDown(screen.getByRole("button", { name: "Namespace actions" }))
  return screen.findByRole("menuitem", { name })
}

afterEach(() => {
  cleanup()
})

describe("the namespace row menu", () => {
  it("links Browse assets to the namespace id, not to its display name", async () => {
    const item = await openItem("Browse assets")
    expect(item.getAttribute("href")).toBe(namespaceAssetsHref(NAMESPACE.id))
    // The display name is what the label showed, and what the link must not carry.
    expect(item.getAttribute("href")).not.toBe(namespaceAssetsHref(NAMESPACE.name))
    expect(item.getAttribute("href")).not.toContain("Silver")
  })

  it("copies the namespace id, not the display name", async () => {
    const clipboard = { writeText: mock(async (value: string) => void value) }
    const original = Object.getOwnPropertyDescriptor(navigator, "clipboard")
    Object.defineProperty(navigator, "clipboard", { value: clipboard, configurable: true })
    try {
      const item = await openItem("Copy namespace")
      fireEvent.click(item)
      expect(clipboard.writeText.mock.calls).toEqual([[NAMESPACE.id]])
    } finally {
      if (original) Object.defineProperty(navigator, "clipboard", original)
      else Reflect.deleteProperty(navigator, "clipboard")
    }
  })
})
