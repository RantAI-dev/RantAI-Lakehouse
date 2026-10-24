/**
 * Pure helpers for the Share dialog's signed-embedding section (SEC-12).
 * Kept apart from the component so the wording a customer copies into their
 * own code can be tested.
 */

/** A lifetime in the unit a person reads: "24 hours", "90 minutes", "45 seconds". */
export function formatLifetime(seconds: number): string {
  const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? "" : "s"}`
  if (seconds >= 3600 && seconds % 3600 === 0) return plural(seconds / 3600, "hour")
  if (seconds >= 60 && seconds % 60 === 0) return plural(seconds / 60, "minute")
  return plural(seconds, "second")
}

/** The lifetime the snippet suggests: ten minutes, or the limit when that is lower. */
export function suggestedLifetimeSeconds(maxLifetimeSeconds: number): number {
  return Math.min(600, maxLifetimeSeconds)
}

/**
 * The Node snippet the dialog offers. `maxLifetimeSeconds` comes from the
 * server (`embed-info`), never from a constant here: an administrator can
 * lower or raise the limit.
 */
export function signSnippet(input: {
  board: string
  origin: string
  maxLifetimeSeconds: number
  /** `BI-26`: sign a token for this one chart instead of the dashboard. */
  chart?: string
  /** `BI-26`: the filters in "locked" mode; the token must set every one (an omitted one is refused). */
  lockedFilters?: readonly string[]
  /** `BI-26`: the look to put in the address (`embedLookQuery`). */
  query?: string
}): string {
  const { board, origin, maxLifetimeSeconds, chart, query } = input
  const locked = input.lockedFilters ?? []
  const resource = chart ? `{ chart: "${chart}" }` : `{ dashboard: "${board}" }`
  const params = locked.length
    ? [`  params: {`, ...locked.map((c) => `    ${JSON.stringify(c)}: "<value for ${c}>", // locked: required, the viewer cannot change it`), `  },`]
    : [`  params: { /* locked filters, e.g. */ kawasan: "Jakarta Pusat" },`]
  return [
    `// Node — sign a per-viewer embed token (KEEP THE SECRET SERVER-SIDE)`,
    `import jwt from "jsonwebtoken";`,
    `import { randomUUID } from "node:crypto";`,
    `const iat = Math.floor(Date.now() / 1000);`,
    `const token = jwt.sign({`,
    `  resource: ${resource},`,
    ...params,
    `  iat,                   // required`,
    `  exp: iat + ${suggestedLifetimeSeconds(maxLifetimeSeconds)}, // required; exp - iat may be at most ${maxLifetimeSeconds} (${formatLifetime(maxLifetimeSeconds)})`,
    `  jti: randomUUID(),     // optional; lets you withdraw this one token later`,
    `}, EMBED_SECRET);`,
    `const url = "${origin}/embed/signed/" + token${query ? ` + "?${query}"` : ""};`,
  ].join("\n")
}

/** The look options of the embed address, as the query a snippet or a preview uses (`BI-26` D6). */
export function embedLookQuery(look: { theme: "light" | "dark" | "dashboard"; hideName: boolean; transparent: boolean }): string {
  const parts: string[] = []
  if (look.theme !== "dashboard") parts.push(`theme=${look.theme}`)
  if (look.hideName) parts.push("logo=none")
  if (look.transparent) parts.push("background=transparent")
  return parts.join("&")
}

/**
 * The host page's side of the height messages (`BI-26` D7): an `<iframe>` and a
 * listener of about ten lines. The listener accepts a message only from the
 * console's own origin, so a page that is not the embed cannot set the height.
 */
export function iframeSnippet(input: { url: string; origin: string }): string {
  return [
    `<iframe id="rantai" src="${input.url}" width="100%" height="400" frameborder="0" style="border:0"></iframe>`,
    `<script>`,
    `  const frame = document.getElementById("rantai");`,
    `  window.addEventListener("message", (event) => {`,
    `    if (event.origin !== "${input.origin}") return;        // only our embed`,
    `    if (event.source !== frame.contentWindow) return;`,
    `    const data = event.data;`,
    `    if (data && data.type === "rantai-embed:size" && Number.isFinite(data.height)) {`,
    `      frame.style.height = data.height + "px";`,
    `    }`,
    `  });`,
    `</script>`,
  ].join("\n")
}

/** The one sentence shown above the sample token. */
export function tokenRules(maxLifetimeSeconds: number): string {
  return `Tokens need iat and exp and may live at most ${formatLifetime(maxLifetimeSeconds)}.`
}

/** "just now", "5 minutes ago", "3 hours ago", "2 days ago" for a Unix-seconds instant. */
export function relativeTime(unixSeconds: number, nowMs: number = Date.now()): string {
  const seconds = Math.max(0, Math.floor(nowMs / 1000 - unixSeconds))
  if (seconds < 60) return "just now"
  const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? "" : "s"} ago`
  if (seconds < 3600) return plural(Math.floor(seconds / 60), "minute")
  if (seconds < 86_400) return plural(Math.floor(seconds / 3600), "hour")
  return plural(Math.floor(seconds / 86_400), "day")
}
