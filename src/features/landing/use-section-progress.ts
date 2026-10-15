"use client";

import * as React from "react";
import { useMotionValue, type MotionValue } from "motion/react";
import { clamp01 } from "./scroll-timeline";

/**
 * Scroll progress of one section as a MotionValue, computed from `scrollY`
 * in plain JavaScript.
 *
 * Why not `useScroll({ target, offset })`: motion 13 hands scroll-linked
 * opacity, filter and transform to the browser's native ViewTimeline when it
 * can. Measured here, with a tall section holding a `position: sticky` stage,
 * the native path left opacity and blur at their first values while the
 * transform moved, so scroll-driven fades never showed. This hook gives the
 * same 0..1 numbers on every browser, and it is the same arithmetic the 3D
 * engine uses (`scroll-timeline.ts`), so the DOM and the canvas cannot drift.
 *
 * `range` mirrors motion's offset pairs:
 * - `"start end"`:   0 when the section's top meets the viewport's bottom
 * - `"start start"`: 0 when the section's top meets the viewport's top
 * - `"end end"`:     1 when the section's bottom meets the viewport's bottom
 * - `"end start"`:   1 when the section's bottom meets the viewport's top
 */
export type ProgressRange = readonly ["start end" | "start start", "end end" | "end start"];

export function useSectionProgress(
  ref: React.RefObject<HTMLElement | null>,
  range: ProgressRange,
): MotionValue<number> {
  const value = useMotionValue(0);
  const [from, to] = range;

  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let top = 0;
    let height = 0;
    let vh = window.innerHeight;

    const update = () => {
      const start = from === "start end" ? top - vh : top;
      const end = to === "end end" ? top + height - vh : top + height;
      const span = end - start;
      value.set(span <= 0 ? 0 : clamp01((window.scrollY - start) / span));
    };
    const measure = () => {
      const r = el.getBoundingClientRect();
      top = r.top + window.scrollY;
      height = r.height;
      vh = window.innerHeight;
      update();
    };

    measure();
    window.addEventListener("scroll", update, { passive: true });
    window.addEventListener("resize", measure);
    // Fonts and images change sections above this one, which moves it.
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    ro.observe(document.body);
    return () => {
      window.removeEventListener("scroll", update);
      window.removeEventListener("resize", measure);
      ro.disconnect();
    };
  }, [ref, from, to, value]);

  return value;
}
