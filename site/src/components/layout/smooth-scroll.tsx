"use client";

import * as React from "react";
import dynamic from "next/dynamic";
import { afterPaint } from "@/lib/after-paint";
import { QuietBoundary } from "@/components/shared/quiet-boundary";

const SmoothScrollEngine = dynamic(() => import("./smooth-scroll-engine"), { ssr: false });

/**
 * WEB-5 D16: smooth wheel scrolling with Lenis, as the RantAI site does it
 * (`components/layout/smooth-scroll.tsx`, `hooks/use-desktop-motion.ts`):
 * only on a desktop with a mouse or trackpad, never under reduced motion,
 * and never on phones and tablets, which keep their native, inertial touch
 * scrolling. Leaving the query (a narrower window, reduced motion switched
 * on) takes Lenis off again.
 *
 * Lenis is not in the first JavaScript: it is a `next/dynamic` chunk
 * fetched once the page is usable (`afterPaint`), and only when the query
 * matches, so a phone never downloads it (D5, checked by `bun run size`
 * and in the P2 evidence). If the chunk fails, the page keeps native
 * scrolling (`QuietBoundary`).
 */
export const DESKTOP_MOTION =
  "(min-width: 768px) and (hover: hover) and (pointer: fine) and (prefers-reduced-motion: no-preference)";

function subscribeQuery(onChange: () => void): () => void {
  const mq = window.matchMedia(DESKTOP_MOTION);
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}

let painted = false;
const paintListeners = new Set<() => void>();
let paintScheduled = false;
function subscribePainted(onChange: () => void): () => void {
  paintListeners.add(onChange);
  if (!paintScheduled) {
    paintScheduled = true;
    afterPaint(() => {
      painted = true;
      for (const l of paintListeners) l();
    });
  }
  return () => {
    paintListeners.delete(onChange);
  };
}

export function SmoothScroll() {
  const desktop = React.useSyncExternalStore(subscribeQuery, () => window.matchMedia(DESKTOP_MOTION).matches, () => false);
  const ready = React.useSyncExternalStore(subscribePainted, () => painted, () => false);
  if (!desktop || !ready) return null;
  return (
    <QuietBoundary>
      <SmoothScrollEngine />
    </QuietBoundary>
  );
}
