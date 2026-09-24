import * as React from "react"

/**
 * Current viewport width in pixels, or `null` before the first
 * measurement (SSR and the first client render).
 *
 * `useIsMobile` answers one fixed breakpoint; this returns the raw width
 * so a caller can make several decisions from it — the Data Explorer
 * drops three different columns at three different widths, which would
 * otherwise need three media-query hooks.
 *
 * The `null` is deliberate rather than a `0` or a guessed default:
 * callers can tell "not measured yet" apart from "genuinely narrow" and
 * render the full layout until they know better, instead of flashing a
 * stripped-down one on first paint.
 */
export function useWindowWidth(): number | null {
  const [width, setWidth] = React.useState<number | null>(null)

  React.useEffect(() => {
    const onResize = () => setWidth(window.innerWidth)
    onResize()
    window.addEventListener("resize", onResize)
    return () => window.removeEventListener("resize", onResize)
  }, [])

  return width
}
