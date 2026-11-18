/**
 * The public branding answer (`GET /api/branding`, `ADM-2` decision 5): the
 * workspace's own name and the three pictures everybody sees.
 *
 * The route is public on purpose: the sign-in page shows the name and the logo
 * before any session exists. It answers the name and each picture's type and
 * version, never the bytes and never who saved them.
 */

/** Which of the three pictures a picture is (`ADM-2` decision 5). */
export type BrandingImageKind = "logo" | "logo-dark" | "icon"

/**
 * One picture as the public answer shows it: its type and a version that
 * changes with its bytes, so the console can bypass a cached older picture.
 */
export type BrandingPicture = {
  /** The stored media type, e.g. `image/png`. */
  type: string
  /** A version that changes with the bytes; it is the picture's `ETag`. */
  version: string
}

/**
 * `GET /api/branding`: the workspace's own name and each picture's type and
 * version. A picture that is not set is `null`.
 */
export type Branding = {
  /** The workspace's own name, or `null` when the product's own name is shown. */
  workspaceName: string | null
  /** The logo, or `null` when none is set. */
  logo: BrandingPicture | null
  /** The dark-mode logo, or `null` when none is set. */
  logoDark: BrandingPicture | null
  /** The browser-tab icon, or `null` when none is set. */
  icon: BrandingPicture | null
}

/**
 * The console's view of the workspace's branding (`ADM-2`). The route is
 * public, so reading it needs no session.
 */
export interface BrandingService {
  /** The public branding answer: the name and each picture's type and version. */
  get(signal?: AbortSignal): Promise<Branding>
  /**
   * The URL of one stored picture (`GET /api/branding/images/<kind>`). The
   * `version` busts the browser's cache: the server answers 304 for the same
   * bytes and a new picture for new ones.
   */
  imageUrl(kind: BrandingImageKind, version?: string): string
}
