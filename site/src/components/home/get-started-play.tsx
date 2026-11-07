"use client";

import * as React from "react";

/**
 * WEB-5 O23: brings the Quickstart's terminals to life, the only client
 * code of the section. The server renders every terminal finished:
 * commands, output and the "ready" state, so the section reads complete
 * without JavaScript and under reduced motion.
 *
 * After hydration each terminal still off screen is set to `idle`, which
 * hides only what its commands print (CSS on `data-term`); when it comes
 * into view it is `done` and its output appears, line by line. A terminal
 * already on screen stays finished.
 *
 * O45 rollback (owner: "kembalikan kayak yang dulu"), and audit #5: the
 * commands no longer type themselves. Typed one terminal at a time, steps 2
 * and 3 sat blank for seconds while step 1 typed; now every command shows
 * at once and only the output is animated, so the deferred typing chunk is
 * gone too.
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
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          const t = e.target as HTMLElement;
          io.unobserve(t);
          t.dataset.term = "done";
        }
      },
      { threshold: 0.6 },
    );
    for (const t of terms) io.observe(t);
    return () => {
      io.disconnect();
      for (const t of terms) t.dataset.term = "done";
    };
  }, []);
  return <div ref={ref}>{children}</div>;
}
