"use client";

import * as React from "react";
import dynamic from "next/dynamic";
import { QuietBoundary } from "@/components/shared/quiet-boundary";

const BentoFx = dynamic(() => import("./bento-fx"), { ssr: false });

/**
 * WEB-5 owner QA O37: the bento's pointer effects (spotlight, border glow,
 * particles, ripple) load as a chunk on the first pointer over the grid,
 * like the calendar tile's hover, so the first JavaScript carries only
 * this host (WEB-5 D5).
 */
export function BentoFxHost() {
  const ref = React.useRef<HTMLSpanElement>(null);
  const [wanted, setWanted] = React.useState(false);
  React.useEffect(() => {
    const grid = ref.current?.parentElement;
    if (!grid) return;
    const want = () => setWanted(true);
    grid.addEventListener("pointerenter", want, { once: true });
    return () => grid.removeEventListener("pointerenter", want);
  }, []);
  return (
    <span ref={ref} aria-hidden="true" className="contents">
      {wanted && (
        <QuietBoundary>
          <BentoFx />
        </QuietBoundary>
      )}
    </span>
  );
}
