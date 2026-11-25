/**
 * The rules of the workspace's three pictures (`ADM-2` decision 3), kept here
 * so the screen can say no BEFORE a request, in the API's own words, and
 * never say yes to something the API will refuse.
 *
 * The rule here is the API's rule: the console sniffs the same first bytes
 * the API sniffs (`sniff_picture_type` in
 * `rust/crates/lakehouse-api/src/routes/settings.rs`), so it refuses exactly
 * what the API refuses and accepts exactly what the API accepts. The file's
 * name and the browser's claim of its type are never read: the API does not
 * read them either, so a PNG named `logo.txt` is a PNG for both.
 *
 * The API stays the authority for what is stored. These rules only save the
 * person a request the API would refuse.
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

/** The media type the API names a PNG. */
const PNG_TYPE = "image/png"

/** The media type the API names a JPEG. */
const JPEG_TYPE = "image/jpeg"

/** The media type the API names a WebP. */
const WEBP_TYPE = "image/webp"

/** The first bytes of a PNG (the API's `sniff_picture_type`, `settings.rs:2888`). */
const PNG_MAGIC = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]

/** The first bytes of a JPEG (`settings.rs:2889`). */
const JPEG_MAGIC = [0xff, 0xd8, 0xff]

/** The first four bytes of a WebP file: `RIFF` (`settings.rs:2894`). */
const RIFF_MAGIC = [0x52, 0x49, 0x46, 0x46]

/** The bytes 8 to 12 of a WebP file: `WEBP` (`settings.rs:2894`). */
const WEBP_TAG = [0x57, 0x45, 0x42, 0x50]

/** How many bytes the sniffer reads: the WebP tag ends at byte 12. */
const HEAD_BYTES = 12

/**
 * The types each picture takes, for the picker's `accept`. The logo and the
 * dark-mode logo take the same three; the icon takes a PNG only. One list
 * for the whole console, so the picker cannot offer a type the rules refuse.
 */
export const PICTURE_TYPES: Record<BrandingImageKind, readonly string[]> = {
  logo: [PNG_TYPE, JPEG_TYPE, WEBP_TYPE],
  "logo-dark": [PNG_TYPE, JPEG_TYPE, WEBP_TYPE],
  icon: [PNG_TYPE],
}

/** Whether the bytes of the head at `at` are these bytes. */
function bytesAt(head: Uint8Array, at: number, bytes: readonly number[]): boolean {
  if (head.length < at + bytes.length) return false
  return bytes.every((byte, index) => head[at + index] === byte)
}

/**
 * The media type the first bytes of a file say it is, exactly as the API
 * sniffs it (`sniff_picture_type`, `settings.rs:2887-2899`): PNG, JPEG and
 * WebP, and nothing else. The file's name and the browser's claim are never
 * read; an SVG or a text file named `logo.png` has none of these bytes.
 */
function sniffPictureType(head: Uint8Array): string | null {
  if (bytesAt(head, 0, PNG_MAGIC)) return PNG_TYPE
  if (bytesAt(head, 0, JPEG_MAGIC)) return JPEG_TYPE
  if (bytesAt(head, 0, RIFF_MAGIC) && bytesAt(head, 8, WEBP_TAG)) return WEBP_TYPE
  return null
}

/**
 * What the API would answer for a file that breaks the rule of its kind, in
 * the API's own words, or `null` when the file may be sent.
 *
 * The type comes from the file's first bytes, as it does in the API, and the
 * order of the checks is the API's (`check_picture`, `settings.rs:2944`): the
 * type first, then the size. A file that is no picture is refused with the
 * type sentence even when it is small; a picture that is too big is refused
 * with the size sentence. The dark-mode logo follows the logo's rules and the
 * logo's sentences.
 */
export async function pictureRefusal(
  kind: BrandingImageKind,
  file: File
): Promise<string | null> {
  const head = new Uint8Array(await file.slice(0, HEAD_BYTES).arrayBuffer())
  const type = sniffPictureType(head)
  if (kind === "icon") {
    // The API has one sentence for the icon, for its type and for its size.
    if (type !== PNG_TYPE || file.size > ICON_MAX_BYTES) return ICON_REFUSED
    return null
  }
  // The sniffer answers with the three types the logo takes, or with none.
  if (type === null) return LOGO_TYPE_REFUSED
  if (file.size > LOGO_MAX_BYTES) return LOGO_SIZE_REFUSED
  return null
}
