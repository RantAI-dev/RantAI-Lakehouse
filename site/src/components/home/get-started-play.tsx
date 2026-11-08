"use client";

import * as React from "react";

/**
 * WEB-5 O23: the only client code of "Get started". The server renders every
 * step lit; after hydration (and unless motion is reduced) the steps still
 * below the middle of the screen are set unlit (`data-lit` absent under
 * `data-js`), and each lights, with a short eased transition on its number
 * (`.lk-gs-num` in app/globals.css), when its block reaches the middle of the
 * screen. The line between the numbers fills with the scroll in CSS.
 */
export function GetStartedPlay({ children }: { children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const root = ref.current;
    if (!root || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const steps = [...root.querySelectorAll<HTMLElement>(".lk-gs-steps > li")];
    const mid = window.innerHeight * 0.55;
    const pending = steps.filter((t) => t.getBoundingClientRect().top > mid);
    if (pending.length === 0) return;
    root.dataset.js = "";
    for (const t of steps) if (!pending.includes(t)) t.dataset.lit = "";
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting && e.boundingClientRect.top > 0) continue;
          const t = e.target as HTMLElement;
          io.unobserve(t);
          t.dataset.lit = "";
        }
      },
      { rootMargin: "0px 0px -45% 0px" },
    );
    for (const t of pending) io.observe(t);
    return () => {
      io.disconnect();
      delete root.dataset.js;
    };
  }, []);
  return <div ref={ref}>{children}</div>;
}
