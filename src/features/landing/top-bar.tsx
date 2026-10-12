"use client";

import * as React from "react";
import { useMotionValueEvent, useScroll } from "motion/react";
import { cn } from "@/lib/utils";
import { CTA, LINKS, NAV_LINKS } from "./landing-copy";
import { CtaLink } from "./landing-ui";
import { MascotMark } from "./mascot-svg";

/**
 * S0 — fixed top bar. Transparent over the hero; a backdrop blur and a 1 px
 * foam hairline arrive once the page has scrolled 40 px (plan §4 S0). The
 * links are plain anchors to the sections; on phones only the wordmark and the
 * console button stay, because four links would not fit beside them.
 */
export function TopBar() {
  const { scrollY } = useScroll();
  const [scrolled, setScrolled] = React.useState(false);
  useMotionValueEvent(scrollY, "change", (y) => {
    const next = y > 40;
    setScrolled((prev) => (prev === next ? prev : next));
  });

  return (
    <header
      className={cn(
        "fixed inset-x-0 top-0 z-50 border-b transition-[background-color,backdrop-filter,border-color] duration-500",
        scrolled
          ? "border-[#E8F4FF]/10 bg-[#020514]/55 backdrop-blur-xl"
          : "border-transparent bg-transparent backdrop-blur-0",
      )}
    >
      <div className="mx-auto flex h-16 w-full max-w-[1280px] items-center justify-between gap-4 px-4 md:px-6">
        <a href="#hero" className="flex items-center gap-2.5 text-[15px] font-medium tracking-tight text-[#E8F4FF]">
          <MascotMark className="size-6" />
          <span>RantAI Lakehouse</span>
        </a>
        <nav aria-label="Page sections" className="hidden items-center gap-1 md:flex">
          {NAV_LINKS.map((l) => (
            <a
              key={l.href}
              href={l.href}
              className="rounded-full px-3.5 py-1.5 text-[14px] text-[#E8F4FF]/75 transition-colors hover:text-[#E8F4FF]"
            >
              {l.label}
            </a>
          ))}
        </nav>
        <CtaLink href={LINKS.console} size="sm">
          {CTA.console}
        </CtaLink>
      </div>
    </header>
  );
}
