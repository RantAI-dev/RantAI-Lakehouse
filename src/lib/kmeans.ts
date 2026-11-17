/**
 * k-means clustering of the points on a scatter or bubble chart (BI-17 part
 * B, plan `2026-10-24-bi-17b-forecast-clustering.md`, decision D5).
 *
 * Pure and deterministic: the same points give the same groups every time, so
 * a chart does not reshuffle its colours on reload.
 *
 *  - Each measure is scaled to 0..1 first (min-max), so a measure in millions
 *    does not decide the groups on its own; a constant measure scales to 0 and
 *    plays no part.
 *  - Seeds by k-means++ from a fixed-seed PRNG (mulberry32, seed 1), Lloyd
 *    iterations to convergence or 100, five restarts keeping the lowest
 *    inertia.
 *  - With `k: "auto"` the number of groups is the one with the highest
 *    Calinski-Harabasz score over 2..min(10, floor(n / 3)); a fixed `k` is
 *    honoured or refused, never silently changed.
 *  - Every group must have at least 3 points (feature decision 6); a k whose
 *    best solution has a smaller group is not a candidate.
 *
 * Fewer than 6 usable points, or no k that meets the rules, returns
 * `{ refused: sentence }` for the chart to print.
 */

export type KMeansInput = {
  /** One row per point, one number per measure (two for a scatter, three for a bubble). */
  points: readonly (readonly number[])[]
  /** "auto", or a number of groups from 2 to 10. */
  k: "auto" | number
}

export type Cluster = {
  /** 0-based; clusters are numbered from the largest, ties by the first measure's centre. */
  index: number
  size: number
  /** Mean of each measure over the cluster's points, in the original units. */
  center: number[]
}

export type KMeansResult = {
  k: number
  /** True when `k` was chosen by the Calinski-Harabasz score. */
  auto: boolean
  /** The cluster index of each input point; -1 for a point with a measure that is not a finite number. */
  labels: number[]
  clusters: Cluster[]
  /** Calinski-Harabasz score of the chosen solution; null when it has no value (a perfect split). */
  score: number | null
  /** Points left out because a measure was not a finite number. */
  skipped: number
}

export type KMeansRefusal = { refused: string }

export const MIN_POINTS = 6
export const MIN_GROUP = 3
export const MAX_K = 10
const RESTARTS = 5
const MAX_ITERATIONS = 100
const SEED = 1

