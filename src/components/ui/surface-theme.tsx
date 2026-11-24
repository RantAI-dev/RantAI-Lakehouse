"use client"

import * as React from "react"

/**
 * WEB-10: a popup opened from a surface painted with `.theme-light` is
 * portalled to `<body>`, outside that surface's DOM subtree. CSS custom
 * properties and `color-scheme` inherit from the element a node is mounted
 * under, so the popup used to take the console's dark tokens and read dark on
 * a light dashboard (`BI-17` D6, `WEB-9` D6).
 *
 * The provider carries the theme *class*, not a DOM container. The portal
 * stays on `<body>` — moving it inside a dashboard pane would clip it with
 * `overflow-hidden` and put it under the pane's stacking context — and every
 * portaled popup copies the class onto its own element instead.
 * `design-system/tokens/colors.css` declares `:root, .theme-light` in one
 * block, so the class activates every light token on the popup and its
 * children, and `app/globals.css` gives it `color-scheme: light`.
 */
const SurfaceThemeContext = React.createContext<string | undefined>(undefined)

/** The theme class a portaled popup must copy, or `undefined` for the console's. */
export function useSurfaceThemeClass(): string | undefined {
  return React.useContext(SurfaceThemeContext)
}

/**
 * Wraps a surface (a dashboard, an embed, a render view) so popups opened
 * inside it follow its theme. `themeClass` comes from `dashboardLook`, the
 * single owner of a surface's look. React context crosses portals, so the
 * popup re-reads the class when the surface switches theme while it is open.
 */
export function SurfaceThemeProvider({
  themeClass,
  children,
}: {
  readonly themeClass: string | undefined
  readonly children: React.ReactNode
}) {
  return (
    <SurfaceThemeContext.Provider value={themeClass}>
      {children}
    </SurfaceThemeContext.Provider>
  )
}
