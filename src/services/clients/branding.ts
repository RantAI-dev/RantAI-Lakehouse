import type { Branding, BrandingImageKind, BrandingService } from "../contracts/branding"
import { ServiceError, deniedFor } from "../errors"
import { apiFetch } from "../http"

/**
 * The public branding over `/api/branding` (`ADM-2`, plan decision 5). The
 * route needs no session: the sign-in page shows the name and the logo before
 * any session exists. An error body's text is the route's own fixed sentence,
 * so it is safe to show; a refusal goes through `deniedFor` (`errors.ts`).
 */

/** The public branding route. */
const BRANDING_PATH = "/api/branding"

/**
 * The URL of one stored picture (`ADM-2`). The `version` busts the browser's
 * cache: the server answers 304 for the same bytes and a new picture for new
 * ones.
 */
export function brandingImageUrl(kind: BrandingImageKind, version?: string): string {
  const path = `${BRANDING_PATH}/images/${encodeURIComponent(kind)}`
  return version ? `${path}?v=${encodeURIComponent(version)}` : path
}

export const brandingService: BrandingService = {
  get: async (signal) => {
    const res = await apiFetch(BRANDING_PATH, { signal })
    const json = await res.json().catch(() => null)
    if (!res.ok) {
      // WEB-12: a refusal goes through deniedFor (errors.ts).
      const denied = deniedFor(res.status, json)
      if (denied) throw denied
      const kind = res.status >= 500 ? "unavailable" : "invalid_request"
      throw new ServiceError(kind, json?.error ?? `Failed (${res.status})`, res.status)
    }
    return json as Branding
  },
  imageUrl: brandingImageUrl,
}
