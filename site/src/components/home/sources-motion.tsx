"use client";

import * as React from "react";
import { m, useTransform, type MotionValue } from "framer-motion";
import { useProgress } from "@/components/shared/progress-section";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * The sources section's client leaves (WEB-5 review fix R5): the headline
 * box's scroll fade and the falling pills. The headline, lead and caption
 * text arrive as server-rendered `children` or as plain strings.
 */

/** Fades the headline box in as the section arrives (opacity and travel). */
export function SourcesHeadingFade({ children }: { children: React.ReactNode }) {
  const reduced = useReducedMotionSafe();
  // 0 when the section's top meets the bottom of the viewport; 1 at the end of the pin.
  const scrollYProgress = useProgress();
  const headOpacity = useTransform(scrollYProgress, [0.04, 0.2], [0, 1]);
  const headY = useTransform(scrollYProgress, [0.04, 0.2], [48, 0]);
  return (
    <m.div
      // WEB-5 D5: scroll fades it in from opacity 0, which the server
      // HTML carries; `data-reveal` shows it in full without JavaScript.
      data-reveal
      // WEB-1 review round 1 SHOULD-FIX 5: on a phone the copy sits over the
      // glow of the mascot's eye seen through the water, so it gets a dark
      // backing there (body text stays above 4.5:1 on it).
      className="relative z-10 max-md:rounded-3xl max-md:bg-[#020b2a]/70 max-md:p-3"
      // Keyed on `reduced`: before motion's features load an `m` element keeps
      // the style it first rendered (review fix R1), so a switch must remount it.
      key={reduced ? "reduced" : "motion"}
      style={reduced ? { opacity: 1, y: 0 } : { opacity: headOpacity, y: headY }}
    >
      {children}
    </m.div>
  );
}

/** The pills, as the items of the server-rendered list. */
export function SourcePills({ items }: { items: readonly string[] }) {
  const reduced = useReducedMotionSafe();
  const progress = useProgress();
  return (
    <>
      {items.map((item, i) => (
        <SourcePill key={item} label={item} index={i} count={items.length} progress={progress} animate={!reduced} />
      ))}
    </>
  );
}

/**
 * Ease-out cubic: the droplet decelerates into its slot and stops there.
 * WEB-1 review round 1 SHOULD-FIX 1: this used to overshoot by about 10 % of
 * the fall (around 100 px on a desktop), which pushed the last pills down over
 * the caption. Transforms never change layout, so the grid itself already
 * holds its settled height; the pills just must not travel past it.
 */
const settle = (t: number): number => 1 - Math.pow(1 - t, 3);

/** Gap between two droplets' departures, so the last of `count` arrives by 0.80. */
export const stagger = (count: number): number => (count > 1 ? Math.min(0.034, 0.4 / (count - 1)) : 0);

const PILL_CLASS =
  "rounded-full border px-2.5 py-1 text-[11px] text-[#E8F4FF] md:px-4 md:py-2 md:text-[13px]";

function SourcePill({
  label,
  index,
  count,
  progress,
  animate,
}: {
  label: string;
  index: number;
  count: number;
  progress: MotionValue<number>;
  animate: boolean;
}) {
  if (!animate) {
    return <li className={`${PILL_CLASS} border-[#44A6FA]/30 bg-[#44A6FA]/10`}>{label}</li>;
  }
  return <FallingPill label={label} index={index} count={count} progress={progress} />;
}

function FallingPill({
  label,
  index,
  count,
  progress,
}: {
  label: string;
  index: number;
  count: number;
  progress: MotionValue<number>;
}) {
  // Staggered: the first droplet leaves at 0.10, the last arrives by 0.80.
  // WEB-5 T6: the step shrinks when the list grows (19 sources now), so the
  // last droplet still lands inside the pin; nine pills keep the old 0.034.
  const start = 0.1 + index * stagger(count);
  const end = start + 0.3;
  const dir = index % 2 === 0 ? -1 : 1;
  const y = useTransform(progress, [start, end], ["-110vh", "0vh"], { ease: settle });
  const x = useTransform(progress, [start, end], [`${dir * (36 + index * 9)}px`, "0px"], { ease: settle });
  const rotate = useTransform(progress, [start, end], [((index % 3) - 1) * 16, 0], { ease: settle });
  const opacity = useTransform(progress, [start, start + 0.06, end], [0, 0.55, 1]);
  const filter = useTransform(progress, [start, end - 0.05], ["blur(4px)", "blur(0px)"]);
  // Stretched while falling, compressed for a beat on arrival.
  const scaleY = useTransform(progress, [start, start + 0.08, end - 0.06, end], [1.5, 1.3, 1.0, 1]);
  const glow = useTransform(
    progress,
    [start, end - 0.02, end + 0.04],
    [
      "0 0 30px 8px rgba(68,166,250,0.65)",
      "0 0 18px 3px rgba(68,166,250,0.35)",
      "0 0 0px 0px rgba(68,166,250,0)",
    ],
  );
  const border = useTransform(
    progress,
    [start, end],
    ["rgba(124,196,255,0.9)", "rgba(68,166,250,0.3)"],
  );
  const fill = useTransform(progress, [start, end], ["rgba(68,166,250,0.35)", "rgba(68,166,250,0.1)"]);
  return (
    <m.li
      data-reveal
      className={PILL_CLASS}
      style={{ y, x, rotate, opacity, filter, scaleY, boxShadow: glow, borderColor: border, backgroundColor: fill }}
    >
      {label}
    </m.li>
  );
}
