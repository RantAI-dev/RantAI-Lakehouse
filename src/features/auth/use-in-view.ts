"use client";

/**
 * When the sign-in page's moving parts may move (`WEB-15`): only while they
 * are on screen and the tab is the one in front. Shared by the sample
 * dashboard's redraw loop (`login-showcase.tsx`) and the lake contours'
 * drift (`login-contours.tsx`), so the two pause and resume together.
 *
 * Its own module so the page can use it without importing the showcase,
 * which is code-split and must stay out of the page's first bundle.
 */
import * as React from "react";

function subscribeVisibility(onChange: () => void): () => void {
  document.addEventListener("visibilitychange", onChange);
  return () => document.removeEventListener("visibilitychange", onChange);
}

/** Whether the tab is the one in front (`document.visibilityState`); `true` on the server. */
export function usePageVisible(): boolean {
  return React.useSyncExternalStore(
    subscribeVisibility,
    () => document.visibilityState === "visible",
    () => true,
  );
}

/** Whether `ref`'s element is on screen (at least `threshold` of it), by IntersectionObserver; `false` until first measured. */
export function useOnScreen(ref: React.RefObject<HTMLElement | null>, threshold = 0): boolean {
  const [onScreen, setOnScreen] = React.useState(false);
  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => setOnScreen(e.isIntersecting), { threshold });
    io.observe(el);
    return () => io.disconnect();
  }, [ref, threshold]);
  return onScreen;
}
