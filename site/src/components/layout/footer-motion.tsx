"use client";

import { useEffect, useRef, useSyncExternalStore } from "react";
import { m } from "framer-motion";
import { ArrowUp } from "lucide-react";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import { cn } from "@/lib/utils";
import { DESKTOP_MOTION } from "./smooth-scroll";

/**
 * WEB-5 owner QA O15: a 1:1 copy of rantai-page's
 * `components/layout/footer-motion.tsx` (`Reveal`, `FooterWordmark`,
 * `BackToTop`), so the family's footers move the same way. Only these
 * change:
 * - the colours, to the lake by role (rantai's `foreground` is the foam);
 * - the overscroll pull moves nothing in layout (O28, see below);
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

function subscribeDesktop(onChange: () => void): () => void {
  const mq = window.matchMedia(DESKTOP_MOTION);
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}

function useDesktopMotion(): boolean {
  return useSyncExternalStore(subscribeDesktop, () => window.matchMedia(DESKTOP_MOTION).matches, () => false);
}

export function FooterWordmark({ text, className }: Readonly<{ text: string; className?: string }>) {
  const ref = useRef<HTMLDivElement>(null);
  const reduce = useReducedMotionSafe();
  const desktopMotion = useDesktopMotion();
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

  // Overscroll reveal: the wordmark is cropped by the panel's bottom edge.
  // If the visitor keeps scrolling down once the page has ended, the panel
  // stretches (with growing resistance) to show the whole word, and the
  // footer lifts by the same amount so the new space stays in view. When
  // they stop, it settles back.
  //
  // WEB-5 owner QA O28: rantai-page's pull, with its timing and easing, but
  // nothing in it changes layout. rantai grows the wordmark's margin, so the
  // page gets longer; here that re-coloured the page-wide water to the abyss
  // (O26). Instead the footer lifts with a transform, as on rantai, and the
  // panel's paint clip widens below it (`overflow: clip` with
  // `overflow-clip-margin`), so more of the word shows while the document's
  // height, Lenis and the gauge stay exactly as they were. Where the margin
  // is not supported (older Safari), the word rises inside the clip instead.
  useEffect(() => {
    const root = ref.current;
    const panel = root?.parentElement;
    const footer = root?.closest("footer");
    // Never intercept touch scrolling, even while the footer is offscreen.
    // Safari must be free to handle momentum and its own rubber band.
    if (!root || !panel || !footer || !desktopMotion) return;
    const clipMargin = CSS.supports("overflow-clip-margin", "1px");

    let pull = 0; // raw input since the pull started
    let shown = 0; // pixels currently revealed
    let room = 0; // pixels needed to show the whole word
    let frame = 0;
    let release: ReturnType<typeof setTimeout> | undefined;

    const atBottom = () => window.scrollY + window.innerHeight >= document.documentElement.scrollHeight - 4;

    const measure = () => {
      const word = root.firstElementChild?.getBoundingClientRect();
      const edge = panel.getBoundingClientRect().bottom;
      room = word ? Math.max(0, word.bottom - edge) + 40 : 0;
    };

    const apply = () => {
      if (clipMargin) panel.style.setProperty("overflow-clip-margin", shown ? `${shown}px` : "");
      else root.style.transform = shown ? `translateY(${-shown}px)` : "";
      // How far through the reveal we are, 0 to 1; brings in the "reward" layer.
      // The rubber band only creeps toward its end, so reach the full tint
      // by about three quarters of the way instead.
      const progress = room ? shown / room : 0;
      const solid = Math.min(1, Math.max(0, (progress - 0.1) / 0.65));
      root.style.setProperty("--pull", String(solid));
      footer.style.transform = shown ? `translateY(${-shown}px)` : "";
    };

    const tick = () => {
      // Rubber band: easy at first, then harder the further it goes.
      const goal = room * (1 - Math.exp(-pull / (room * 1.4)));
      shown += (goal - shown) * 0.18;
      if (Math.abs(goal - shown) < 0.4) shown = goal;
      apply();
      frame = shown !== goal ? requestAnimationFrame(tick) : 0;
    };
    const kick = () => {
      if (!frame) frame = requestAnimationFrame(tick);
    };

    /** Returns true when this input was used for the pull. */
    const take = (delta: number) => {
      if (pull === 0) {
        if (delta <= 0 || !atBottom()) return false;
        measure();
        if (!room) return false;
      }
      pull = Math.max(0, pull + delta);
      clearTimeout(release);
      release = setTimeout(() => {
        pull = 0;
        kick();
      }, 180);
      kick();
      return true;
    };

    // Capture phase, so the smooth scroller never sees a wheel we used.
    const onWheel = (e: WheelEvent) => {
      if (take(e.deltaY)) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    window.addEventListener("wheel", onWheel, { capture: true, passive: false });
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(release);
      window.removeEventListener("wheel", onWheel, { capture: true });
      panel.style.removeProperty("overflow-clip-margin");
      root.style.transform = "";
      root.style.removeProperty("--pull");
      footer.style.transform = "";
    };
  }, [desktopMotion]);

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
              {/* Flat copy that takes over as the visitor pulls past the end. O28: a
                  faint foam, the wordmark's own faded tint, never a grey word. */}
              <span className="absolute inset-0 inline-block text-[#E8F4FF]/[0.12]" style={{ opacity: "var(--pull, 0)" }}>
                {char}
              </span>
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
