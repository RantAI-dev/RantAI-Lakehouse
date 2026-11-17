/**
 * Forecast a series with additive exponential smoothing (BI-17 part B,
 * plan `2026-10-24-bi-17b-forecast-clustering.md`, decision D3; the owner
 * signed the method in Q5).
 *
 * Pure maths over the numbers a chart already has, like `trendEnds`: no
 * query, no clock of its own, so the same forecast shows on screen and in the
 * PDF. Four models are fitted and the one with the lowest AICc is kept:
 *
 *   ANN  level only
 *   AAN  level and trend
 *   ANA  level and seasons
 *   AAA  level, trend and seasons
 *
 * (additive error, additive trend and season, no damping and no
 * multiplicative forms). In the innovations form of Hyndman, Koehler, Ord and
 * Snyder (2008), with one-step error e_t = y_t - (l + b + s):
 *
 *   l_t = l_{t-1} + b_{t-1} + alpha e_t
 *   b_t = b_{t-1} + beta e_t
 *   s_t = s_{t-m} + gamma e_t
 *
 * The smoothing parameters minimise the sum of squared one-step errors (for
 * additive errors that is the maximum likelihood fit) by a bounded
 * Nelder-Mead search inside 0 < alpha < 1, 0 < beta <= alpha, 0 < gamma <= 1 -
 * alpha. Initial states come from the classical decomposition of the first
 * seasons and a regression on the first ten points (`statsmodels` calls this
 * the "heuristic" start; the unit tests compare against it, see
 * `ops/forecast/make_reference.py`).
 *
 * The 95 % band is the analytical one of the additive class: the h-step
 * forecast variance is sigma^2 (1 + sum_{j<h} c_j^2) with
 * c_j = alpha + j beta + gamma [m divides j], and sigma^2 = SSE / n.
 *
 * Every case where a forecast would be a guess returns `{ refused: sentence }`
 * for the chart to print; nothing here draws a line it cannot defend.
 */
import type { Grain } from "./time-grain"

export type ForecastModel = "ANN" | "AAN" | "ANA" | "AAA"

export type ForecastInput = {
  /**
   * One value per consecutive period, oldest first. `null` (or a value that is
   * not a finite number) is a period with no value.
   */
  values: readonly (number | null)[]
  /** The chart's time grain; it sets the season length. */
  grain: Grain | string
  /** Periods ahead (1 to 60, and never more than the history). */
  horizon: number
  /**
   * End of the last period, exclusive, epoch milliseconds in the report time
   * zone. When `now` is before it the period is unfinished and left out
   * (feature decision 3). The caller owns the bucket arithmetic.
   */
  lastPeriodEnd?: number
  /** The clock, epoch milliseconds; the tests pass it, the chart omits it. */
  now?: number
}

export type Forecast = {
  model: ForecastModel
  /** The model in words: "level and trend", "level, trend and seasons". */
  label: string
  /** Periods in a season when the chosen model has seasons, else null. */
  seasonLength: number | null
  alpha: number
  beta: number | null
  gamma: number | null
  aicc: number
  /** Standard deviation of the one-step errors (sqrt(SSE / n)). */
  sigma: number
  point: number[]
  lower: number[]
  upper: number[]
  /** Periods the fit used, after the unfinished one was left out and gaps were filled. */
  periodsUsed: number
  /** Indexes into `input.values` of the first and last period the fit used. */
  firstIndex: number
  lastIndex: number
  /** Periods without a value that were filled in a straight line. */
  filled: number
  droppedCurrent: boolean
  /** Sentences for the chart's note: what was left out, filled or cut. */
  notes: string[]
}

export type ForecastRefusal = { refused: string }

export const MIN_PERIODS = 5
export const MAX_HORIZON = 60
/** More than this share of missing periods and no forecast; up to it, they are filled. */
export const MAX_GAP_SHARE = 0.1

const SEASON: Record<string, number> = { hour: 24, day: 7, week: 52, month: 12, quarter: 4 }
const LABEL: Record<ForecastModel, string> = {
  ANN: "level only",
  AAN: "level and trend",
  ANA: "level and seasons",
  AAA: "level, trend and seasons",
}
const MIN_PARAM = 1e-4
/** The normal quantile of a two-sided 95 % interval. */
const Z95 = 1.959963984540054

/** Periods in a season for a grain; null where the grain has none (year, minute, parts). */
export function seasonLength(grain: string): number | null {
  return SEASON[grain] ?? null
}

