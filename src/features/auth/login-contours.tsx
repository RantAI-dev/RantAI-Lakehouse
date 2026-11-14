"use client";

/**
 * The lake behind the sign-in page's preview (`WEB-15` T1, owner 2026-11-14:
 * the panel "kosong banget"): depth lines of a lake, as on a bathymetric map,
 * densest around the dashboard card and fading out toward the panel's edges.
 *
 * The lines are three static drawings of the same lake at water levels a
 * third of a step apart (`public/login-contours-*.svg`, drawn by
 * `scripts/login-contours.mjs`), used as CSS masks over the theme's line
 * colour, so they are server-rendered markup, cost no JavaScript to draw, and
 * stay 1 px at every panel size. With motion allowed the three take turns
 * over 36 s and the rings creep slowly outward, like water; the drift runs
 * only while the panel is on screen and the tab is in front, and with
 * `prefers-reduced-motion` (or before the page's script runs) the first
 * drawing stands still. Styles: `login.css`.
 */
import * as React from "react";
import { useOnScreen, usePageVisible } from "./use-in-view";

export function LoginContours() {
  const ref = React.useRef<HTMLDivElement>(null);
  const onScreen = useOnScreen(ref);
  const pageVisible = usePageVisible();
  return (
    <div ref={ref} aria-hidden data-playing={onScreen && pageVisible ? "" : undefined} className="login-contours">
      <span className="login-contour" data-phase="0" />
      <span className="login-contour" data-phase="1" />
      <span className="login-contour" data-phase="2" />
    </div>
  );
}
