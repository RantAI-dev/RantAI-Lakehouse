"use client";

import * as React from "react";
import { SlidersHorizontal } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { PHONE_MAX_WIDTH } from "@/lib/dashboard-tabs";
import { cn } from "@/lib/utils";

/**
 * The row of grouping, parameter and filter controls (BI-18 part C review fix
 * (SHOULD-FIX) R5). On a wide dashboard it is the row it always was. Narrower
 * than `PHONE_MAX_WIDTH`, the width test the grid uses, it collapses to one
 * "Filters · N" button (N: the filters in force and the parameters set) that
 * opens a sheet holding the same controls, unchanged: at 390 px they wrapped
 * into three rows above the tabs. Required filters are still enforced by the
 * controls and the server, wherever they are shown.
 */
export function FilterRow({ count, children }: { readonly count: number; readonly children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  const [width, setWidth] = React.useState(0);
  React.useLayoutEffect(() => {
    if (ref.current) setWidth(ref.current.getBoundingClientRect().width);
  }, []);
  React.useEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver((e) => setWidth(e[0].contentRect.width));
    ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);
  const narrow = width > 0 && width < PHONE_MAX_WIDTH;
  return (
    <div ref={ref} data-print-hide>
      {narrow ? (
        <Sheet>
          <SheetTrigger render={<Button variant="outline" size="sm" className="w-full justify-start" />}>
            <SlidersHorizontal className="size-4" aria-hidden /> {count > 0 ? `Filters · ${count}` : "Filters"}
          </SheetTrigger>
          <SheetContent side="bottom" className="max-h-[85vh] overflow-y-auto p-4">
            <SheetHeader className="p-0"><SheetTitle>Filters</SheetTitle></SheetHeader>
            <div className="flex flex-col items-start gap-3">{children}</div>
          </SheetContent>
        </Sheet>
      ) : (
        <div className={cn("flex flex-wrap items-center gap-3 rounded-lg border border-border bg-card/50 px-3 py-2")}>{children}</div>
      )}
    </div>
  );
}
