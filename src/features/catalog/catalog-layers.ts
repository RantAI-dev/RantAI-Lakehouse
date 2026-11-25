import type { DataLayer } from "@/lib/status"

/**
 * The Catalog's layer choices (`UX-8`), apart from the controls that draw
 * them so the table's Layer cell can use the same colours without pulling
 * the page frame in.
 */

/** A layer choice: the values of the table's `layer` filter it stands for. */
export type LayerChoice = { id: "all" | "bronze" | "silver" | "gold"; label: string; layers: DataLayer[]; dot?: string }

/**
 * The layer switch's choices. `raw` and `bronze` are one layer physically
 * (both are Iceberg tables in the `bronze` namespace; `lib/status.ts`), so
 * one Bronze choice stands for both. The API sends no other layer
 * (`routes/catalog.rs`: `raw`, `bronze`, `silver`, `gold`); the Filter menu
 * still offers every layer one by one, Bronze (raw) and Bronze (curated)
 * apart, for the rarer question.
 */
export const LAYER_CHOICES: readonly LayerChoice[] = [
  { id: "all", label: "All", layers: [] },
  { id: "bronze", label: "Bronze", layers: ["raw", "bronze"], dot: "bg-[#b9773d]" },
  { id: "silver", label: "Silver", layers: ["silver"], dot: "bg-slate-400 dark:bg-slate-300" },
  { id: "gold", label: "Gold", layers: ["gold"], dot: "bg-amber-400" },
]

/** The layer's dot colour, for the Layer cell, beside its name. */
export function layerDot(layer: string): string | undefined {
  return LAYER_CHOICES.find((choice) => choice.layers.includes(layer as DataLayer))?.dot
}

/**
 * Which choice the `layer` filter's values are: `all` when there is none,
 * the choice whose layers they are exactly (Bronze is raw and curated
 * together), and `null` for anything else the Filter menu set — Bronze
 * (raw) alone, or Silver and Gold — so no segment claims a filter it does
 * not stand for, and that filter keeps its chip.
 */
export function activeLayerChoice(values: readonly string[]): LayerChoice["id"] | null {
  if (values.length === 0) return "all"
  const set = new Set(values)
  const found = LAYER_CHOICES.find(
    (choice) =>
      choice.id !== "all" &&
      choice.layers.length === set.size &&
      choice.layers.every((layer) => set.has(layer))
  )
  return found?.id ?? null
}

/**
 * Whether a `filters` entry is one the Catalog's own controls show and edit
 * (`meta.isQuickFilter`, UX-8): a layer filter that is exactly one switch
 * choice, or one namespace by id, as the picker sets it. Its chip is left
 * out of the property bar; anything else the Filter menu set keeps one.
 */
export function isCatalogQuickFilter(filter: { id: string; value: unknown; operator: string }): boolean {
  if (filter.id === "namespace") {
    return filter.operator === "eq" && typeof filter.value === "string" && filter.value !== ""
  }
  if (filter.id === "layer") {
    if (filter.operator !== "inArray" || !Array.isArray(filter.value)) return false
    const choice = activeLayerChoice(filter.value.map(String))
    return choice !== null && choice !== "all"
  }
  return false
}
