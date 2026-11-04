"use client";

import { domAnimation, LazyMotion } from "motion/react";

/**
 * WEB-5 D5: the page's animations use `m.*` components with only the
 * `domAnimation` features (animate, exit, variants, hover, in-view),
 * instead of `motion.*`, which bundles every feature (layout, drag) into
 * the first JavaScript. `strict` makes a stray `motion.*` an error at
 * runtime rather than a silent return of the full bundle.
 */
export function MotionFeatures({ children }: { children: React.ReactNode }) {
  return (
    <LazyMotion features={domAnimation} strict>
      {children}
    </LazyMotion>
  );
}
