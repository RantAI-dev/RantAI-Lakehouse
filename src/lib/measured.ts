/**
 * A metric the backend may or may not be able to measure.
 *
 * `null` means "not measured" — the API could not compute this value and said
 * so, rather than emitting a `0` that a reader would mistake for a real
 * measurement. `0` means a genuine measured zero. The distinction is the whole
 * point of the type; do not collapse it with `?? 0` at a call site.
 *
 * WS1 task 1.0 — see docs/superpowers/plans/2026-09-10-ws1-honesty-pass.md
 */
export type Measured = number | null

/** Renders a {@link Measured} for display, as `—` when it was not measured. */
export function fmtMeasured(v: Measured, f: (n: number) => string = String): string {
  return v === null ? "—" : f(v)
}
