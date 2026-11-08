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
/**
 * The conversation is stacked from the bottom, so once it has settled the oldest user
 * bubble can sit half out of the top of the log, under the fade. A half bubble reads as a
 * rendering fault, so a bubble that is not whole is let go entirely (`data-clipped`);
 * assistant replies are tall and keep scrolling out as they do in a real chat.
 */
function hideClippedBubbles(panel: HTMLElement) {
  const log = panel.querySelector(".lk-ast-log");
  if (!log) return;
  const top = log.getBoundingClientRect().top;
  for (const b of log.querySelectorAll<HTMLElement>(".lk-ast-user")) {
    b.removeAttribute("data-clipped");
    if (b.getBoundingClientRect().top < top + 8) b.setAttribute("data-clipped", "");
  }
}

export function AssistantPlay({ children }: { children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const panel = ref.current?.firstElementChild;
    if (!(panel instanceof HTMLElement)) return;
    // Re-measure whenever the panel has reached its last step or is resized.
    let settle = 0;
    const settled = () => {
      window.clearTimeout(settle);
      if (!panel.dataset.s?.split(" ").includes("12")) {
        for (const b of panel.querySelectorAll("[data-clipped]")) b.removeAttribute("data-clipped");
      }
      settle = window.setTimeout(() => {
        if (panel.dataset.s?.split(" ").includes("12")) hideClippedBubbles(panel);
      }, 750);
    };
    const mo = new MutationObserver(settled);
    mo.observe(panel, { attributes: true, attributeFilter: ["data-s"] });
    window.addEventListener("resize", settled);
    settled();
    const stopSettle = () => {
      mo.disconnect();
      window.clearTimeout(settle);
      window.removeEventListener("resize", settled);
    };
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return stopSettle;
    const box = panel.getBoundingClientRect();
    if (box.top < window.innerHeight && box.bottom > 0) return stopSettle;
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
      stopSettle();
      if (frame) cancelAnimationFrame(frame);
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      player?.stop();
    };
  }, []);
  return <div ref={ref}>{children}</div>;
}
