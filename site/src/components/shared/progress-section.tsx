"use client";

import * as React from "react";
import type { MotionValue } from "framer-motion";
import { useSectionProgress, type ProgressRange } from "@/hooks/use-section-progress";

/**
 * A `<section>` that measures its own scroll progress and hands it to the
 * client leaves inside it (WEB-5 review fix R5). It lets a section's frame
 * and text stay server components: the headings, leads, lists and captions
 * arrive as `children` rendered on the server, and only the pieces that
 * move with the scroll (`useProgress`) are hydrated. The element it renders
 * is the one the section rendered itself before, so the DOM is unchanged.
 */
const ProgressContext = React.createContext<MotionValue<number> | null>(null);

export function ProgressSection({
  range,
  children,
  ...rest
}: { range: ProgressRange; children: React.ReactNode } & React.HTMLAttributes<HTMLElement>) {
  const ref = React.useRef<HTMLElement>(null);
  const progress = useSectionProgress(ref, range);
  return (
    <section ref={ref} {...rest}>
      <ProgressContext.Provider value={progress}>{children}</ProgressContext.Provider>
    </section>
  );
}

/** The enclosing `ProgressSection`'s progress, 0..1. */
export function useProgress(): MotionValue<number> {
  const value = React.useContext(ProgressContext);
  if (!value) throw new Error("useProgress must be used inside a ProgressSection");
  return value;
}
