"use client";

import * as React from "react";

/**
 * Lab 2's only script (lab routes only). Three small jobs:
 * - fit: the poster lines fill the width, at one size for all of them, and
 *   never so tall that the poster and its reflection leave the screen;
 * - the pointer's place in the water, as `--wx` / `--wy` on the hero,
 *   where the harder ripple shows;
 * - the whale ducks (`data-duck`) while the pointer is near it.
 * Nothing runs per frame unless the pointer moves; under reduced motion only
 * the fit runs.
 */
export function Lab2Interact({ children }: { children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const hero = ref.current?.querySelector<HTMLElement>("[data-lab2]");
    if (!hero) return;
    const poster = hero.querySelector<HTMLElement>("h1");
    const fit = () => {
      if (!poster) return;
      const lines = [...poster.querySelectorAll<HTMLElement>("[data-line]")].filter((l) => l.offsetParent !== null);
      if (!lines.length) return;
      hero.style.setProperty("--fs", "100px");
      const widest = Math.max(...lines.map((l) => l.scrollWidth));
      const pad = parseFloat(getComputedStyle(hero).paddingLeft) || 0;
      const byWidth = (100 * (hero.clientWidth - 2 * pad)) / widest;
      const byHeight = (window.innerHeight - 260) / (lines.length * 0.9 * 1.62);
      hero.style.setProperty("--fs", `${Math.min(byWidth, byHeight).toFixed(1)}px`);
    };
    fit();
    void document.fonts?.ready.then(fit);
    window.addEventListener("resize", fit);
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      return () => window.removeEventListener("resize", fit);
    }
    let raf = 0;
    let x = 0;
    let y = 0;
    const apply = () => {
      raf = 0;
      const water = hero.querySelector<HTMLElement>("[data-water]")?.getBoundingClientRect();
      if (water) {
        hero.style.setProperty("--wx", `${x - water.left}px`);
        hero.style.setProperty("--wy", `${y - water.top}px`);
      }
      hero.dataset.pointer = "";
      const visibleWhale = [...(poster?.querySelectorAll<HTMLElement>("[data-whale]") ?? [])].find((w) => w.offsetParent !== null);
      if (visibleWhale) {
        const b = visibleWhale.getBoundingClientRect();
        const d = Math.hypot(x - (b.left + b.width / 2), y - (b.top + b.height / 2));
        if (d < Math.max(140, b.width * 1.6)) hero.dataset.duck = "";
        else delete hero.dataset.duck;
      }
    };
    const onMove = (e: PointerEvent) => {
      x = e.clientX;
      y = e.clientY;
      if (!raf) raf = requestAnimationFrame(apply);
    };
    const onLeave = () => {
      delete hero.dataset.pointer;
      delete hero.dataset.duck;
    };
    hero.addEventListener("pointermove", onMove);
    hero.addEventListener("pointerdown", onMove);
    hero.addEventListener("pointerleave", onLeave);
    return () => {
      window.removeEventListener("resize", fit);
      hero.removeEventListener("pointermove", onMove);
      hero.removeEventListener("pointerdown", onMove);
      hero.removeEventListener("pointerleave", onLeave);
      cancelAnimationFrame(raf);
    };
  }, []);
  return <div ref={ref}>{children}</div>;
}
