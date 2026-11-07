"use client";

import { ArrowUp } from "lucide-react";

/**
 * Back to top (WEB-5 O7, rantai-page's `BackToTop`). With Lenis running
 * (desktop, D16) it asks the smooth-scroll engine to glide there, through a
 * window event, so Lenis stays out of the first JavaScript; otherwise the
 * browser scrolls, at once under reduced motion. Focus then moves to the
 * top of `<main>` (not the skip link, which would show itself after a
 * mouse click too), so a keyboard visitor continues from the top.
 */
export const SCROLL_TOP_EVENT = "lk:scroll-top";

export function BackToTop({ label }: Readonly<{ label: string }>) {
  return (
    <button
      type="button"
      onClick={() => {
        const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
        if (document.documentElement.classList.contains("lenis")) window.dispatchEvent(new Event(SCROLL_TOP_EVENT));
        else window.scrollTo({ top: 0, behavior: reduce ? "auto" : "smooth" });
        document.getElementById("main")?.focus({ preventScroll: true });
      }}
      className="group inline-flex min-h-10 items-center gap-2 rounded-full text-[13px] whitespace-nowrap text-[#E8F4FF]/70 transition-colors duration-200 ease-out hover:text-[#E8F4FF]"
    >
      {label}
      <span className="grid size-7 place-items-center rounded-full border border-[#E8F4FF]/20 transition-[background-color,border-color,color] duration-200 ease-out group-hover:border-[#E8F4FF] group-hover:bg-[#E8F4FF] group-hover:text-[#020514] group-active:scale-90">
        <ArrowUp
          aria-hidden="true"
          className="size-3.5 transition-transform duration-200 ease-out group-hover:-translate-y-0.5 motion-reduce:transition-none"
        />
      </span>
    </button>
  );
}
