import * as React from "react"

const MOBILE_BREAKPOINT = 768

/**
 * Returns whether the viewport is currently below the mobile breakpoint (768px).
 *
 * Subscribes to the matching media query so the value updates as the user resizes.
 * Returns `false` during SSR / first render before `useEffect` runs.
 * Used by the `Sidebar` UI component to switch between desktop and sheet variants.
 */
export function useIsMobile() {
  const [isMobile, setIsMobile] = React.useState<boolean | undefined>(undefined)

  React.useEffect(() => {
    const mql = window.matchMedia(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`)
    const onChange = () => {
      setIsMobile(window.innerWidth < MOBILE_BREAKPOINT)
    }
    mql.addEventListener("change", onChange)
    setIsMobile(window.innerWidth < MOBILE_BREAKPOINT)
    return () => mql.removeEventListener("change", onChange)
  }, [])

  return !!isMobile
}
