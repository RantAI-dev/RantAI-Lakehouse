import * as echarts from "echarts";
import type { RegionAliases, RegionLevel } from "@/lib/geo-regions";
import { apiFetch } from "@/services/http";
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
  /** An administrator's uploaded map (BI-16 C2b), read from `/api/maps/{id}/geojson`, not from `public/geo`. */
  uploaded?: boolean;
};

/** The id of an uploaded map: server-made, `m_` and 12 hex digits (BI-16 C2b, plan D5), so safe in a URL. */
const UPLOADED_ID = /^m_[0-9a-f]{12}$/;

/** The catalogue entry of an uploaded map; its label is the map's name once the list has loaded. */
export function uploadedEntry(id: string, label = id): MapEntry {
  return { id, label, url: `/api/maps/${id}/geojson`, level: "uploaded", uploaded: true };
}

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
  // An uploaded map needs no list to be drawn: its id says where its file is.
  return byId.get(id) ?? (UPLOADED_ID.test(id) ? uploadedEntry(id) : undefined);
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
 * How an uploaded map's boundary file is fetched.
 *
 * The console signs in and reads it through `apiFetch` (the map's own
 * route). A public link and an embed are anonymous: they pass their own
 * token-based loader, because `apiFetch`'s 401 sends the browser to the
 * login page — which an anonymous viewer has no use for
 * (`PR #246 review BLOCKER 1`).
 */
export type MapLoader = (id: string) => Promise<Response>;

/** The console's loader: the signed-in reader's own route. */
const consoleLoader: MapLoader = (id) => apiFetch(uploadedEntry(id).url);

/**
 * Load + register the map `id` once (idempotent). Resolves false for an id
 * that is not in the catalogue, or when its GeoJSON is missing or malformed.
 *
 * `load` reads an uploaded map's file; a bundled file is a static asset and
 * is never fetched through it.
 */
export function ensureMap(id: string, load: MapLoader = consoleLoader): Promise<boolean> {
  const entry = mapEntry(id);
  if (!entry) return Promise.resolve(false);
  if (featureNames.has(id)) return Promise.resolve(true);
  let p = loading.get(id);
  if (!p) {
    // A bundled file is a static asset: deliberately NOT `apiFetch` (no auth
    // state to react to). An uploaded map is read through the loader the
    // caller gives: the console's `apiFetch` (a 401 sends the person to the
    // login page), or a public link's / embed's own token route (no session
    // at all, so a refusal there is "not found", never a redirect). The
    // server's `Cache-Control` and ETag decide how long it is kept.
    p = (entry.uploaded ? load(id) : fetch(entry.url, { cache: "force-cache" }))
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
