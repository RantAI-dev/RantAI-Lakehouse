"use client";

import * as React from "react";
import { m } from "framer-motion";
import { cn } from "@/lib/utils";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * Scroll-into-view reveals shared by the sections.
 *
 * With `prefers-reduced-motion` they are fades only: no travel, no blur,
 * no stagger (plan §2.8). Every revealed node carries `data-reveal`, which
 * `app/globals.css` shows in full until `motion-ready` is on <html> (WEB-5
 * D5, review fix R1), so the page reads without JavaScript and when the
 * animation features fail to load.
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
  const Tag = m[as] as typeof m.div;
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
 * A headline whose words rise out of a mask one after another.
 *
 * The in-view trigger sits on the outer span, never on the words: a word that
 * is translated below its clipping mask has no visible area, so an
 * IntersectionObserver on it would never fire. The words follow the parent
 * through variants. For assistive tech the text is just the plain words in
 * reading order.
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
    <m.span
      data-reveal
      className={cn("inline", className)}
      initial="hidden"
      whileInView="shown"
      viewport={{ once: true, amount: 0.3 }}
    >
      {words.map((word, i) => (
        <React.Fragment key={`${word}-${i}`}>
          <span className="inline-block overflow-hidden pb-[0.14em] align-bottom">
            <m.span
              data-reveal
              className="inline-block will-change-transform"
              custom={i}
              variants={{
                hidden: { y: "112%", rotate: 3 },
                // Reduced motion: the words are simply there (no rise, no stagger).
                shown: (k: number) => ({
                  y: "0%",
                  rotate: 0,
                  transition: reduced
                    ? { duration: 0 }
                    : { duration: 1.05, delay: delay + k * stagger, ease: EASE_OUT },
                }),
              }}
            >
              {word}
            </m.span>
          </span>
          {i < words.length - 1 ? " " : null}
        </React.Fragment>
      ))}
    </m.span>
  );
}
