"use client";

import * as React from "react";

const QUERY = "(prefers-reduced-motion: reduce)";

function subscribe(onChange: () => void): () => void {
  const mq = window.matchMedia(QUERY);
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}

/**
 * `prefers-reduced-motion: reduce`, safe for server-rendered markup.
 *
 * motion's own `useReducedMotion()` reads the media query during the first
 * client render, so for a reduced-motion visitor the first client tree differs
 * from the server's and React reports a hydration mismatch. This hook returns
 * `false` for the server and for the hydration pass and the real value right
 * after, so the markup always matches and the page then adapts.
 *
 * Shared, not in `features/landing` where it started (WEB-15 review
 * BLOCKER 1): the console image leaves `src/features/landing` out
 * (`.dockerignore`, WEB-2), so the sign-in page could not import it from
 * there and the image's build failed.
 */
export function useReducedMotionSafe(): boolean {
  return React.useSyncExternalStore(
    subscribe,
    () => window.matchMedia(QUERY).matches,
    () => false,
  );
}
