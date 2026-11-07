"use client";

import * as React from "react";
import { m, useMotionValue, type MotionValue } from "framer-motion";

/**
 * WEB-5 O24/O33: the only client pieces of the final section's whale. The
 * whale and its light are server-rendered SVG and CSS (`whale-svg.tsx`,
 * `mascot-rising.tsx`), so they add nothing to the page's JavaScript; the
 * stage (`home/final-mascot.tsx`) shares its gaze and the element it moves
 * through this context, and these two leaves use them.
 */

export type FinalWhaleMotion = {
  /** The eyes' shift, in the whale's own units. */
  eyeX: MotionValue<number>;
  eyeY: MotionValue<number>;
  /** The element the stage moves to bring the whale in. */
  swimRef: React.RefObject<HTMLDivElement | null>;
};

const FinalWhaleContext = React.createContext<FinalWhaleMotion | null>(null);
export const FinalWhaleProvider = FinalWhaleContext.Provider;

/**
 * One of the whale's eyes (O29: a plain glowing white, the logo's eye),
 * shifted a little towards the stage's gaze. Without a stage it rests,
 * looking up.
 */
export function WhaleEye({ cx, cy, r, glow }: { cx: number; cy: number; r: number; glow: string }) {
  const ctx = React.useContext(FinalWhaleContext);
  const restX = useMotionValue(-1);
  const restY = useMotionValue(-3);
  return (
    <m.g style={{ x: ctx?.eyeX ?? restX, y: ctx?.eyeY ?? restY }}>
      <circle cx={cx} cy={cy} r={r * 2.3} fill={glow} />
      <circle cx={cx} cy={cy} r={r} fill="#ffffff" />
    </m.g>
  );
}

/** The layer the stage moves to bring the whale in. */
export function SwimLayer({ className, children }: { className?: string; children: React.ReactNode }) {
  const ctx = React.useContext(FinalWhaleContext);
  return (
    <div ref={ctx?.swimRef} className={className}>
      {children}
    </div>
  );
}
