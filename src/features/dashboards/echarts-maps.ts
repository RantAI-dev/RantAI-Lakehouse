import * as echarts from "echarts";

/**
 * Registrasi peta ECharts dari GeoJSON LOKAL (di-bundle di public/), tanpa
 * panggilan ke server tile eksternal — konsisten dengan ethos self-host &
 * aman di embed/offline. Choropleth memakai nama peta ini.
 */
export const JAKARTA_MAP = "dki-jakarta";
const JAKARTA_GEOJSON_URL = "/geo/dki-jakarta.geojson";

const loading = new Map<string, Promise<boolean>>();

/** Muat + daftarkan peta sekali (idempoten). Resolve false bila GeoJSON tak ada. */
export function ensureMap(name = JAKARTA_MAP, url = JAKARTA_GEOJSON_URL): Promise<boolean> {
  if (echarts.getMap(name)) return Promise.resolve(true);
  let p = loading.get(name);
  if (!p) {
    p = fetch(url, { cache: "force-cache" })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error("geojson not found"))))
      .then((gj) => { echarts.registerMap(name, gj as Parameters<typeof echarts.registerMap>[1]); return true; })
      .catch(() => false);
    loading.set(name, p);
  }
  return p;
}

/**
 * Normalisasi nama wilayah agar cocok dengan nama fitur di GeoJSON Jakarta
 * (mis. "KOTA JAKARTA PUSAT"/"Jakarta Pusat" → "Jakarta Pusat").
 */
export function normalizeJakartaArea(raw: string): string {
  let s = String(raw ?? "").trim().replace(/\s+/g, " ");
  s = s.replace(/^(kota\s+(administrasi\s+)?|kabupaten\s+(administrasi\s+)?|kab\.?\s+)/i, "");
  const t = s.toLowerCase();
  if (t.includes("seribu")) return "Kepulauan Seribu";
  if (t.includes("pusat")) return "Jakarta Pusat";
  if (t.includes("utara")) return "Jakarta Utara";
  if (t.includes("barat")) return "Jakarta Barat";
  if (t.includes("selatan")) return "Jakarta Selatan";
  if (t.includes("timur")) return "Jakarta Timur";
  // Title-case fallback.
  return s.replace(/\b\w/g, (c) => c.toUpperCase());
}
