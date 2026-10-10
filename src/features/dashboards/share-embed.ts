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
export function signSnippet(input: { board: string; origin: string; maxLifetimeSeconds: number }): string {
  const { board, origin, maxLifetimeSeconds } = input
  return [
    `// Node — sign a per-viewer embed token (KEEP THE SECRET SERVER-SIDE)`,
    `import jwt from "jsonwebtoken";`,
    `import { randomUUID } from "node:crypto";`,
    `const iat = Math.floor(Date.now() / 1000);`,
    `const token = jwt.sign({`,
    `  resource: { dashboard: "${board}" },`,
    `  params: { /* locked filters, e.g. */ kawasan: "Jakarta Pusat" },`,
    `  iat,                   // required`,
    `  exp: iat + ${suggestedLifetimeSeconds(maxLifetimeSeconds)}, // required; exp - iat may be at most ${maxLifetimeSeconds} (${formatLifetime(maxLifetimeSeconds)})`,
    `  jti: randomUUID(),     // optional; lets you withdraw this one token later`,
    `}, EMBED_SECRET);`,
    `const url = "${origin}/embed/signed/" + token;`,
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
