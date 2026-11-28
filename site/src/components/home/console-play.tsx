"use client";

import * as React from "react";

/**
 * The only client code the console-home picture puts in the initial bundle: it watches whether
 * the picture is really on show (in the viewport, and, in the sticky showcase, the promise lit)
 * and, when it is, loads `console-script.ts` as its own chunk and runs it. The script is told to
 * hold while the tab is hidden, the picture is off screen or the visitor hovers or focuses it.
 * Under reduced motion nothing loads and the settled console stays.
 */
export function ConsolePlay({ children }: { children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const root = ref.current?.querySelector<HTMLElement>(".lk-con");
    if (!root || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    let inView = false;
    let hold = false;
    let gone = false;
    let stop: (() => void) | undefined;
    let control: { setActive: (on: boolean) => void } | undefined;
    const slot = root.closest<HTMLElement>(".lk-show-visual");
    const active = () => inView && !hold && !document.hidden && (!slot || slot.hasAttribute("data-on"));
    let loading = false;
    const sync = () => {
      if (gone) return;
      const on = active();
      if (on && !control && !loading) {
        loading = true;
        void import("./console-script").then(({ start }) => {
          if (gone) return;
          const c = start(root);
          control = c;
          stop = c.stop;
          c.setActive(active());
        });
      } else control?.setActive(on);
    };
    const io = new IntersectionObserver(([e]) => {
      inView = !!e?.isIntersecting;
      sync();
    }, { threshold: 0.35 });
    io.observe(root);
    const poll = window.setInterval(sync, 400);
    const enter = () => {
      hold = true;
      sync();
    };
    const leave = () => {
      hold = false;
      sync();
    };
    root.addEventListener("pointerenter", enter);
    root.addEventListener("pointerleave", leave);
    root.addEventListener("focusin", enter);
    root.addEventListener("focusout", leave);
    return () => {
      gone = true;
      io.disconnect();
      window.clearInterval(poll);
      root.removeEventListener("pointerenter", enter);
      root.removeEventListener("pointerleave", leave);
      root.removeEventListener("focusin", enter);
      root.removeEventListener("focusout", leave);
      stop?.();
    };
  }, []);
  return (
    <div ref={ref} className="h-full">
      {children}
    </div>
  );
}
