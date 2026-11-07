"use client";

import * as React from "react";
import dynamic from "next/dynamic";
import { QuietBoundary } from "@/components/shared/quiet-boundary";

const CalendarHover = dynamic(() => import("./calendar-hover"), { ssr: false });

export type CalendarHoverText = { days: string[]; week: string; unit: string };

/**
 * WEB-5 O18: the calendar's hover, fetched on the first pointer over the
 * tile (a `next/dynamic` chunk, like the scene and Lenis), so it adds
 * nothing to the first JavaScript but this host (WEB-5 D5).
 */
export function CalendarHoverHost(text: Readonly<CalendarHoverText>) {
  const ref = React.useRef<HTMLSpanElement>(null);
  const [wanted, setWanted] = React.useState(false);
  React.useEffect(() => {
    const tile = ref.current?.parentElement;
    if (!tile) return;
    const want = () => setWanted(true);
    tile.addEventListener("pointerenter", want, { once: true });
    return () => tile.removeEventListener("pointerenter", want);
  }, []);
  return (
    <span ref={ref} aria-hidden="true" className="contents">
      {wanted && (
        <QuietBoundary>
          <CalendarHover {...text} />
        </QuietBoundary>
      )}
    </span>
  );
}
