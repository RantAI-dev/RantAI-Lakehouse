/**
 * The hosts `next dev` serves its own resources to (`allowedDevOrigins`).
 *
 * `BI-1 QA fix`: the PDF is made by a headless Chrome in the Gotenberg
 * container that opens the console at `CONSOLE_INTERNAL_URL`, which on a dev
 * machine is `http://127.0.0.1:<port>`. Next.js 16 treats `127.0.0.1` as a
 * foreign host (its default is `localhost` only), refuses the dev server's
 * `/_next/hmr` websocket from it, and in dev the page then never hydrates:
 * no effect runs, `POST /api/render/data` is never sent, the readiness flag is
 * never set and the API gives up with "the PDF was not ready in time"
 * (measured with a headless Chrome on `127.0.0.1` against `localhost`).
 * Loopback is not a LAN exposure, so it is always allowed; the env list adds
 * other hosts (OPS-11). Affects `next dev` only: a production build ignores it.
 */
export const LOOPBACK_DEV_ORIGINS = ["127.0.0.1"] as const

export function devOrigins(envList: string | undefined): string[] {
  const extra = (envList ?? "")
    .split(",")
    .map((origin) => origin.trim())
    .filter((origin) => origin.length > 0)
  return [...new Set([...LOOPBACK_DEV_ORIGINS, ...extra])]
}
