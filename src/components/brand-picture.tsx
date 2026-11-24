"use client"

import * as React from "react"

/**
 * One branding picture (`ADM-2`). A picture that fails to load falls back
 * to the product's own file, so the console never shows a broken image.
 *
 * A source that is empty means the branding answer is still on its way:
 * the slot is drawn empty then, and the product's own file does not stand
 * in for it (`adm-120`).
 */
export function BrandPicture({
  src,
  alt,
  fallbackSrc,
  className,
}: {
  /**
   * The picture to show. Empty while the branding answer is on its way:
   * nothing is drawn then, not the product's own picture.
   */
  src: string
  /** The picture's alternative text: the workspace's name. */
  alt: string
  /** The product's own file, shown when `src` fails to load. */
  fallbackSrc: string
  className?: string
}) {
  // The picture that failed is remembered by its own URL, so a newer
  // picture (a new branding version) is tried again instead of staying on
  // the fallback for the rest of the page's life.
  const [failedSrc, setFailedSrc] = React.useState<string | null>(null)
  // No picture yet: the slot is drawn empty, with the size the picture
  // would have had. An `<img>` with an empty source would ask the server
  // for the current page, and the product's own file must not stand in
  // here either: the workspace's logo would then land after a flash of
  // ours (`adm-120`).
  if (!src) {
    return <span className={className} aria-hidden />
  }
  const shown = failedSrc === src ? fallbackSrc : src
  return (
    // eslint-disable-next-line @next/next/no-img-element -- the picture is served by the API at /api/branding/images/<kind>, not a file under `public/`; `next/image` would proxy it through the console's image optimizer.
    <img
      src={shown}
      alt={alt}
      className={className}
      onError={() => {
        if (shown !== fallbackSrc) setFailedSrc(src)
      }}
    />
  )
}
