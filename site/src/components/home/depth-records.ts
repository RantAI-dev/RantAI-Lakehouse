/**
 * WEB-5 O21: the few example records the depths section follows through
 * Bronze, Silver and Gold, so a visitor watches one dataset change instead
 * of reading three sentences (owner, 2026-11-07: "ini 3 layer ini tu apa?").
 *
 * The records are fictional, and plainly so (O32: the owner dropped the
 * visible "Illustration" label; the screen-reader text still describes
 * them as example rows). Silver and Gold are *computed* from the Bronze rows here, not typed
 * in, so the three cards can never disagree with each other: the duplicate
 * that leaves, the spellings that settle, the empty amount the quality
 * check flags, and the totals Gold shows all follow from the five rows
 * below. `depth-records.test.ts` pins the result.
 *
 * What each step stands for, and the backlog row behind it:
 * - Bronze keeps every row as it came, mess and all (BLD-3: Bronze is
 *   Iceberg; WEB-1's "nothing is lost").
 * - Silver is a pipeline's clean-up (WEB-1: pipelines clean the raw data)
 *   with quality checks on the result (DATA-14: duplicate count, accepted
 *   values and null checks are in its library; a failing check is flagged
 *   and never blocks the load, so the row with the empty amount stays).
 * - Gold groups the clean rows into a table built for one question, which
 *   refreshes itself within a target lag (BLD-2).
 */

export type SourceId = "orders" | "crm";

export type RawRecord = {
  /** Stable identity of the row across the three cards (React key, CSS hooks). */
  id: string;
  source: SourceId;
  order: string;
  city: string;
  date: string;
  /** `null`: the source sent no amount. */
  amount: number | null;
};

/**
 * Bronze, as the two sources sent it: an orders file and a CRM export. The
 * mess is deliberate and visible: one row arrives twice, the city is spelt
 * three ways (one with a trailing space), the dates come in three formats,
 * and one amount is missing.
 */
export const RAW_RECORDS: readonly RawRecord[] = [
  { id: "r1", source: "orders", order: "#1042", city: "jakarta", date: "2026-11-03", amount: 450_000 },
  { id: "r2", source: "crm", order: "#1043", city: "Jakarta ", date: "03/11/26", amount: 320_000 },
  { id: "r3", source: "orders", order: "#1044", city: "Bandung", date: "2026-11-03", amount: 280_000 },
  { id: "r4", source: "orders", order: "#1044", city: "Bandung", date: "2026-11-03", amount: 280_000 },
  { id: "r5", source: "crm", order: "#1045", city: "JKT", date: "4 Nov 2026", amount: null },
];

/** The city names Silver accepts, and the spellings it maps onto them. */
const CITY_ALIASES: Record<string, string> = { jakarta: "Jakarta", jkt: "Jakarta", bandung: "Bandung" };

const MONTHS: Record<string, string> = {
  jan: "01", feb: "02", mar: "03", apr: "04", may: "05", jun: "06",
  jul: "07", aug: "08", sep: "09", oct: "10", nov: "11", dec: "12",
};

/** One city spelling onto its accepted name; unknown spellings stay as they are. */
export function standardCity(raw: string): string {
  return CITY_ALIASES[raw.trim().toLowerCase()] ?? raw.trim();
}

/** `2026-11-03`, `03/11/26` (day first) or `4 Nov 2026` onto ISO `YYYY-MM-DD`. */
export function isoDate(raw: string): string {
  const s = raw.trim();
  if (/^\d{4}-\d{2}-\d{2}$/.test(s)) return s;
  const dmy = /^(\d{1,2})\/(\d{1,2})\/(\d{2}|\d{4})$/.exec(s);
  if (dmy) {
    const year = dmy[3].length === 2 ? `20${dmy[3]}` : dmy[3];
    return `${year}-${dmy[2].padStart(2, "0")}-${dmy[1].padStart(2, "0")}`;
  }
  const named = /^(\d{1,2}) ([A-Za-z]{3})\w* (\d{4})$/.exec(s);
  if (named && MONTHS[named[2].toLowerCase()]) {
    return `${named[3]}-${MONTHS[named[2].toLowerCase()]}-${named[1].padStart(2, "0")}`;
  }
  return s;
}

