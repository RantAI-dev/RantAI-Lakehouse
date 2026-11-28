import { describe, expect, test } from "bun:test";
import {
  GOLD_ROWS,
  GOLD_TOTAL,
  PHONE_ROW_IDS,
  RAW_RECORDS,
  SILVER_CHECKS,
  SILVER_RECORDS,
  isoDate,
  standardCity,
} from "./depth-records";

describe("the depths section's example records (WEB-5 O21)", () => {
  test("Bronze shows the mess the section promises: a duplicate, three city spellings, mixed dates, a missing value, two sources", () => {
    expect(RAW_RECORDS.length).toBe(5);
    expect(new Set(RAW_RECORDS.map((r) => r.source))).toEqual(new Set(["orders", "crm"]));
    expect(new Set(RAW_RECORDS.filter((r) => standardCity(r.city) === "Jakarta").map((r) => r.city)).size).toBe(3);
    expect(RAW_RECORDS.some((r) => r.amount === null)).toBe(true);
    expect(new Set(RAW_RECORDS.map((r) => r.date)).size).toBeGreaterThanOrEqual(3);
    expect(SILVER_RECORDS.filter((r) => r.duplicate).length).toBe(1);
  });

  test("dates in the three raw formats all become ISO dates", () => {
    expect(isoDate("2026-11-03")).toBe("2026-11-03");
    expect(isoDate("03/11/26")).toBe("2026-11-03");
    expect(isoDate("4 Nov 2026")).toBe("2026-11-04");
  });

  test("Silver standardises every city and every date, and keeps the empty amount empty", () => {
    expect(SILVER_RECORDS.every((r) => r.city === "Jakarta" || r.city === "Bandung")).toBe(true);
    expect(SILVER_RECORDS.every((r) => /^\d{4}-\d{2}-\d{2}$/.test(r.date))).toBe(true);
    expect(SILVER_RECORDS.find((r) => r.order === "#1045")?.amount).toBeNull();
  });

  test("the quality checks pass on duplicates and cities and fail on the one empty amount", () => {
    expect(SILVER_CHECKS).toEqual({ duplicates: 0, invalidCities: 0, emptyAmounts: 1 });
  });

  test("Gold is revenue by city from the rows Silver keeps", () => {
    expect(GOLD_ROWS).toEqual([
      { city: "Jakarta", orders: 3, revenue: 770_000 },
      { city: "Bandung", orders: 1, revenue: 280_000 },
    ]);
    expect(GOLD_TOTAL).toBe(1_050_000);
  });

  test("the phone's three rows still show every problem: a misspelt city, a duplicate and an empty amount", () => {
    const rows = RAW_RECORDS.filter((r) => PHONE_ROW_IDS.includes(r.id));
    expect(rows.length).toBe(3);
    const clean = SILVER_RECORDS.filter((r) => PHONE_ROW_IDS.includes(r.id));
    expect(rows.some((r) => r.city !== standardCity(r.city))).toBe(true);
    expect(clean.filter((r) => r.duplicate).length).toBe(1);
    expect(rows.filter((r) => r.amount === null).length).toBe(1);
    // Every city a phone's Silver keeps is one Gold shows.
    for (const r of clean.filter((c) => !c.duplicate)) expect(GOLD_ROWS.some((g) => g.city === r.city)).toBe(true);
  });
});
