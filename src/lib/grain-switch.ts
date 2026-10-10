/**
 * The dashboard's grain switch (`BI-9`): one control that regroups every
 * chart whose own grain is a day-to-year truncation. The choice lives in the
 * address, beside the filters (`?grain=`), and is sent to
 * `GET /api/dashboard` as `grain`; an editor can save it as the board's
 * default. Embeds and public links use the saved value and offer no switch.
 */
import { isGrain, isTruncation, type Grain } from "./time-grain"

/** The address parameter, next to the filters' `f`. */
export const GRAIN_PARAM = "grain"

/** Each chart keeps its own grain, saved default or not. */
export const OWN_GRAIN = "own"

/** What the address says: a truncation, "own", or nothing (the saved default applies). */
export type GrainChoice = Grain | typeof OWN_GRAIN | null

/** A `?grain=` value as a choice; anything that is not a truncation or `own` is no choice. */
export function readGrainParam(raw: string | null): GrainChoice {
  if (raw === OWN_GRAIN) return OWN_GRAIN
  return raw !== null && isGrain(raw) && isTruncation(raw) ? raw : null
}

/**
 * What the control shows: the address's choice, else the board's saved grain,
 * else "own". The empty string is the "each chart's own" entry.
 */
export function shownGrain(choice: GrainChoice, saved: string | undefined): Grain | "" {
  if (choice === OWN_GRAIN) return ""
  if (choice) return choice
  return saved && isGrain(saved) && isTruncation(saved) ? saved : ""
}

/** The `grain` to send to the API for an address choice; absent when the address says nothing. */
export function grainQuery(choice: GrainChoice): string | null {
  return choice
}

/** The saved default as the PUT body carries it: `""` clears it. */
export function savedGrainBody(shown: Grain | ""): string {
  return shown
}

/** Whether the shown choice differs from the saved default, so "Save as default" is offered. */
export function differsFromSaved(shown: Grain | "", saved: string | undefined): boolean {
  return shown !== (saved && isGrain(saved) && isTruncation(saved) ? saved : "")
}

/** The address with the choice written (`own` for "each chart's own"), or the parameter removed when `choice` is null. */
export function withGrainParam(search: string, choice: GrainChoice): string {
  const q = new URLSearchParams(search)
  if (choice === null) q.delete(GRAIN_PARAM)
  else q.set(GRAIN_PARAM, choice)
  return q.toString()
}
