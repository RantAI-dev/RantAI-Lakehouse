"use client";

import * as React from "react";
import { Moon, Sun } from "lucide-react";
import { getTod, subscribeTod, toggleTod } from "@/lib/tod";
import { cn } from "@/lib/utils";

/**
 * The day and night button, in the hero at the bottom right. It shows the
 * mode the page is in now (a sun and "Day", or a moon and "Night"); pressing
 * it changes the mode. The two icons trade places with a 200 ms turn and
 * scale. `aria-pressed` is true by day. The server renders night (the page
 * without JavaScript is night), and the real mode arrives on hydration.
 */
export function TodToggle({ copy }: { copy: { day: string; night: string; label: string } }) {
  const tod = React.useSyncExternalStore(subscribeTod, getTod, () => "night" as const);
  const day = tod === "day";
  const icon = "absolute inset-0 m-auto size-4 transition-[opacity,transform] duration-200 ease-out motion-reduce:transition-none";
  return (
    <button
      type="button"
      onClick={toggleTod}
      aria-pressed={day}
      aria-label={copy.label}
      className="lk-tod-btn inline-flex h-10 cursor-pointer items-center gap-2 rounded-full border border-[#E8F4FF]/25 bg-[#E8F4FF]/[0.08] pr-4 pl-3 text-[13px] font-medium text-[#E8F4FF] backdrop-blur-md transition-[background-color,border-color,color,transform] duration-200 ease-out hover:border-[#E8F4FF]/50 hover:bg-[#E8F4FF]/[0.16] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#44A6FA] active:scale-[0.96] motion-reduce:transition-none"
    >
      <span aria-hidden="true" className="relative block size-4">
        <Moon className={cn(icon, day ? "scale-50 -rotate-90 opacity-0" : "scale-100 rotate-0 opacity-100")} />
        <Sun className={cn(icon, day ? "scale-100 rotate-0 opacity-100" : "scale-50 rotate-90 opacity-0")} />
      </span>
      <span>{day ? copy.day : copy.night}</span>
    </button>
  );
}

/**
 * Tells the CSS whether the top bar sits over a day sky: the hero at the
 * top, or the sky of the final call. Over the underwater sections it is
 * off, so the bar stays dark there. One passive scroll listener writing
 * `data-over-sky` on <html> only when it flips; removed on leaving the page,
 * so the contact page's bar is never touched.
 */
export function SkyWatch({ page = "home" }: { page?: "home" | "contact" }) {
  React.useEffect(() => {
    const d = document.documentElement;
    let last = "";
    let raf = 0;
    const apply = () => {
      raf = 0;
      const y = 32;
      const vh = window.innerHeight;
      // The live lake keeps its sky under the bar until the camera is under (about 1.55 screens down);
      // the DOM hero's stage has scrolled away by about one.
      let over = page === "home" && window.scrollY < vh * (d.getAttribute("data-scene") === "full" ? 1.55 : 1);
      const fin = document.getElementById("final")?.getBoundingClientRect();
      const rise = document.getElementById("surface")?.getBoundingClientRect();
      if (!over && fin) {
        // By day the surfacing's bright last part runs into the final call's sky; at night only the sky counts.
        over =
          rise && rise.height > 0
            ? rise.top + rise.height * 0.72 <= y && fin.top + fin.height * 0.55 >= y
            : fin.top + fin.height * 0.28 <= y && fin.top + fin.height * 0.55 >= y;
      }
      // The light footer (by day), with a soft run-in above it.
      const foot = document.querySelector("footer")?.getBoundingClientRect();
      if (!over && foot) over = foot.top - 140 <= y;
      const next = over ? "1" : "0";
      if (next !== last) {
        last = next;
        d.setAttribute("data-over-sky", next);
      }
    };
    const onScroll = () => {
      if (!raf) raf = requestAnimationFrame(apply);
    };
    apply();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll, { passive: true });
    return () => {
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      if (raf) cancelAnimationFrame(raf);
      d.removeAttribute("data-over-sky");
    };
  }, [page]);
  return null;
}
