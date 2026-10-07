import { STORAGE_TIER_DESCRIPTION, type DataLayer } from "@/lib/status"
import type { Asset } from "@/services/contracts/assets"

/** What an `unknown` health means: there is nothing to judge it by. */
export const NO_HEALTH_SIGNAL =
  "Nothing measures this asset yet: it has no freshness target and no quality check has run."

/** Why the asset has the health it has, one signal per line. */
export function healthTitle(a: Pick<Asset, "healthReasons">): string {
  const reasons = a.healthReasons ?? []
  return reasons.length > 0 ? reasons.join("\n") : NO_HEALTH_SIGNAL
}

/** Where the asset's classification comes from. */
export function classificationTitle(a: Pick<Asset, "classificationSource">): string {
  return a.classificationSource === "rule"
    ? "Set by a classification rule on this asset or one of its columns."
    : "The default level: no classification rule names this asset."
}

const LAYER_TITLE: Record<DataLayer, string> = {
  raw: "Bronze, as loaded: an append-only copy of its source, kept as an Iceberg table.",
  bronze:
    "Bronze that this deployment lists as curated: an append-only copy of its source, kept as an Iceberg table.",
  silver: "Silver: cleaned and conformed data, in ClickHouse.",
  gold: "Gold: a mart built for dashboards and reports, in ClickHouse.",
  semantic: "Semantic: business definitions over the marts.",
}

/** What the asset's layer means, and how raw differs from curated Bronze. */
export function layerTitle(a: Pick<Asset, "layer">): string {
  return LAYER_TITLE[a.layer]
}

/** Why the asset is on the tier it is on — where its data physically sits. */
export function tierTitle(a: Pick<Asset, "tier" | "type">): string {
  if (a.type === "iceberg-table") return "Warm: an Iceberg table on object storage."
  if (a.tier === "hot") return "Hot: its data sits in ClickHouse."
  if (a.type === "view") {
    return "Warm: a view. It stores nothing of its own and reads other tables when queried."
  }
  return STORAGE_TIER_DESCRIPTION[a.tier]
}
