"use client";

import * as React from "react";

import { PHONE_MAX_WIDTH } from "@/lib/dashboard-tabs";
import { cn } from "@/lib/utils";

/**
 * The row of grouping, parameter and filter chips (`BI-61` T3, layout A): a
 * quiet row with no box around it, beside the tabs. On a wide dashboard the
 * chips wrap at the right. Narrower than `PHONE_MAX_WIDTH`, the width test
 * the grid uses, they stay on one line that scrolls sideways, as on mockup
 * A's phone: BI-18 part C review fix (SHOULD-FIX) R5 found them wrapping into
 * three rows above the tabs at 390 px, which a single scrolling line also
 * avoids, and each chip still opens its own panel there. Required filters are
 * still enforced by the controls and the server, wherever they are shown.
 */
export function FilterRow({ children, className, align = "end" }: {
  readonly children: React.ReactNode;
  readonly className?: string;
  /**
   * Where the chips sit on a wide line: `end` beside the tabs, `start` on a
   * dashboard with no tabs, where the right-hand chips left the line's start
   * empty (BI-61 review round 1, SHOULD-FIX 5).
   */
  readonly align?: "start" | "end";
}) {
  const ref = React.useRef<HTMLDivElement>(null);
  const [width, setWidth] = React.useState(0);
  // The width of the line the row shares with the tabs, not the row's own: beside
  // the tabs the row is narrow on any screen.
  React.useLayoutEffect(() => {
    const line = ref.current?.parentElement ?? ref.current;
    if (line) setWidth(line.getBoundingClientRect().width);
  }, []);
  React.useEffect(() => {
    const line = ref.current?.parentElement ?? ref.current;
    if (!line) return;
    const ro = new ResizeObserver((e) => setWidth(e[0].contentRect.width));
    ro.observe(line);
    return () => ro.disconnect();
  }, []);
  const narrow = width > 0 && width < PHONE_MAX_WIDTH;
  return (
    <div ref={ref} data-print-hide className={cn("min-w-0", className)}>
      <div
        data-layout={narrow ? "scroll" : "wrap"}
        data-align={align}
        className={cn(
          "flex items-center gap-1.5",
          narrow
            ? "-mx-4 flex-nowrap overflow-x-auto px-4 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
            : cn("flex-wrap", align === "end" ? "justify-end" : "justify-start"),
        )}
      >
        {children}
      </div>
    </div>
  );
}
