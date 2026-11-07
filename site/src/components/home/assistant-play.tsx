"use client";

import * as React from "react";

/**
 * WEB-5 O19: the only client code the assistant panel puts in the initial
 * bundle. The server renders the finished conversation; this rewinds it to
 * an empty chat once the page has hydrated, and only when the visitor will
 * see it play: not under reduced motion, and not when the panel is already
 * on screen (a reload half way down keeps the finished one rather than
 * wiping it in view). When the panel comes into view it loads the timed
 * script as its own chunk and plays it once.
 */
export function AssistantPlay({ children }: { children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const panel = ref.current?.firstElementChild;
    if (!(panel instanceof HTMLElement)) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const box = panel.getBoundingClientRect();
    if (box.top < window.innerHeight && box.bottom > 0) return;
    panel.dataset.s = "";
    panel.classList.remove("lk-ast-still");
    let stop: (() => void) | undefined;
    let gone = false;
    const io = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return;
        io.disconnect();
        void import("./assistant-script").then(({ play }) => {
          if (!gone) stop = play(panel);
        });
      },
      { threshold: 0.45 },
    );
    io.observe(panel);
    return () => {
      gone = true;
      io.disconnect();
      stop?.();
    };
  }, []);
  return <div ref={ref}>{children}</div>;
}
