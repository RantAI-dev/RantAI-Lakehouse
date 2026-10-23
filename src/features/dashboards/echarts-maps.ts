import * as echarts from "echarts";
import type { RegionAliases, RegionLevel } from "@/lib/geo-regions";
import { COUNTRY_MAPS } from "./country-maps";

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

/** The id of the first-level regions map of the country with ISO 3166-1 alpha-3 code `iso3`. */
export const countryMapId = (iso3: string) => `adm1-${iso3.toLowerCase()}`;

/**
 * BI-16 C2a (decision D4): the world by country and, per country, its states
 * or provinces from Natural Earth (public/geo/README.md). Indonesia has no
 * entry here: its provinces stay `id-provinces`, the default for `IDN`.
 */
const NATURAL_EARTH_MAPS: readonly MapEntry[] = [
  { id: "world", label: "World — countries", url: "/geo/world.geojson", level: "country" },
  ...COUNTRY_MAPS.map(([iso3, country]): MapEntry => ({
    id: countryMapId(iso3),
    label: `${country} — states/provinces`,
    url: `/geo/admin1/${iso3}.geojson`,
    level: "subdivision",
  })),
];

export const MAP_CATALOGUE: readonly MapEntry[] = [
  { id: "dki-jakarta", label: "Jakarta — cities", url: "/geo/dki-jakarta.geojson", level: "city" },
  { id: "id-provinces", label: "Indonesia — provinces", url: "/geo/id-provinces.geojson", level: "province" },
  { id: "id-regencies", label: "Indonesia — kabupaten/kota", url: "/geo/id-regencies.geojson", level: "regency" },
  ...NATURAL_EARTH_MAPS,
];

const byId = new Map(MAP_CATALOGUE.map((m) => [m.id, m]));

/** A `geomap` stored before maps were selectable has no `map`: it was Jakarta. */
export const DEFAULT_CHOROPLETH_MAP = "dki-jakarta";
/** The outline a point map starts with. */
export const DEFAULT_POINT_MAP = "id-provinces";

export function mapEntry(id: string): MapEntry | undefined {
  return byId.get(id);
}

/**
 * The attribution the boundary licence (CC BY 3.0 IGO) requires, for the
 * maps that carry it; the Jakarta file's origin was not recorded
 * (public/geo/README.md), so it gets no credit rather than a made-up one.
 */
export function mapCredit(id: string): string | undefined {
  if (id === "id-provinces" || id === "id-regencies") return "Boundaries: BPS, OCHA via geoBoundaries (CC BY 3.0 IGO)";
  // Public domain asks for no credit; naming the source is a courtesy.
  return NATURAL_EARTH_MAPS.some((m) => m.id === id) ? "Boundaries: Natural Earth (public domain)" : undefined;
}

/** Feature names of the maps registered so far, for matching region names. */
const featureNames = new Map<string, string[]>();
/** Other spellings of the features (ISO codes, long names), for the maps that carry them. */
const featureAliases = new Map<string, [string, string][]>();
const loading = new Map<string, Promise<boolean>>();

/** The `name` of every feature of a registered map (empty before `ensureMap` resolves true). */
export function mapFeatureNames(id: string): string[] {
  return featureNames.get(id) ?? [];
}

/** The alias pairs `[alias, feature name]` of a registered map (empty when it has none). */
export function mapFeatureAliases(id: string): RegionAliases {
  return featureAliases.get(id) ?? [];
}

type FeatureCollection = { features?: { properties?: Record<string, unknown> }[] };

/** Properties of the Natural Earth files that name a feature another way (BI-16 C2a). */
const ALIAS_PROPERTIES = ["name_long", "ISO_A2", "ISO_A3", "iso_3166_2"] as const;

function aliasesOf(gj: FeatureCollection): [string, string][] {
  const out: [string, string][] = [];
  for (const f of gj.features ?? []) {
    const name = String(f.properties?.name ?? "");
    if (!name) continue;
    for (const key of ALIAS_PROPERTIES) {
      const v = f.properties?.[key];
      if (typeof v === "string" && v && v !== name) out.push([v, name]);
    }
  }
  return out;
}

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
        featureAliases.set(id, aliasesOf(gj));
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
