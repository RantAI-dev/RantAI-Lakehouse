/** The hero lab's three choices, and how they live in the URL (`?font=2&layout=B&time=day`). */

export const FONTS = [
  { id: "1", short: "Serif", name: "Instrument Serif (current)" },
  { id: "2", short: "Fraunces", name: "Fraunces" },
  { id: "3", short: "Tight", name: "Inter Tight" },
  { id: "4", short: "Bricolage", name: "Bricolage Grotesque" },
] as const;
export const LAYOUTS = [
  { id: "A", short: "Current", name: "A, current: text left, whale right" },
  { id: "B", short: "Centred", name: "B, centred" },
  { id: "C", short: "Editorial", name: "C, editorial" },
] as const;
export const TIMES = [
  { id: "night", short: "Night", name: "Night" },
  { id: "day", short: "Day", name: "Day" },
] as const;

export type FontId = (typeof FONTS)[number]["id"];
export type LayoutId = (typeof LAYOUTS)[number]["id"];
export type TimeId = (typeof TIMES)[number]["id"];
export type LabChoice = { font: FontId; layout: LayoutId; time: TimeId };

export const DEFAULT_CHOICE: LabChoice = { font: "1", layout: "A", time: "night" };

/** Reads a query string; anything unknown falls back to the default. */
export function parseChoice(search: string): LabChoice {
  const q = new URLSearchParams(search);
  const pick = <T extends { id: string }>(list: readonly T[], value: string | null, fallback: T["id"]) =>
    (list.find((o) => o.id === value)?.id ?? fallback) as T["id"];
  return {
    font: pick(FONTS, q.get("font"), DEFAULT_CHOICE.font),
    layout: pick(LAYOUTS, q.get("layout")?.toUpperCase() ?? null, DEFAULT_CHOICE.layout),
    time: pick(TIMES, q.get("time"), DEFAULT_CHOICE.time),
  };
}

export function choiceQuery(c: LabChoice): string {
  return `?font=${c.font}&layout=${c.layout}&time=${c.time}`;
}
