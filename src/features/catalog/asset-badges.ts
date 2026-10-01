import { STORAGE_TIER_DESCRIPTION } from "@/lib/status"
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

/** Why the asset is on the tier it is on — where its data physically sits. */
export function tierTitle(a: Pick<Asset, "tier" | "type">): string {
  if (a.type === "iceberg-table") return "Warm: an Iceberg table on object storage."
  if (a.tier === "hot") return "Hot: its data sits in ClickHouse."
  if (a.type === "view") {
    return "Warm: a view. It stores nothing of its own and reads the Bronze data."
  }
  return STORAGE_TIER_DESCRIPTION[a.tier]
}
