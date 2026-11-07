"use client";

import * as React from "react";

/**
 * WEB-5 O23: brings the Quickstart's terminals to life, and is the only
 * client code the section adds to the initial bundle (the typing is
 * `get-started-type.ts`, loaded on first sight). The server renders every
 * terminal finished: commands, output and the "ready" state, so the
 * section reads complete without JavaScript and under reduced motion.
 *
 * After hydration each terminal that is still off screen is set to `idle`
 * (its command blank, its output hidden: CSS on `data-term`); when one
 * comes into view its commands type themselves and its output appears.
 * A terminal already on screen stays finished.
 */
export function GetStartedPlay({ children }: { children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const root = ref.current;
    if (!root || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const terms = [...root.querySelectorAll<HTMLElement>("[data-term]")].filter((t) => {
      const r = t.getBoundingClientRect();
      return r.top >= window.innerHeight || r.bottom <= 0;
    });
    if (terms.length === 0) return;
    for (const t of terms) t.dataset.term = "idle";
    const stops: (() => void)[] = [];
    let gone = false;
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          const t = e.target as HTMLElement;
          io.unobserve(t);
          void import("./get-started-type").then(({ typeTerminal }) => {
            if (!gone) stops.push(typeTerminal(t));
          });
        }
      },
      { threshold: 0.6 },
    );
    for (const t of terms) io.observe(t);
    return () => {
      gone = true;
      io.disconnect();
      for (const stop of stops) stop();
    };
  }, []);
  return <div ref={ref}>{children}</div>;
}
