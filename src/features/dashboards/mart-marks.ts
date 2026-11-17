import type { Asset } from "@/services/contracts/assets"
import type { ChartSourceChoice } from "@/lib/chart-source"

/**
 * A Gold mart as `GET /api/dashboard/fields` lists it (`DATA-12` part 3): its
 * name and row count, and, when someone marked it, the same certification
 * fields a catalog row carries, so the picker can show the mark and warn on a
 * deprecated one. Unmarked marts have none of the optional fields.
 */
export type Mart = { name: string; rows: number } & Pick<
  Asset,
  | "certification"
  | "certificationNote"
  | "replacementAssetId"
  | "replacementName"
  | "tags"
  | "governedTags"
>

/** The mart a data-source choice names, or `undefined` for a SQL source or no choice. */
export function selectedMart(marts: Mart[], choice: ChartSourceChoice | null): Mart | undefined {
  return choice?.kind === "mart" ? marts.find((m) => m.name === choice.name) : undefined
}
