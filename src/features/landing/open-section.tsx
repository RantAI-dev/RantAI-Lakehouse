"use client";

import * as React from "react";
import { ArrowUpRight } from "lucide-react";
import { CTA, LINKS, OPEN } from "./landing-copy";
import { Container, CtaLink, DisplayHeading } from "./landing-ui";
import { Reveal, SplitWords } from "./reveal";
import { TiltCard } from "./tilt-card";

/** S7 — "Yours to keep": open source, open tables, your servers or ours. */
export function OpenSection() {
  return (
    <section id="open" className="relative scroll-mt-16 bg-[#050A30] py-28 md:py-36">
      <Container>
        <DisplayHeading>
          <SplitWords text={OPEN.headline} />
        </DisplayHeading>
        <ul className="mt-14 grid gap-5 md:grid-cols-3">
          {OPEN.cards.map((c, i) => (
            <Reveal as="li" key={c.n} delay={i * 0.12} y={44} className="[perspective:1000px]">
              <TiltCard className="h-full rounded-3xl border border-[#E8F4FF]/12 bg-[linear-gradient(160deg,rgba(232,244,255,0.07),rgba(232,244,255,0.02))] p-8 shadow-[0_30px_80px_-40px_rgba(2,5,20,0.9)]">
                <span className="lk-display lk-outline-text block text-[96px] leading-[0.9]" aria-hidden="true">
                  {c.n}
                </span>
                <h3 className="mt-7 text-[20px] leading-7 font-medium text-[#E8F4FF]">{c.title}</h3>
                <p className="mt-2 text-[16px] leading-[1.6] text-[#E8F4FF]/80">{c.body}</p>
              </TiltCard>
            </Reveal>
          ))}
        </ul>
        <Reveal className="mt-10" delay={0.2}>
          <CtaLink href={LINKS.source} variant="ghost" target="_blank" rel="noreferrer noopener">
            {CTA.source}
            <ArrowUpRight aria-hidden="true" className="size-4 transition-transform duration-300 group-hover:translate-x-0.5 group-hover:-translate-y-0.5" />
          </CtaLink>
        </Reveal>
      </Container>
    </section>
  );
}
