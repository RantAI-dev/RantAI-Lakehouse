"use client";

import { useEffect, useRef } from "react";
import { m } from "framer-motion";
import { ArrowUp } from "lucide-react";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import { cn } from "@/lib/utils";

/**
 * WEB-5 owner QA O15: a 1:1 copy of rantai-page's
 * `components/layout/footer-motion.tsx` (`Reveal`, `FooterWordmark`,
 * `BackToTop`), so the family's footers move the same way. Only these
 * change:
 * - the colours, to the lake by role (rantai's `foreground` is the foam);
 * - no overscroll pull on the wordmark (O26, see below);
 * - `m.*` instead of `motion.*`, because this site loads motion's features
 *   lazily (`motion-features.tsx`), and `data-reveal`, so every piece is
 *   visible without JavaScript (D5);
 * - Back to top reaches Lenis through a window event (`SCROLL_TOP_EVENT`),
 *   so Lenis stays out of the first JavaScript (D16);
 * - the wordmark's letters rise as soon as any of it is in view, not at
 *   three tenths: the page's end crops it, and the reveal must never wait
 *   on more of it than the page can show (O26).
 */

const ease = [0.22, 1, 0.36, 1] as const;

/* ------------------------------------------------------------------ */
/*  Fade-up reveal with an index-based delay                          */
/* ------------------------------------------------------------------ */

export function Reveal({
  children,
  index = 0,
  className,
}: Readonly<{
  children: React.ReactNode;
  index?: number;
  className?: string;
}>) {
  const reduce = useReducedMotionSafe();

  return (
    <m.div
      data-reveal
      className={cn(className, "max-md:filter-none!")}
      initial={reduce ? false : { opacity: 0, y: 24, filter: "blur(6px)" }}
      whileInView={{ opacity: 1, y: 0, filter: "blur(0px)" }}
      viewport={{ once: true, margin: "-60px" }}
      transition={{ duration: 0.8, ease, delay: index * 0.08 }}
    >
      {children}
    </m.div>
  );
}

/* ------------------------------------------------------------------ */
/*  Wordmark: letters rise in; near the cursor they lift and brighten, */
/*  so moving across the word sends a soft wave through it             */
/* ------------------------------------------------------------------ */

const letterClass = "inline-block bg-linear-to-b bg-clip-text text-transparent";

export function FooterWordmark({ text, className }: Readonly<{ text: string; className?: string }>) {
  const ref = useRef<HTMLDivElement>(null);
  const reduce = useReducedMotionSafe();
  const letters = Array.from(text);

  useEffect(() => {
    const root = ref.current;
    if (!root || reduce) return;
    const waves = [...root.querySelectorAll<HTMLElement>("[data-wave]")];
    const inks = [...root.querySelectorAll<HTMLElement>("[data-wave-ink]")];
    const current = waves.map(() => 0);
    let pointerX: number | null = null;
    let frame = 0;

    const tick = () => {
      let moving = false;
      const reach = root.clientWidth * 0.18;
      waves.forEach((el, i) => {
        let goal = 0;
        if (pointerX !== null) {
          const r = el.getBoundingClientRect();
          const k = Math.max(0, 1 - Math.abs(pointerX - (r.left + r.width / 2)) / reach);
          goal = k * k * (3 - 2 * k);
        }
        // Ease toward the goal so the wave follows the cursor smoothly.
        current[i] += (goal - current[i]) * 0.12;
        if (Math.abs(goal - current[i]) > 0.002) moving = true;
        el.style.transform = `translateY(${-8 * current[i]}%)`;
        inks[i].style.opacity = String(current[i]);
      });
      frame = moving ? requestAnimationFrame(tick) : 0;
    };
    const kick = () => {
      if (!frame) frame = requestAnimationFrame(tick);
    };
    const move = (e: PointerEvent) => {
      if (e.pointerType !== "mouse") return;
      pointerX = e.clientX;
      kick();
    };
    const leave = () => {
      pointerX = null;
      kick();
    };
    root.addEventListener("pointermove", move);
    root.addEventListener("pointerleave", leave);
    return () => {
      cancelAnimationFrame(frame);
      root.removeEventListener("pointermove", move);
      root.removeEventListener("pointerleave", leave);
    };
  }, [reduce]);

  // WEB-5 owner QA O26: rantai-page's overscroll pull (scrolling on past the
  // end stretches the panel to show the whole word in solid colour, and lifts
  // the footer) is left out here. On this page it moved the end of the page,
  // so the page-wide water re-coloured to the abyss and the bottom turned
  // black, and a solid grey word appeared under the bar. The browser's own
  // rubber band shows the footer's colour instead (app/globals.css).

  return (
    <div ref={ref} aria-hidden="true" className={cn("relative select-none", className)}>
      {/* WEB-5 owner QA O26: this site's base styles give every <p> 16 px / 24 px
          (app/globals.css, the console's type rules), which rantai-page does not
          have, so the wordmark rendered 24 px tall below the page's end. It
          inherits the wrapper's clamp and leading instead. */}
      <m.p
        data-reveal
        className="flex justify-center [font-size:inherit] [line-height:inherit]"
        initial={reduce ? false : "hidden"}
        whileInView="shown"
        viewport={{ once: true, amount: "some" }}
        transition={{ staggerChildren: 0.06 }}
      >
        {letters.map((char, i) => (
          <m.span
            key={`${char}-${i}`}
            data-reveal
            className="inline-block"
            variants={{
              hidden: { y: "45%", opacity: 0 },
              shown: { y: "0%", opacity: 1, transition: { duration: 1.1, ease } },
            }}
          >
            <span data-wave className="relative inline-block">
              <span className={cn(letterClass, "from-[#E8F4FF]/[0.08] to-transparent")}>{char}</span>
              {/* Brighter copy, faded in as the cursor comes near */}
              <span
                data-wave-ink
                className={cn(letterClass, "absolute inset-0 from-[#E8F4FF]/[0.42] to-[#E8F4FF]/[0.06]")}
                style={{ opacity: 0 }}
              >
                {char}
              </span>
            </span>
          </m.span>
        ))}
      </m.p>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Back to top                                                       */
/* ------------------------------------------------------------------ */

/** Lenis listens for this (`smooth-scroll-engine.tsx`); without it the browser scrolls. */
export const SCROLL_TOP_EVENT = "lk:scroll-top";

function scrollToTop() {
  if (document.documentElement.classList.contains("lenis")) window.dispatchEvent(new Event(SCROLL_TOP_EVENT));
  else {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    window.scrollTo({ top: 0, behavior: reduce ? "auto" : "smooth" });
  }
  // The site's addition: a keyboard visitor continues from the top too.
  document.getElementById("main")?.focus({ preventScroll: true });
}

export function BackToTop({ label }: Readonly<{ label: string }>) {
  return (
    <button
      type="button"
      onClick={scrollToTop}
      className="group inline-flex items-center gap-2 rounded-full whitespace-nowrap transition-colors hover:text-[#E8F4FF] focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-[#44A6FA] max-sm:min-h-10"
    >
      {label}
      <span className="grid size-7 place-items-center rounded-full border border-[#E8F4FF]/15 transition-colors group-hover:border-[#E8F4FF] group-hover:bg-[#E8F4FF] group-hover:text-[#020514]">
        <ArrowUp
          aria-hidden="true"
          className="size-3.5 transition-transform duration-300 group-hover:-translate-y-0.5 motion-reduce:transition-none"
        />
      </span>
    </button>
  );
}