type Spec = { trend: boolean; season: boolean }
const SPECS: Record<ForecastModel, Spec> = {
  ANN: { trend: false, season: false },
  AAN: { trend: true, season: false },
  ANA: { trend: false, season: true },
  AAA: { trend: true, season: true },
}

type Init = { level: number; trend: number; seasonal: number[] }

/** Ordinary least squares of `ys` on x = 1..n: the value at x = 0 and the slope. */
function regress(ys: readonly number[]): [number, number] {
  const n = ys.length
  const mx = (n + 1) / 2
  const my = ys.reduce((a, b) => a + b, 0) / n
  let sxy = 0
  let sxx = 0
  ys.forEach((y, i) => { sxy += (i + 1 - mx) * (y - my); sxx += (i + 1 - mx) ** 2 })
  const slope = sxx === 0 ? 0 : sxy / sxx
  return [my - slope * mx, slope]
}

/** Fewest points the seasonal start needs: two full seasons and enough smoothed points to regress on. */
function seasonalMinimum(m: number): number {
  return Math.max(2 * m, 10 + 2 * Math.floor(m / 2))
}

/** Classical decomposition of the first seasons, then a regression on the first ten trend points. */
function seasonalInit(y: readonly number[], m: number): Init {
  const n = y.length
  const minObs = 10 + 2 * Math.floor(m / 2)
  const cycles = Math.max(Math.min(5, Math.floor(n / m)), Math.ceil(minObs / m))
  const len = Math.min(n, cycles * m)
  const half = Math.floor(m / 2)
  // Centred moving average of one season (2 x m for an even season).
  const trend: number[] = new Array(len).fill(Number.NaN)
  for (let i = 0; i < len; i++) {
    if (m % 2 === 0) {
      if (i - half < 0 || i + half > len - 1) continue
      let s = 0.5 * (y[i - half] + y[i + half])
      for (let j = i - half + 1; j <= i + half - 1; j++) s += y[j]
      trend[i] = s / m
    } else {
      if (i - half < 0 || i + half > len - 1) continue
      let s = 0
      for (let j = i - half; j <= i + half; j++) s += y[j]
      trend[i] = s / m
    }
  }
  const seasonal: number[] = []
  for (let pos = 0; pos < m; pos++) {
    let sum = 0
    let count = 0
    for (let c = 0; c < cycles; c++) {
      const t = c * m + pos
      if (t < len && !Number.isNaN(trend[t])) { sum += y[t] - trend[t]; count++ }
    }
    seasonal.push(count ? sum / count : 0)
  }
  const mean = seasonal.reduce((a, b) => a + b, 0) / m
  const centred = seasonal.map((s) => s - mean)
  const smooth = trend.filter((v) => !Number.isNaN(v)).slice(0, 10)
  const [level, slope] = regress(smooth)
  return { level, trend: slope, seasonal: centred }
}

function plainInit(y: readonly number[]): Init {
  const [level, slope] = regress(y.slice(0, Math.min(10, y.length)))
  return { level, trend: slope, seasonal: [] }
}

type Params = { alpha: number; beta: number; gamma: number }

/** Project onto the admissible region: 0 < alpha < 1, 0 < beta <= alpha, 0 < gamma <= 1 - alpha. */
function admit(x: readonly number[], spec: Spec): Params {
  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))
  const alpha = clamp(x[0], MIN_PARAM, 1 - MIN_PARAM)
  let i = 1
  const beta = spec.trend ? clamp(x[i++], MIN_PARAM, alpha) : 0
  const gamma = spec.season ? clamp(x[i], MIN_PARAM, Math.max(MIN_PARAM, 1 - alpha)) : 0
  return { alpha, beta, gamma }
}

type Run = { sse: number; level: number; trend: number; seasonal: number[] }

/** One pass over the series: the sum of squared one-step errors and the final states. */
function run(y: readonly number[], spec: Spec, init: Init, p: Params, m: number): Run {
  let level = init.level
  let trend = spec.trend ? init.trend : 0
  const seasonal = spec.season ? init.seasonal.slice() : []
  let sse = 0
  for (let t = 0; t < y.length; t++) {
    const si = spec.season ? t % m : 0
    const predicted = level + trend + (spec.season ? seasonal[si] : 0)
    const e = y[t] - predicted
    sse += e * e
    level = level + trend + p.alpha * e
    if (spec.trend) trend += p.beta * e
    if (spec.season) seasonal[si] += p.gamma * e
  }
  return { sse, level, trend, seasonal }
}

