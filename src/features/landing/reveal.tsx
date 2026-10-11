"use client";

import * as React from "react";
import { motion } from "motion/react";
import { cn } from "@/lib/utils";
import { useReducedMotionSafe } from "./use-reduced-motion";

/**
 * Scroll-into-view reveals shared by the sections.
 *
 * With `prefers-reduced-motion` they are fades only: no travel, no blur,
 * no stagger (plan §2.8). Every revealed node carries `data-reveal` so the
 * `<noscript>` rule in `app/landing/page.tsx` can show it without JavaScript.
 */

/** A cubic ease-out with a long tail; the page's default "arrive" curve. */
export const EASE_OUT = [0.16, 1, 0.3, 1] as const;

export function Reveal({
  children,
  className,
  delay = 0,
  y = 28,
  as = "div",
}: {
  children: React.ReactNode;
  className?: string;
  delay?: number;
  y?: number;
  as?: "div" | "p" | "li" | "ul" | "span";
}) {
  const reduced = useReducedMotionSafe();
  const Tag = motion[as] as typeof motion.div;
  return (
    <Tag
      data-reveal
      className={className}
      initial={{ opacity: 0, y }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true, amount: 0.25, margin: "0px 0px -6% 0px" }}
      // Reduced motion: a plain fade. The offset above is applied while the
      // node is still invisible, so snapping it back costs nothing.
      transition={{
        duration: reduced ? 0.4 : 0.9,
        delay: reduced ? 0 : delay,
        ease: EASE_OUT,
        y: reduced ? { duration: 0 } : undefined,
      }}
    >
      {children}
    </Tag>
  );
}

/**
 * A headline whose words rise out of a mask one after another. The whole
 * string is one text node group for assistive tech: the words are plain
 * inline spans and the visible order is the reading order.
 */
export function SplitWords({
  text,
  className,
  delay = 0,
  stagger = 0.06,
}: {
  text: string;
  className?: string;
  delay?: number;
  stagger?: number;
}) {
  const reduced = useReducedMotionSafe();
  const words = text.split(" ");
  return (
    <motion.span
      data-reveal
      className={cn("inline", className)}
      viewport={{ once: true, amount: 0.6 }}
    >
      {words.map((word, i) => (
        <React.Fragment key={`${word}-${i}`}>
          <span className="inline-block overflow-hidden pb-[0.12em] align-bottom">
            <motion.span
              data-reveal
              className="inline-block will-change-transform"
              initial={{ y: "112%", rotate: 3 }}
              whileInView={{ y: "0%", rotate: 0 }}
              viewport={{ once: true, amount: 0.6 }}
              // Reduced motion: the words are simply there (no rise, no stagger).
              transition={
                reduced
                  ? { duration: 0 }
                  : { duration: 1.05, delay: delay + i * stagger, ease: EASE_OUT }
              }
            >
              {word}
            </motion.span>
          </span>
          {i < words.length - 1 ? " " : null}
        </React.Fragment>
      ))}
    </motion.span>
  );
}
