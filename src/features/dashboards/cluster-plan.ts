import { kmeans, type KMeansRefusal, type KMeansResult } from "@/lib/kmeans";
import type { ClusterSetting } from "@/lib/chart-style";

/**
 * Clusters of the points a scatter or bubble chart shows (BI-17 part B, plan
 * `2026-10-24-bi-17b-forecast-clustering.md`, D5). The chart asks for the
 * grouping to colour the points and the tile asks for the sentence under the
 * chart; both get the same answer, computed once per set of points.
 */
export type ClusterPlan = {
  result: KMeansResult | KMeansRefusal;
  /** The line under the chart: the points used and how k was chosen, or the refusal. */
  note: string;
};

const cache = new Map<string, ClusterPlan>();

export function planClusters(points: readonly (readonly number[])[], setting: ClusterSetting): ClusterPlan {
  const key = `${setting.k}|${points.map((p) => p.join(",")).join(";")}`;
  const hit = cache.get(key);
  if (hit) return hit;
  const result = kmeans({ points, k: setting.k });
  const plan: ClusterPlan = "refused" in result
    ? { result, note: result.refused }
    : {
        result,
        note: `Clusters of the ${(points.length - result.skipped).toLocaleString("en-US")} points shown${result.auto ? `; ${result.k} groups chosen automatically` : ""}${result.skipped ? `; ${result.skipped} without a usable value left out` : ""}.`,
      };
  if (cache.size > 24) cache.clear();
  cache.set(key, plan);
  return plan;
}
