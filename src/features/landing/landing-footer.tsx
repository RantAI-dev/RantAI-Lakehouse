"use client";

import * as React from "react";
import { FOOTER } from "./landing-copy";
import { Container } from "./landing-ui";
import { MascotMark } from "./mascot-svg";

type Column = { title: string; links: readonly { label: string; href: string }[] };

const COLUMNS: Column[] = [
  { title: "Product", links: FOOTER.product },
  { title: "RantAI", links: FOOTER.rantai },
  { title: "Connect", links: FOOTER.connect },
];

/** S9 — footer: the family links and the legal line. */
export function LandingFooter() {
  return (
    <footer className="relative border-t border-[#E8F4FF]/10 bg-[#020514] py-16">
      <Container className="grid gap-12 md:grid-cols-[1.4fr_repeat(3,1fr)]">
        <div>
          <MascotMark className="size-8" />
          <p className="mt-4 max-w-[18rem] text-[15px] leading-6 text-[#E8F4FF]/75">{FOOTER.tagline}</p>
        </div>
        {COLUMNS.map((col) => (
          <nav key={col.title} aria-label={col.title}>
            <h2 className="lk-mono text-[11px] leading-4 font-medium tracking-[0.2em] text-[#E8F4FF]/55 uppercase">
              {col.title}
            </h2>
            <ul className="mt-4 space-y-2.5">
              {col.links.map((l) => (
                <li key={l.href + l.label}>
                  <a
                    href={l.href}
                    className="text-[15px] leading-6 text-[#E8F4FF]/85 transition-colors hover:text-[#44A6FA]"
                    {...(l.href.startsWith("http") ? { target: "_blank", rel: "noreferrer noopener" } : {})}
                  >
                    {l.label}
                  </a>
                </li>
              ))}
            </ul>
          </nav>
        ))}
      </Container>
      <Container className="mt-14">
        <p className="lk-mono text-[12px] text-[#E8F4FF]/55">{FOOTER.legal}</p>
      </Container>
    </footer>
  );
}
