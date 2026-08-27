/**
 * Captures golden responses from the running TypeScript backend.
 *
 *   PUBLIC_DASH_TOKEN=<token> bun run rust/tests/parity/capture.ts http://localhost:3000
 *
 * The corpus is committed to git, and these responses come from a live system,
 * so everything sensitive is stripped HERE — in code, on every run — rather
 * than by hand afterwards. A hand-sanitized corpus silently re-leaks the next
 * time someone re-captures.
 *
 * Three mechanisms, applied in order:
 *   1. `SECRETS`  — known values swapped for stable placeholders both ways, so
 *                   requests can still use the real value while the corpus
 *                   never sees it.
 *   2. `SECRET_SHAPES` — defence in depth: anything *shaped* like a credential
 *                   (64-hex key, JWT) is replaced even if we did not know it
 *                   existed. This is what catches the secret nobody listed.
 *   3. `REDACT_TEXT_KEYS` — free-text keys carrying real conversation or model
 *                   output. Replaced with `<redacted:N>` length markers.
 *                   Structure is preserved, which is all parity compares.
 */
import { mkdir, writeFile } from "node:fs/promises"
import requests from "./requests.json"

type Req = {
  name: string
  method?: string
  path?: string
  body?: unknown
  raw?: string
  expectStatus?: number
  comment?: string
  skip?: boolean
}

const base = process.argv[2] ?? "http://localhost:3000"
const outDir = new URL("./corpus/", import.meta.url).pathname
await mkdir(outDir, { recursive: true })

// ── 1. Known secrets, resolved from the environment ─────────────────────────

const SECRETS: Record<string, string | undefined> = {
  __PUBLIC_DASH_TOKEN__: process.env.PUBLIC_DASH_TOKEN,
}

function resolveSecrets(value: string): string {
  let out = value
  for (const [marker, actual] of Object.entries(SECRETS)) {
    if (!out.includes(marker)) continue
    if (!actual) {
      throw new Error(
        `${marker} appears in requests.json but the matching env var is unset. ` +
          `Export it before capturing, e.g. PUBLIC_DASH_TOKEN=... bun run capture.ts`,
      )
    }
    out = out.split(marker).join(actual)
  }
  return out
}

// ── 2. Credential-shaped values, whether or not we knew about them ──────────

const SECRET_SHAPES: { pattern: RegExp; placeholder: string }[] = [
  // Public dashboard tokens: p_ + 32 hex.
  { pattern: /p_[0-9a-f]{32}/g, placeholder: "__PUBLIC_DASH_TOKEN__" },
  // HS256 signing keys and any other 32-byte hex secret (e.g. the embed
  // secret persisted in console.app_kv).
  { pattern: /\b[0-9a-f]{64}\b/g, placeholder: "__HEX64_SECRET__" },
  // Signed JWTs — three base64url segments. Holding one is holding access.
  {
    pattern: /\bey[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b/g,
    placeholder: "__JWT__",
  },
]

function stripSecretShapes(value: string): string {
  let out = value
  for (const { pattern, placeholder } of SECRET_SHAPES) {
    out = out.replace(pattern, placeholder)
  }
  return out
}

/** Reverses `SECRETS` and strips anything credential-shaped. */
function scrub(value: string): string {
  let out = value
  for (const [marker, actual] of Object.entries(SECRETS)) {
    if (actual) out = out.split(actual).join(marker)
  }
  return stripSecretShapes(out)
}

// ── 3. Free text that must not be persisted ─────────────────────────────────

/**
 * Captures whose bodies carry real conversation content or live model output.
 * Their text is non-deterministic across runs, so the parity harness could
 * never have asserted on it — redacting costs nothing.
 */
const REDACT_TEXT_IN = new Set([
  "ai-sessions-list",
  "ai-sessions-detail",
  "ai-chat-ok",
  "agent-ask-ok",
  "agent-query-ok",
  "agent-text-to-sql-ok",
])

/**
 * Keys whose string values are free text. `detail`, `explanation`, and
 * `assumptions` are included because model output reaches them too — an
 * earlier hand-sanitization pass missed exactly those.
 */
const REDACT_TEXT_KEYS = new Set([
  "content",
  "title",
  "answer",
  "text",
  "summary",
  "sql",
  "detail",
  "explanation",
  "assumptions",
  "question",
])

function redactText(value: unknown, key?: string): unknown {
  if (typeof value === "string") {
    if (key && REDACT_TEXT_KEYS.has(key) && value.length > 0) {
      return `<redacted:${value.length}>`
    }
    return value
  }
  if (Array.isArray(value)) return value.map((v) => redactText(v, key))
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([k, v]) => [k, redactText(v, k)]),
    )
  }
  return value
}

// ── Capture ─────────────────────────────────────────────────────────────────

let failures = 0
for (const r of requests as Req[]) {
  if (r.skip || !r.method || !r.path) {
    console.log(`skipped ${r.name}${r.comment ? ` — ${r.comment.slice(0, 80)}...` : ""}`)
    continue
  }

  const init: RequestInit = { method: r.method }
  if (r.raw !== undefined) {
    init.body = r.raw
    init.headers = { "Content-Type": "application/json" }
  } else if (r.body !== undefined) {
    init.body = JSON.stringify(r.body)
    init.headers = { "Content-Type": "application/json" }
  }

  const res = await fetch(`${base}${resolveSecrets(r.path)}`, init)
  const text = await res.text()

  let parsed: unknown
  try {
    parsed = JSON.parse(scrub(text))
  } catch {
    parsed = { __nonJsonBody: scrub(text) }
  }
  if (REDACT_TEXT_IN.has(r.name)) parsed = redactText(parsed)

  if (r.expectStatus !== undefined && res.status !== r.expectStatus) {
    console.error(`MISMATCH ${r.name}: expected ${r.expectStatus}, got ${res.status}`)
    failures += 1
  }

  await writeFile(
    `${outDir}${r.name}.json`,
    `${JSON.stringify(
      { request: r, status: res.status, contentType: res.headers.get("content-type"), body: parsed },
      null,
      2,
    )}\n`,
  )
  console.log(`captured ${r.name} → ${res.status}`)
}

if (failures > 0) {
  console.error(`${failures} request(s) did not match expectStatus`)
  process.exit(1)
}
