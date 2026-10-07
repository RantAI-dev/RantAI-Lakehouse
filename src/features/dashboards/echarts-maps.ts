import * as echarts from "echarts";
import type { RegionLevel } from "@/lib/geo-regions";

/**
 * The maps a chart can be drawn on, registered with ECharts from LOCAL
 * GeoJSON (bundled under public/geo, see public/geo/README.md for where each
 * file comes from), with no call to an external tile or boundary server —
 * consistent with the self-host ethos and safe for embed/offline use.
 *
 * The console owns this catalogue. The API stores only a map id for a chart
 * and checks its shape; an id that is not listed here renders an honest "map
 * not available" state (GeoChart), so a new map needs no API change.
 */
export type MapEntry = {
  id: string;
  label: string;
  url: string;
  /** What one feature of the map is; picks the region-name matching rules. */
  level: RegionLevel;
};

export const MAP_CATALOGUE: readonly MapEntry[] = [
  { id: "dki-jakarta", label: "Jakarta — cities", url: "/geo/dki-jakarta.geojson", level: "city" },
  { id: "id-provinces", label: "Indonesia — provinces", url: "/geo/id-provinces.geojson", level: "province" },
  { id: "id-regencies", label: "Indonesia — kabupaten/kota", url: "/geo/id-regencies.geojson", level: "regency" },
];

/** A `geomap` stored before maps were selectable has no `map`: it was Jakarta. */
export const DEFAULT_CHOROPLETH_MAP = "dki-jakarta";
/** The outline a point map starts with. */
export const DEFAULT_POINT_MAP = "id-provinces";

export function mapEntry(id: string): MapEntry | undefined {
  return MAP_CATALOGUE.find((m) => m.id === id);
}

/**
 * The attribution the boundary licence (CC BY 3.0 IGO) requires, for the
 * maps that carry it; the Jakarta file's origin was not recorded
 * (public/geo/README.md), so it gets no credit rather than a made-up one.
 */
export function mapCredit(id: string): string | undefined {
  return id === "id-provinces" || id === "id-regencies"
    ? "Boundaries: BPS, OCHA via geoBoundaries (CC BY 3.0 IGO)"
    : undefined;
}

/** Feature names of the maps registered so far, for matching region names. */
const featureNames = new Map<string, string[]>();
const loading = new Map<string, Promise<boolean>>();

/** The `name` of every feature of a registered map (empty before `ensureMap` resolves true). */
export function mapFeatureNames(id: string): string[] {
  return featureNames.get(id) ?? [];
}

type FeatureCollection = { features?: { properties?: { name?: unknown } }[] };

/**
 * Load + register the map `id` once (idempotent). Resolves false for an id
 * that is not in the catalogue, or when its GeoJSON is missing or malformed.
 */
export function ensureMap(id: string): Promise<boolean> {
  const entry = mapEntry(id);
  if (!entry) return Promise.resolve(false);
  if (featureNames.has(id)) return Promise.resolve(true);
  let p = loading.get(id);
  if (!p) {
    // Deliberately NOT `apiFetch`: a static GeoJSON asset, not an `/api/*`
    // call — there is no auth state to react to.
    p = fetch(entry.url, { cache: "force-cache" })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error("geojson not found"))))
      .then((gj: FeatureCollection) => {
        const names = (gj.features ?? []).map((f) => String(f.properties?.name ?? "")).filter(Boolean);
        echarts.registerMap(id, gj as unknown as Parameters<typeof echarts.registerMap>[1]);
        featureNames.set(id, names);
        return true;
      })
      .catch(() => {
        // Forget the failure so the next render can try again (a deploy
        // that adds the file, a flaky first request).
        loading.delete(id);
        return false;
      });
    loading.set(id, p);
  }
  return p;
}
