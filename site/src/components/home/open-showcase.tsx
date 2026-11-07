"use client";

import * as React from "react";

/**
 * WEB-5 O38: the only client code of the "Yours to keep" showcase. It marks
 * which promise is current: the one whose scroll marker (one screen tall,
 * desktop only) crosses the middle of the screen, its list item and its
 * picture, through `data-on` on both
 * and `data-active` on the list (CSS does the rest, open-section.tsx). One
 * IntersectionObserver, no React state, nothing per scroll event. A click on
 * an item is a plain link to its marker, so it scrolls there with or
 * without JavaScript; this only lights it at once.
 */
export function OpenShowcase({ children }: { children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const root = ref.current?.querySelector<HTMLElement>(".lk-show");
    if (!root) return;
    const marks = [...root.querySelectorAll<HTMLElement>(".lk-show-mark")];
    const items = [...root.querySelectorAll<HTMLElement>(".lk-show-item")];
    const visuals = [...root.querySelectorAll<HTMLElement>(".lk-show-visual")];
    const light = (i: number) => {
      root.dataset.active = String(i);
      items.forEach((el, k) => toggle(el, k === i));
      visuals.forEach((el, k) => toggle(el, k === i));
    };
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (e.isIntersecting) light(marks.indexOf(e.target as HTMLElement));
      },
      // A line across the middle of the screen.
      { rootMargin: "-50% 0px -50% 0px" },
    );
    for (const el of marks) io.observe(el);
    const onClick = (ev: MouseEvent) => {
      const link = (ev.target as HTMLElement).closest<HTMLAnchorElement>("a[data-show]");
      if (link) light(Number(link.dataset.show));
    };
    root.addEventListener("click", onClick);
    return () => {
      io.disconnect();
      root.removeEventListener("click", onClick);
    };
  }, []);
  return <div ref={ref}>{children}</div>;
}

function toggle(el: HTMLElement, on: boolean) {
  if (on) el.dataset.on = "";
  else delete el.dataset.on;
}