/** Nelder-Mead on `f`, started at `x0`; the objective projects its own input, so the search is unconstrained. */
function nelderMead(f: (x: number[]) => number, x0: number[], step: number): { x: number[]; value: number } {
  const n = x0.length
  let simplex = [x0.slice(), ...x0.map((_, i) => x0.map((v, j) => (i === j ? v + step : v)))]
  let values = simplex.map(f)
  for (let iter = 0; iter < 400 * n; iter++) {
    const order = values.map((_, i) => i).sort((a, b) => values[a] - values[b])
    simplex = order.map((i) => simplex[i])
    values = order.map((i) => values[i])
    const spread = Math.abs(values[n] - values[0])
    if (spread <= 1e-12 * (Math.abs(values[0]) + 1e-12)) break
    const centroid = x0.map((_, j) => simplex.slice(0, n).reduce((a, p) => a + p[j], 0) / n)
    const along = (k: number) => centroid.map((c, j) => c + k * (simplex[n][j] - c))
    const reflected = along(-1)
    const fr = f(reflected)
    if (fr < values[0]) {
      const expanded = along(-2)
      const fe = f(expanded)
      if (fe < fr) { simplex[n] = expanded; values[n] = fe } else { simplex[n] = reflected; values[n] = fr }
    } else if (fr < values[n - 1]) {
      simplex[n] = reflected; values[n] = fr
    } else {
      const contracted = fr < values[n] ? along(-0.5) : along(0.5)
      const fc = f(contracted)
      if (fc < Math.min(fr, values[n])) { simplex[n] = contracted; values[n] = fc } else {
        for (let i = 1; i <= n; i++) {
          simplex[i] = simplex[i].map((v, j) => simplex[0][j] + 0.5 * (v - simplex[0][j]))
          values[i] = f(simplex[i])
        }
      }
    }
  }
  let best = 0
  values.forEach((v, i) => { if (v < values[best]) best = i })
  return { x: simplex[best], value: values[best] }
}

type Fit = Params & { model: ForecastModel; aicc: number; sse: number; end: Run; m: number }

function fitModel(y: readonly number[], model: ForecastModel, m: number): Fit | null {
  const spec = SPECS[model]
  const n = y.length
  const states = 1 + (spec.trend ? 1 : 0) + (spec.season ? m : 0)
  const k = 1 + (spec.trend ? 1 : 0) + (spec.season ? 1 : 0) + states + 1
  // Too few points for this many parameters: AICc has no value, the model is not a candidate.
  if (n - k - 1 <= 0) return null
  const init = spec.season ? seasonalInit(y, m) : plainInit(y)
  const dims = 1 + (spec.trend ? 1 : 0) + (spec.season ? 1 : 0)
  const objective = (x: number[]) => {
    const sse = run(y, spec, init, admit(x, spec), m).sse
    return Number.isFinite(sse) ? sse : Number.POSITIVE_INFINITY
  }
  // A few starts: the surface has flat regions at the bounds and more than one basin.
  const starts = [[0.1, 0.01, 0.01], [0.5, 0.1, 0.1], [0.9, 0.05, 0.05], [0.3, 0.02, 0.3]]
  let best: { x: number[]; value: number } | null = null
  for (const s of starts) {
    const r = nelderMead(objective, s.slice(0, dims), 0.1)
    if (!best || r.value < best.value) best = r
  }
  if (!best) return null
  const p = admit(best.x, spec)
  const end = run(y, spec, init, p, m)
  const sse = Math.max(end.sse, 1e-300)
  const llf = -(n / 2) * (1 + Math.log(2 * Math.PI) + Math.log(sse / n))
  const aicc = -2 * llf + 2 * k + (2 * k * (k + 1)) / (n - k - 1)
  return { ...p, model, aicc, sse, end, m }
}

function interpolate(values: readonly (number | null)[]): { y: number[]; filled: number } {
  const ok = (v: number | null): v is number => typeof v === "number" && Number.isFinite(v)
  const y: number[] = []
  let filled = 0
  for (let i = 0; i < values.length; i++) {
    const v = values[i]
    if (ok(v)) { y.push(v); continue }
    let lo = i - 1
    while (lo >= 0 && !ok(values[lo])) lo--
    let hi = i + 1
    while (hi < values.length && !ok(values[hi])) hi++
    const a = values[lo] as number
    const b = values[hi] as number
    y.push(a + ((b - a) * (i - lo)) / (hi - lo))
    filled++
  }
  return { y, filled }
}

