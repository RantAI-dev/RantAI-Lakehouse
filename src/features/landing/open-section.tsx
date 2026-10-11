"use client";

import * as React from "react";
import { CTA, LINKS, OPEN } from "./landing-copy";
import { Container, CtaLink, DisplayHeading } from "./landing-ui";

/** S7 — "Yours to keep": open source, open tables, your servers or ours. */
export function OpenSection() {
  return (
    <section id="open" className="relative bg-[#050A30] py-28 md:py-36">
      <Container>
        <DisplayHeading>{OPEN.headline}</DisplayHeading>
        <ul className="mt-14 grid gap-5 md:grid-cols-3">
          {OPEN.cards.map((c) => (
            <li key={c.n} className="rounded-3xl border border-[#E8F4FF]/10 bg-[#E8F4FF]/[0.04] p-8">
              <span className="lk-display lk-outline-text block text-[88px] leading-none">{c.n}</span>
              <h3 className="mt-6 text-[20px] font-medium">{c.title}</h3>
              <p className="mt-2 text-[16px] leading-[1.6] text-[#E8F4FF]/80">{c.body}</p>
            </li>
          ))}
        </ul>
        <div className="mt-10">
          <CtaLink href={LINKS.source} variant="ghost" target="_blank" rel="noreferrer noopener">
            {CTA.source}
          </CtaLink>
        </div>
      </Container>
    </section>
  );
}
