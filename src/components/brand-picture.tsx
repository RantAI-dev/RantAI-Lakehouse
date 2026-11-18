"use client"

import * as React from "react"

/**
 * One branding picture (`ADM-2`). A picture that fails to load falls back
 * to the product's own file, so the console never shows a broken image.
 */
export function BrandPicture({
  src,
  alt,
  fallbackSrc,
  className,
}: {
  /** The picture to show. */
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
