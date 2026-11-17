import { strict as assert } from "node:assert"
import { test } from "node:test"
import { isRefusal, kmeans, mulberry32, type KMeansResult } from "./kmeans"

const ok = (r: ReturnType<typeof kmeans>): KMeansResult => {
  assert.ok(!isRefusal(r), isRefusal(r) ? r.refused : "")
  return r
}

/** Three blobs of 20 points around well-separated centres, from the module's own PRNG. */
function blobs(): { points: number[][]; blob: number[] } {
  const rand = mulberry32(42)
  const centres = [[0, 0], [10, 1], [5, 12]]
  const points: number[][] = []
  const blob: number[] = []
  centres.forEach((c, b) => {
    for (let i = 0; i < 20; i++) { points.push([c[0] + rand() - 0.5, c[1] + rand() - 0.5]); blob.push(b) }
  })
  return { points, blob }
}

/** The labels, reduced to "which points share a label", so the numbering does not matter. */
const partition = (labels: number[]) => labels.map((l) => labels.indexOf(l))

test("three well-separated blobs give k = 3 and every point lands with its blob", () => {
  const { points, blob } = blobs()
  const r = ok(kmeans({ points, k: "auto" }))
  assert.equal(r.k, 3)
  assert.equal(r.auto, true)
  assert.deepEqual(partition(r.labels), partition(blob))
  assert.deepEqual(r.clusters.map((c) => c.size), [20, 20, 20])
})

test("the centres are in the original units, not the scaled ones", () => {
  const { points } = blobs()
  const r = ok(kmeans({ points, k: "auto" }))
  const centres = r.clusters.map((c) => c.center.map((v) => Math.round(v) + 0)).sort((a, b) => a[0] - b[0])
  assert.deepEqual(centres, [[0, 0], [5, 12], [10, 1]])
})

test("a fixed k is honoured, even when auto would choose another", () => {
  const { points } = blobs()
  for (const k of [2, 4, 5]) {
    const r = ok(kmeans({ points, k }))
    assert.equal(r.k, k)
    assert.equal(r.auto, false)
    assert.equal(new Set(r.labels).size, k)
  }
})

test("the same input gives the same labels every time, and a fixed k matches what auto found for it", () => {
  const { points } = blobs()
  const a = ok(kmeans({ points, k: "auto" }))
  assert.deepEqual(a, ok(kmeans({ points, k: "auto" })))
  assert.deepEqual(a.labels, ok(kmeans({ points, k: 3 })).labels)
})

test("clusters are numbered from the largest", () => {
  const rand = mulberry32(5)
  const points = [
    ...Array.from({ length: 5 }, () => [rand(), rand()]),
    ...Array.from({ length: 15 }, () => [20 + rand(), 20 + rand()]),
  ]
  const r = ok(kmeans({ points, k: 2 }))
  assert.deepEqual(r.clusters.map((c) => c.size), [15, 5])
  assert.equal(r.labels[0], 1)
  assert.equal(r.labels[19], 0)
})

test("fewer than 6 points is refused with a sentence that gives the count", () => {
  assert.deepEqual(kmeans({ points: [[1, 1], [2, 2], [3, 3], [4, 4], [5, 5]], k: "auto" }), {
    refused: "Clusters need at least 6 points; this chart has 5.",
  })
})

test("points with a measure that is not a number are left out and counted, not labelled", () => {
  const { points } = blobs()
  const withBad = [...points, [Number.NaN, 1], [1, Number.POSITIVE_INFINITY]]
  const r = ok(kmeans({ points: withBad, k: "auto" }))
  assert.equal(r.skipped, 2)
  assert.deepEqual(r.labels.slice(-2), [-1, -1])
  assert.equal(r.k, 3)
  assert.ok(isRefusal(kmeans({ points: [[Number.NaN, 1], ...points.slice(0, 4)], k: "auto" })))
})

test("a measure in millions does not decide the groups on its own", () => {
  const rand = mulberry32(9)
  const points: number[][] = []
  const group: number[] = []
  for (let i = 0; i < 40; i++) {
    const g = i % 2
    // x: revenue, spread over a million with no structure; y: two tight groups.
    points.push([1_000_000 + rand() * 1_000_000, g + (rand() - 0.5) * 0.1])
    group.push(g)
  }
  const r = ok(kmeans({ points, k: 2 }))
  assert.deepEqual(partition(r.labels), partition(group))
})

test("a measure that never changes plays no part", () => {
  const { points, blob } = blobs()
  const r = ok(kmeans({ points: points.map((p) => [...p, 7]), k: "auto" }))
  assert.deepEqual(partition(r.labels), partition(blob))
})

test("a k that the points cannot support is refused, never changed", () => {
  const { points } = blobs()
  assert.deepEqual(kmeans({ points: points.slice(0, 8), k: 4 }), {
    refused: "4 clusters need at least 12 points, 3 in each; this chart has 8.",
  })
  assert.ok(isRefusal(kmeans({ points, k: 1 })))
  assert.ok(isRefusal(kmeans({ points, k: 11 })))
  assert.ok(isRefusal(kmeans({ points, k: 2.5 })))
})

test("points that are all the same have nothing to split and are refused", () => {
  const r = kmeans({ points: Array.from({ length: 12 }, () => [3, 3]), k: "auto" })
  assert.ok(isRefusal(r))
})

test("mulberry32 gives the same stream for a seed on every machine", () => {
  const a = mulberry32(1)
  const first = [a(), a(), a()]
  const b = mulberry32(1)
  assert.deepEqual(first, [b(), b(), b()])
  first.forEach((v) => assert.ok(v >= 0 && v < 1))
  // First three outputs for seed 1, computed independently with masked 32-bit integer arithmetic in Python.
  assert.ok(Math.abs(first[0] - 0.6270739405881613) < 1e-15)
  assert.ok(Math.abs(first[1] - 0.002735721180215478) < 1e-15)
  assert.ok(Math.abs(first[2] - 0.5274470399599522) < 1e-15)
})
