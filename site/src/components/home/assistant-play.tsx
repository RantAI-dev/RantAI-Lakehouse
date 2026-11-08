"use client";

import * as React from "react";

/**
 * WEB-5 O19: the only client code the assistant panel puts in the initial
 * bundle. The server renders the finished conversation; this rewinds it to
 * an empty chat once the page has hydrated, and only when the visitor will
 * see it play: not under reduced motion, and not when the panel is already
 * on screen (a reload half way down keeps the finished one rather than
 * wiping it in view). When the panel is really on show (see below) it loads the
 * timed script as its own chunk and plays it once, pausing while it is not.
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
    // Owner QA: the play starts only when the panel is really on show: the depths stage above it
    // has (nearly) left (its section's bottom is above 35 % of the screen: the assistant rides up
    // under it by 30 vh, so the panel is in view while the stage's card still is) and the panel's top is at least 35 % up the screen. If the visitor scrolls back, it
    // pauses where it was and carries on when they return.
    const depths = document.getElementById("depths");
    let player: import("./assistant-script").Player | undefined;
    let started = false;
    let want = false;
    let frame = 0;
    let gone = false;
    const shown = () => {
      const p = panel.getBoundingClientRect();
      const d = depths?.getBoundingClientRect();
      return (!d || d.bottom <= window.innerHeight * 0.35) && p.top <= window.innerHeight * 0.65 && p.bottom > 0;
    };
    const check = () => {
      frame = 0;
      want = shown();
      if (!started && want) {
        started = true;
        void import("./assistant-script").then(({ play }) => {
          if (gone) return;
          player = play(panel);
          if (!want) player.pause();
        });
      } else if (player) {
        if (want) player.resume();
        else player.pause();
      }
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(check);
    };
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    onScroll();
    return () => {
      gone = true;
      if (frame) cancelAnimationFrame(frame);
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      player?.stop();
    };
  }, []);
  return <div ref={ref}>{children}</div>;
}
