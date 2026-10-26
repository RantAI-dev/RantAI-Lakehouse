/**
 * Reading the head of a GeoJSON file in the browser (`BI-16` part C2b): the
 * administrator chooses which property holds each region's name, and the
 * choices are the properties of the file's first feature. A file may be 50 MB,
 * so only its first `HEAD_BYTES` are read; the server validates the whole file
 * and has the last word.
 */
export const MAX_MAP_BYTES = 50 * 1024 * 1024
/** What the server simplifies an uploaded file to for drawing (feature decision 4). */
export const DRAWN_MAP_BYTES = 5 * 1024 * 1024
export const HEAD_BYTES = 1024 * 1024

/** From `text[start]`, which must be `{`, the end index of the object (string- and escape-aware); -1 when it is cut off. */
function objectEnd(text: string, start: number): number {
  let depth = 0
  let inString = false
  for (let i = start; i < text.length; i++) {
    const c = text[i]
    if (inString) {
      if (c === "\\") i++
      else if (c === '"') inString = false
    } else if (c === '"') inString = true
    else if (c === "{") depth++
    else if (c === "}" && --depth === 0) return i
  }
  return -1
}

export type HeadRead =
  | { ok: true; properties: Record<string, unknown>; keys: string[] }
  | { ok: false; reason: string }

/** The properties of the first feature in the head of a GeoJSON text. */
export function firstFeatureProperties(head: string): HeadRead {
  if (!/"FeatureCollection"/.test(head)) return { ok: false, reason: "This does not look like a GeoJSON FeatureCollection." }
  const features = head.search(/"features"\s*:\s*\[/)
  if (features < 0) return { ok: false, reason: "The file has no features list." }
  const afterFeatures = head.slice(features)
  const at = afterFeatures.search(/"properties"\s*:\s*\{/)
  if (at < 0) return { ok: false, reason: "The first feature has no properties to take a name from." }
  const open = afterFeatures.indexOf("{", at)
  const close = objectEnd(afterFeatures, open)
  if (close < 0) return { ok: false, reason: "The first feature's properties could not be read." }
  try {
    const properties = JSON.parse(afterFeatures.slice(open, close + 1)) as Record<string, unknown>
    return { ok: true, properties, keys: Object.keys(properties) }
  } catch {
    return { ok: false, reason: "The first feature's properties could not be read." }
  }
}

const NAME_WORDS = ["name", "nama", "shapename", "name_en", "admin", "province", "provinsi", "region", "kabupaten", "nm_kab", "wadmpr"]

/** The property most likely to hold the region's name: a known word, else the first text property. */
export function guessNameProperty(properties: Record<string, unknown>): string {
  const keys = Object.keys(properties)
  const known = NAME_WORDS.map((w) => keys.find((k) => k.toLowerCase() === w)).find((k) => k !== undefined)
  return known ?? keys.find((k) => typeof properties[k] === "string" && properties[k] !== "") ?? keys[0] ?? ""
}

/** The map's name suggested from the file's: `provinces-2024.geojson` is "provinces 2024". */
export function nameFromFile(fileName: string): string {
  return fileName.replace(/\.(geo)?json$/i, "").replace(/[_-]+/g, " ").trim()
}

/** Why this file will not be sent, before it is read; `null` when it can be. */
export function fileProblem(file: { name: string; size: number }): string | null {
  if (!/\.(geo)?json$/i.test(file.name)) return "Choose a GeoJSON file (.geojson or .json)."
  if (file.size === 0) return "The file is empty."
  if (file.size > MAX_MAP_BYTES) return "This file is over 50 MB. Simplify it or split it, then upload again."
  return null
}