export type CleanRecord = RawRecord & {
  /** Silver drops a row it has already seen (same order, same values). */
  duplicate: boolean;
};

/**
 * Silver: the same rows, cleaned. The second copy of an order is marked
 * `duplicate` (the card animates it out; it is kept in the list so the
 * row's identity survives the transition), the city and the date are
 * standardised, and the missing amount stays missing: cleaning does not
 * invent a value, the quality check flags it.
 */
export function toSilver(rows: readonly RawRecord[]): CleanRecord[] {
  const seen = new Set<string>();
  return rows.map((r) => {
    const city = standardCity(r.city);
    const date = isoDate(r.date);
    const fingerprint = [r.order, city, date, r.amount].join("|");
    const duplicate = seen.has(fingerprint);
    seen.add(fingerprint);
    return { ...r, city, date, duplicate };
  });
}

export type QualityResult = {
  /** Duplicate count over the kept rows: 0 passes. */
  duplicates: number;
  /** Rows whose city is not an accepted value: 0 passes. */
  invalidCities: number;
  /** Rows with an empty amount: 0 passes. */
  emptyAmounts: number;
};

const ACCEPTED_CITIES = new Set(Object.values(CITY_ALIASES));

/** The three checks the Silver card shows, run over the rows Silver keeps. */
export function silverChecks(rows: readonly CleanRecord[]): QualityResult {
  const kept = rows.filter((r) => !r.duplicate);
  const orders = kept.map((r) => r.order);
  return {
    duplicates: orders.length - new Set(orders).size,
    invalidCities: kept.filter((r) => !ACCEPTED_CITIES.has(r.city)).length,
    emptyAmounts: kept.filter((r) => r.amount === null).length,
  };
}

export type GoldRow = { city: string; orders: number; revenue: number };

/**
 * Gold: revenue by city, from the rows Silver keeps, largest first. An
 * order with no amount still counts as an order but adds nothing to the
 * revenue, the way `SUM` treats a null.
 */
export function toGold(rows: readonly CleanRecord[]): GoldRow[] {
  const byCity = new Map<string, GoldRow>();
  for (const r of rows) {
    if (r.duplicate) continue;
    const g = byCity.get(r.city) ?? { city: r.city, orders: 0, revenue: 0 };
    g.orders += 1;
    g.revenue += r.amount ?? 0;
    byCity.set(r.city, g);
  }
  return [...byCity.values()].sort((a, b) => b.revenue - a.revenue);
}

export const SILVER_RECORDS = toSilver(RAW_RECORDS);

/**
 * WEB-5 owner QA 2026-11-25 ("karena dia versi mobile jadi terkesan banyak banget datanya"): below
 * 1024 px the Bronze and Silver cards show three of the five rows, one per problem the section
 * names: the misspelt "jakarta" (r1), the order that arrived twice (r4, marked ×2) and the empty
 * amount spelt "JKT" (r5). The headers say "3 of 5 rows" and "2 of 4 rows", and Gold is built from
 * all five rows as on desktop, so nothing a phone shows contradicts the totals.
 */
export const PHONE_ROW_IDS: readonly string[] = ["r1", "r4", "r5"];
export const SILVER_CHECKS = silverChecks(SILVER_RECORDS);
export const GOLD_ROWS = toGold(SILVER_RECORDS);
export const GOLD_TOTAL = GOLD_ROWS.reduce((sum, g) => sum + g.revenue, 0);

/**
 * Names shown in code chips: table and file names are code, so they stay in
 * the code face and are not translated (D3; owner QA note 1: monospace only
 * for code).
 */
export const DEPTH_TABLES = {
  bronze: "bronze.orders_raw",
  silver: "silver.orders",
  gold: "gold.revenue_by_city",
} as const;

export const SOURCE_FILES: Record<SourceId, string> = {
  orders: "orders.csv",
  crm: "crm_export",
};
