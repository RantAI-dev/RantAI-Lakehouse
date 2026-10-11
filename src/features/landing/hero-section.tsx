"use client";

import * as React from "react";
import { CTA, HERO, LINKS } from "./landing-copy";
import { Container, CtaLink, DisplayHeading, Eyebrow, LeadText } from "./landing-ui";
import { MascotLogo } from "./mascot-svg";

/**
 * S1 — hero. The DOM hero paints first: a CSS night lake with the logo as
 * inline SVG. It is also the permanent fallback when WebGL is missing or the
 * context is lost (plan §2.8), so the copy never depends on the canvas.
 */
export function HeroSection() {
  return (
    <section id="hero" className="relative isolate min-h-[100svh] overflow-hidden">
      <HeroBackdrop />
      <Container className="relative z-10 flex min-h-[100svh] flex-col items-center justify-end pt-24 pb-32 text-center md:items-start md:justify-center md:pb-24 md:text-left">
        <div className="max-w-[660px]">
          <Eyebrow>{HERO.eyebrow}</Eyebrow>
          <DisplayHeading as="h1" size="xl" className="mt-5">
            {HERO.headline}
          </DisplayHeading>
          <LeadText className="mx-auto mt-6 max-w-[34rem] md:mx-0">{HERO.sub}</LeadText>
          <div className="mt-9 flex flex-wrap items-center justify-center gap-3 md:justify-start">
            <CtaLink href={LINKS.console}>{CTA.console}</CtaLink>
            <CtaLink href={LINKS.talk} variant="ghost">
              {CTA.talk}
            </CtaLink>
          </div>
        </div>
      </Container>
      <ScrollCue />
    </section>
  );
}

function HeroBackdrop() {
  return (
    <div aria-hidden="true" className="absolute inset-0 -z-10">
      {/* Sky to horizon to lake, in the logo's own blues. */}
      <div className="absolute inset-0 bg-[linear-gradient(180deg,#020514_0%,#050A30_38%,#0A1F55_61%,#0B2A66_61.4%,#061440_74%,#030B2A_100%)]" />
      {/* Moon glow and its reflection streak on the water. */}
      <div className="absolute top-[8%] left-[62%] size-[46vmin] -translate-x-1/2 rounded-full bg-[radial-gradient(circle,rgba(232,244,255,0.22)_0%,rgba(68,166,250,0.10)_38%,transparent_70%)]" />
      <div className="absolute top-[62%] left-[62%] h-[34vh] w-[14vw] -translate-x-1/2 bg-[radial-gradient(ellipse_at_top,rgba(68,166,250,0.35)_0%,transparent_70%)] blur-xl" />
      <div className="absolute inset-x-0 top-[61.2%] h-px bg-[#44A6FA]/40" />
      <div className="absolute top-[14%] left-1/2 w-[min(46vw,230px)] -translate-x-1/2 md:top-[18%] md:left-auto md:right-[9%] md:w-[min(38vw,460px)] md:translate-x-0">
        <MascotLogo className="lk-bob w-full drop-shadow-[0_0_60px_rgba(68,166,250,0.25)]" />
      </div>
      <div className="absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-t from-[#020514] via-[#020514]/55 to-transparent md:hidden" />
    </div>
  );
}

function ScrollCue() {
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-6 z-10 flex flex-col items-center gap-3">
      <span className="lk-mono text-[11px] tracking-[0.24em] text-[#E8F4FF]/60 uppercase">{HERO.scrollCue}</span>
      <span className="relative block h-12 w-px overflow-hidden bg-[#E8F4FF]/20">
        <span className="lk-drop absolute top-0 left-1/2 block size-1.5 -translate-x-1/2 rounded-full bg-[#44A6FA] shadow-[0_0_10px_2px_rgba(68,166,250,0.8)]" />
      </span>
    </div>
  );
}
