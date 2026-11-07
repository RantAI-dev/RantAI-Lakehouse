"use client";

import * as React from "react";

/**
 * Lab 3's only client code: it plays one answer on the stage wrapper (its
 * parent) by writing `data-pick` and `data-stage`, types the question into
 * `.l3-q-text`, and replays on a suggested question. Reduced motion: the
 * picked answer is shown complete at once. No React state; the markup is
 * the server's.
 */
export function LakeController({ typeMs, lengths }: Readonly<{ typeMs: number; lengths: readonly number[] }>) {
  const ref = React.useRef<HTMLSpanElement>(null);
  React.useEffect(() => {
    const stage = ref.current?.parentElement;
    const text = stage?.querySelector<HTMLElement>(".l3-q-text");
    const chips = [...(stage?.querySelectorAll<HTMLButtonElement>("[data-ask]") ?? [])];
    if (!stage || !text) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const questions = chips.map((c) => c.textContent ?? "");
    let timers: number[] = [];
    const set = (s: number) => stage.setAttribute("data-stage", String(s));

    const play = (i: number) => {
      timers.forEach((t) => window.clearTimeout(t));
      timers = [];
      stage.setAttribute("data-pick", String(i));
      chips.forEach((c, j) => c.setAttribute("aria-pressed", String(i === j)));
      const q = questions[i];
      if (reduce) {
        text.textContent = q;
        set(5);
        return;
      }
      text.textContent = "";
      set(1);
      const at = (ms: number, f: () => void) => timers.push(window.setTimeout(f, ms));
      let t = 450;
      for (let k = 1; k <= q.length; k++) at((t += typeMs), () => (text.textContent = q.slice(0, k)));
      at((t += 350), () => set(2));
      at((t += 900), () => set(3));
      at((t += 450), () => set(4));
      at((t += 650 + lengths[i] * 70), () => set(5));
    };

    const onClick = (e: Event) => {
      const chip = (e.target as Element).closest<HTMLButtonElement>("[data-ask]");
      if (chip) play(Number(chip.dataset.ask));
    };
    stage.addEventListener("click", onClick);
    play(0);
    return () => {
      timers.forEach((t) => window.clearTimeout(t));
      stage.removeEventListener("click", onClick);
    };
  }, [typeMs, lengths]);
  return <span ref={ref} hidden />;
}