/** mulberry32: a small PRNG with a 32-bit state, so a fixed seed gives the same stream everywhere. */
export function mulberry32(seed: number): () => number {
  let a = seed | 0
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const dist2 = (a: readonly number[], b: readonly number[]) => {
  let s = 0
  for (let i = 0; i < a.length; i++) s += (a[i] - b[i]) ** 2
  return s
}

type Solution = { labels: number[]; centers: number[][]; inertia: number; sizes: number[] }

function seed(points: readonly number[][], k: number, rand: () => number): number[][] {
  const centers = [points[Math.floor(rand() * points.length)].slice()]
  const d2 = points.map((p) => dist2(p, centers[0]))
  while (centers.length < k) {
    const total = d2.reduce((a, b) => a + b, 0)
    let pick = points.length - 1
    if (total > 0) {
      let r = rand() * total
      for (let i = 0; i < points.length; i++) {
        r -= d2[i]
        if (r <= 0) { pick = i; break }
      }
    } else {
      // Every remaining point sits on a centre already: take the first not used as one.
      pick = Math.max(0, d2.findIndex((d, i) => d === 0 && !centers.some((c) => dist2(c, points[i]) === 0)))
    }
    centers.push(points[pick].slice())
    points.forEach((p, i) => { d2[i] = Math.min(d2[i], dist2(p, centers[centers.length - 1])) })
  }
  return centers
}

function lloyd(points: readonly number[][], k: number, rand: () => number): Solution {
  const d = points[0].length
  const centers = seed(points, k, rand)
  const labels: number[] = new Array(points.length).fill(-1)
  for (let iter = 0; iter < MAX_ITERATIONS; iter++) {
    let moved = false
    points.forEach((p, i) => {
      let best = 0
      let bestD = Infinity
      centers.forEach((c, j) => {
        const dd = dist2(p, c)
        if (dd < bestD) { bestD = dd; best = j }
      })
      if (labels[i] !== best) { labels[i] = best; moved = true }
    })
    const sums = centers.map(() => new Array<number>(d).fill(0))
    const counts = new Array<number>(k).fill(0)
    points.forEach((p, i) => { counts[labels[i]]++; p.forEach((v, j) => { sums[labels[i]][j] += v }) })
    for (let j = 0; j < k; j++) {
      if (counts[j] > 0) { centers[j] = sums[j].map((s) => s / counts[j]); continue }
      // An empty group takes the point farthest from its own centre.
      let far = 0
      let farD = -1
      points.forEach((p, i) => { const dd = dist2(p, centers[labels[i]]); if (dd > farD) { farD = dd; far = i } })
      centers[j] = points[far].slice()
      labels[far] = j
      moved = true
    }
    if (!moved) break
  }
  const sizes = new Array<number>(k).fill(0)
  let inertia = 0
  points.forEach((p, i) => { sizes[labels[i]]++; inertia += dist2(p, centers[labels[i]]) })
  return { labels, centers, inertia, sizes }
}

/** Best of the restarts for one k; a fresh PRNG per k so a fixed k gives what auto gave for it. */
function solve(points: readonly number[][], k: number): Solution {
  const rand = mulberry32(SEED)
  let best: Solution | null = null
  for (let r = 0; r < RESTARTS; r++) {
    const s = lloyd(points, k, rand)
    if (!best || s.inertia < best.inertia - 1e-12) best = s
  }
  return best as Solution
}

/** Calinski-Harabasz: between-group over within-group dispersion, each per degree of freedom. */
function calinskiHarabasz(points: readonly number[][], s: Solution): number {
  const n = points.length
  const k = s.centers.length
  const d = points[0].length
  const mean = new Array<number>(d).fill(0)
  points.forEach((p) => p.forEach((v, j) => { mean[j] += v / n }))
  let between = 0
  s.centers.forEach((c, j) => { between += s.sizes[j] * dist2(c, mean) })
  if (s.inertia <= 1e-12) return Infinity
  return (between / (k - 1)) / (s.inertia / (n - k))
}

function scale(points: readonly (readonly number[])[]): number[][] {
  const d = points[0].length
  const lo = new Array<number>(d).fill(Infinity)
  const hi = new Array<number>(d).fill(-Infinity)
  points.forEach((p) => p.forEach((v, j) => { lo[j] = Math.min(lo[j], v); hi[j] = Math.max(hi[j], v) }))
  return points.map((p) => p.map((v, j) => (hi[j] === lo[j] ? 0 : (v - lo[j]) / (hi[j] - lo[j]))))
}

export function kmeans(input: KMeansInput): KMeansResult | KMeansRefusal {
  const usable = (p: readonly number[]) => p.length > 0 && p.every((v) => Number.isFinite(v))
  const kept: number[] = []
  input.points.forEach((p, i) => { if (usable(p)) kept.push(i) })
  const skipped = input.points.length - kept.length
  const n = kept.length
  if (n < MIN_POINTS) {
    return { refused: `Clusters need at least ${MIN_POINTS} points; this chart has ${n}.` }
  }
  const raw = kept.map((i) => input.points[i])
  const points = scale(raw)
  const auto = input.k === "auto"
  const maxK = Math.min(MAX_K, Math.floor(n / MIN_GROUP))
  let candidates: number[]
  if (auto) {
    candidates = Array.from({ length: maxK - 1 }, (_, i) => i + 2)
  } else {
    const k = input.k as number
    if (!Number.isInteger(k) || k < 2 || k > MAX_K) {
      return { refused: `The number of clusters must be from 2 to ${MAX_K}.` }
    }
    if (k > maxK) {
      return { refused: `${k} clusters need at least ${k * MIN_GROUP} points, ${MIN_GROUP} in each; this chart has ${n}.` }
    }
    candidates = [k]
  }
  let best: { k: number; s: Solution; score: number } | null = null
  for (const k of candidates) {
    const s = solve(points, k)
    if (s.sizes.some((size) => size < MIN_GROUP)) continue
    const score = calinskiHarabasz(points, s)
    // The smaller k wins a tie, which is also the case of a perfect split (score Infinity).
    if (!best || score > best.score) best = { k, s, score }
  }
  if (!best) {
    return { refused: `These points do not fall into groups of at least ${MIN_GROUP} points; no clusters are drawn.` }
  }
  // Number the groups from the largest, ties by their first centre, so the legend and the colours are stable.
  const meanOf = (members: number[]) => raw[0].map((_, j) => members.reduce((a, i) => a + raw[i][j], 0) / members.length)
  const members = best.s.centers.map((_, g) => best.s.labels.flatMap((l, i) => (l === g ? [i] : [])))
  const centers = members.map(meanOf)
  const order = members.map((_, g) => g).sort((a, b) => members[b].length - members[a].length || centers[a][0] - centers[b][0])
  const rank = new Map(order.map((g, r) => [g, r]))
  const labels: number[] = new Array(input.points.length).fill(-1)
  kept.forEach((original, i) => { labels[original] = rank.get(best.s.labels[i]) as number })
  return {
    k: best.k,
    auto,
    labels,
    clusters: order.map((g, r) => ({ index: r, size: members[g].length, center: centers[g] })),
    score: Number.isFinite(best.score) ? best.score : null,
    skipped,
  }
}

export function isRefusal(result: KMeansResult | KMeansRefusal): result is KMeansRefusal {
  return "refused" in result
}