export function forecast(input: ForecastInput): Forecast | ForecastRefusal {
  const notes: string[] = []
  const values = input.values.slice()
  const now = input.now ?? Date.now()
  let droppedCurrent = false
  if (input.lastPeriodEnd !== undefined && now < input.lastPeriodEnd && values.length > 0) {
    values.pop()
    droppedCurrent = true
    notes.push("The current period is not finished and is left out.")
  }
  const isValue = (v: number | null) => typeof v === "number" && Number.isFinite(v)
  // BI-17 B QA fix F1: the history runs from the first to the last period that
  // has a value. Empty periods at either end are not data (a daily chart with
  // a row for only some days can end on an empty day once the unfinished one
  // is left out), so they are trimmed, not refused; gaps inside keep the 10 %
  // rule below. The forecast then continues from the last period with a value.
  let firstIndex = 0
  while (values.length && !isValue(values[0])) { values.shift(); firstIndex++ }
  let trimmedEnd = 0
  while (values.length && !isValue(values[values.length - 1])) { values.pop(); trimmedEnd++ }
  if (trimmedEnd > 0) notes.push(`${trimmedEnd} empty ${trimmedEnd === 1 ? "period" : "periods"} at the end ${trimmedEnd === 1 ? "is" : "are"} left out.`)
  const lastIndex = firstIndex + values.length - 1
  const n = values.length
  if (n < MIN_PERIODS) {
    const left = droppedCurrent ? " (the unfinished current period is left out)" : ""
    return { refused: `A forecast needs at least ${MIN_PERIODS} periods; this chart has ${n}${left}.` }
  }
  const missing = values.filter((v) => !isValue(v)).length
  if (missing / n > MAX_GAP_SHARE) {
    // F1: say what is true: the rows skip too many periods. A coarser grouping has fewer empty ones.
    const finer = ["minute", "hour", "day", "week"].includes(String(input.grain))
    return {
      refused: `${missing} of the ${n} periods from the first to the last have no data, more than ${Math.round(MAX_GAP_SHARE * 100)} %, so a forecast would be guessing.${finer ? " Group the dates by a coarser period (for example month) to forecast this chart." : ""}`,
    }
  }
  const { y, filled } = interpolate(values)
  if (filled > 0) notes.push(`${filled} ${filled === 1 ? "period" : "periods"} without a value ${filled === 1 ? "was" : "were"} filled in a straight line.`)

  const m = seasonLength(String(input.grain))
  const models: ForecastModel[] = ["ANN", "AAN"]
  if (m !== null) {
    if (n >= seasonalMinimum(m)) models.push("ANA", "AAA")
    else if (n < 2 * m) notes.push(`No seasons: ${n} periods is fewer than 2 full seasons (2 x ${m}).`)
    else notes.push(`No seasons: ${n} periods is too few to start them (at least ${seasonalMinimum(m)}).`)
  }

  let best: Fit | null = null
  for (const model of models) {
    const fit = fitModel(y, model, m ?? 0)
    // A tie goes to the simpler model, which comes first in `models`.
    if (fit && (!best || fit.aicc < best.aicc - 1e-9)) best = fit
  }
  if (!best) return { refused: `A forecast needs more history than the ${n} periods this chart has.` }

  const wanted = Math.max(1, Math.min(MAX_HORIZON, Math.floor(input.horizon)))
  const horizon = Math.min(wanted, n)
  if (horizon < wanted) notes.push(`The forecast is cut to ${horizon} periods, the length of the history.`)

  const spec = SPECS[best.model]
  const sigma2 = best.sse / n
  const point: number[] = []
  const lower: number[] = []
  const upper: number[] = []
  let cumulative = 0
  for (let h = 1; h <= horizon; h++) {
    if (h > 1) {
      const j = h - 1
      const c = best.alpha + (spec.trend ? j * best.beta : 0) + (spec.season && j % best.m === 0 ? best.gamma : 0)
      cumulative += c * c
    }
    const mean =
      best.end.level + (spec.trend ? h * best.end.trend : 0) + (spec.season ? best.end.seasonal[(n + h - 1) % best.m] : 0)
    const half = Z95 * Math.sqrt(sigma2 * (1 + cumulative))
    point.push(mean)
    lower.push(mean - half)
    upper.push(mean + half)
  }
  return {
    model: best.model,
    label: LABEL[best.model],
    seasonLength: spec.season ? best.m : null,
    alpha: best.alpha,
    beta: spec.trend ? best.beta : null,
    gamma: spec.season ? best.gamma : null,
    aicc: best.aicc,
    sigma: Math.sqrt(sigma2),
    point,
    lower,
    upper,
    periodsUsed: n,
    firstIndex,
    lastIndex,
    filled,
    droppedCurrent,
    notes,
  }
}

export function isRefusal(result: Forecast | ForecastRefusal): result is ForecastRefusal {
  return "refused" in result
}
