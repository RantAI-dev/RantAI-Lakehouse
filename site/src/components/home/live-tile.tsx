"use client";

import * as React from "react";

/**
 * One dashboard tile on the rail (WEB-5 owner QA O10). It does no
 * animation work itself: an IntersectionObserver writes two attributes and
 * CSS does the rest (app/globals.css, "the living tiles"):
 * - `data-shown`, once, when the tile first comes into view: the entrance
 *   (the line draws, the dots light up, the bands grow, the number rolls up,
 *   the cells fill);
 * - `data-live`, while the tile is on screen: the calm idle loop runs, and
 *   it pauses when the tile leaves.
 * No React state, no work per frame. Without JavaScript neither attribute
 * is set and the tiles are their final, static pictures; under reduced
 * motion the CSS keeps them static too.
 *
 * The tile is focusable, so keyboard visitors get the same lift, glow and
 * value label as a hover.
 */
export function LiveTile({ className, label, children }: Readonly<{ className?: string; label: string; children: React.ReactNode }>) {
  const ref = React.useRef<HTMLLIElement>(null);
  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver(
      ([e]) => {
        if (e.isIntersecting) el.setAttribute("data-shown", "");
        el.toggleAttribute("data-live", e.isIntersecting);
      },
      { threshold: 0.45 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return (
    <li ref={ref} tabIndex={0} aria-label={label} className={`lk-tile group/tile ${className ?? ""}`}>
      {children}
    </li>
  );
}
