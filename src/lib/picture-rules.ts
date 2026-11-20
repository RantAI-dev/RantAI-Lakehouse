/**
 * The rules of the workspace's three pictures (`ADM-2` decision 3), kept here
 * so the screen can say no BEFORE a request, in the API's own words, and
 * never say yes to something the API will refuse.
 *
 * Every rule here repeats one the API enforces (`rust/crates/lakehouse-api/
 * src/routes/settings.rs`: `LOGO_MAX_BYTES`, `ICON_MAX_BYTES`,
 * `sniff_picture_type` and `check_picture`). The API is the authority: it
 * reads the type from the file's first bytes and refuses what it refuses.
 * These rules only save the person a request the API would refuse.
 */

import type { BrandingImageKind } from "@/services/contracts/branding"

/**
 * The longest logo, 512 KB (the API's `LOGO_MAX_BYTES`, `settings.rs:2875`).
 * The API's sentence calls it "512 KB".
 */
export const LOGO_MAX_BYTES = 524_288

/**
 * The longest icon, 64 KB (the API's `ICON_MAX_BYTES`, `settings.rs:2878`).
 * The API's sentence calls it "64 KB".
 */
export const ICON_MAX_BYTES = 65_536

/**
 * What the API tells a person whose logo is not PNG, JPEG or WebP (its
 * `LOGO_TYPE_REFUSED`, `settings.rs:2848`).
 */
export const LOGO_TYPE_REFUSED = "The logo must be a PNG, JPEG or WebP image."

/**
 * What the API tells a person whose logo is over 512 KB (its
 * `LOGO_SIZE_REFUSED`, `settings.rs:2851`).
 */
export const LOGO_SIZE_REFUSED = "The logo must be at most 512 KB."

/**
 * What the API tells a person whose icon is not a PNG of at most 64 KB (its
 * `ICON_REFUSED`, `settings.rs:2854`). The API has one sentence for the icon,
 * for its type and for its size together.
 */
export const ICON_REFUSED = "The icon must be a PNG image of at most 64 KB."

/** The types the API sniffs for the logo and the dark-mode logo. */
const LOGO_TYPES: readonly string[] = ["image/png", "image/jpeg", "image/webp"]

/** The type the API sniffs for the icon: a PNG, and nothing else. */
const ICON_TYPES: readonly string[] = ["image/png"]

/**
 * The types each picture takes, for the picker's `accept`. The logo and the
 * dark-mode logo take the same three; the icon takes a PNG only. One list
 * for the whole console, so the picker cannot offer a type the rules refuse.
 */
export const PICTURE_TYPES: Record<BrandingImageKind, readonly string[]> = {
  logo: LOGO_TYPES,
  "logo-dark": LOGO_TYPES,
  icon: ICON_TYPES,
}

/**
 * What the API would answer for a file that breaks the rule of its kind, in
 * the API's own words, or `null` when the file may be sent.
 *
 * The order of the checks is the API's (`check_picture`, `settings.rs:2944`):
 * the type first, then the size. A logo of the wrong type that is also too
 * big is refused with the type sentence, as the API refuses it. The
 * dark-mode logo follows the logo's rules and the logo's sentences.
 */
export function pictureRefusal(kind: BrandingImageKind, file: File): string | null {
  const type = file.type.trim().toLowerCase()
  // The browser does not always know the type of a file: an empty type or
  // `application/octet-stream` says nothing about the bytes. The API sniffs
  // the first bytes and stays the authority, so nothing is refused on the
  // type here; only the size, which the browser does know, is judged.
  const known = type !== "" && type !== "application/octet-stream"
  if (kind === "icon") {
    if (known && !ICON_TYPES.includes(type)) return ICON_REFUSED
    if (file.size > ICON_MAX_BYTES) return ICON_REFUSED
    return null
  }
  if (known && !LOGO_TYPES.includes(type)) return LOGO_TYPE_REFUSED
  if (file.size > LOGO_MAX_BYTES) return LOGO_SIZE_REFUSED
  return null
}
