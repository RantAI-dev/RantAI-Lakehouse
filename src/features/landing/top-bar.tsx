"use client";

import * as React from "react";
import { CTA, LINKS, NAV_LINKS } from "./landing-copy";
import { CtaLink } from "./landing-ui";
import { MascotMark } from "./mascot-svg";

/**
 * S0 — fixed top bar. Transparent over the hero; the blur and the 1 px foam
 * hairline arrive once the page has scrolled 40 px (plan §4 S0).
 */
export function TopBar() {
  return (
    <header className="fixed inset-x-0 top-0 z-50">
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
