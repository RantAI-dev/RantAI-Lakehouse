import * as React from "react";
import type { CommonCopy, HomeCopy } from "@/lib/copy";
import { LINKS } from "@/lib/home-content";
import { Container, CtaLink, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { MascotLogo } from "@/components/shared/mascot-svg";
import { ProgressSection } from "@/components/shared/progress-section";
import { BackdropFade, HeroCopyMotion, ScrollCueFade } from "./hero-motion";

/**
 * S1 — hero. The DOM hero paints first: a CSS night lake with the logo as
 * inline SVG. It is also the permanent fallback when WebGL is missing or the
 * context is lost (plan §2.8), so the copy never depends on the canvas.
 *
 * The section is 150 vh tall with a 100 vh sticky stage: the first 50 vh of
 * scroll pin the hero while its copy lifts and fades and the camera starts
 * to dive; the rest of the dive happens as the stage scrolls away. With
 * `prefers-reduced-motion` the section is a plain 100 vh screen (the CSS
 * `motion-reduce:` variants), so no pinned scroll remains.
 *
 * WEB-5 review fix R5: a server component. The copy, the backdrop's drawing
 * and the cue render on the server; `hero-motion.tsx` holds the three
 * client leaves that move them.
 */
export function HeroSection({ copy, cta }: { copy: HomeCopy["hero"]; cta: CommonCopy["cta"] }) {
  return (
    <ProgressSection range={["start start", "end start"]} id="hero" className="relative h-[150vh] motion-reduce:h-auto">
      <div className="sticky top-0 isolate h-[100svh] overflow-hidden motion-reduce:relative motion-reduce:min-h-[100svh]">
        <HeroBackdrop />
        {/* A soft dark wash behind the copy keeps it above 4.5:1 over any frame of the canvas. */}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 bg-[linear-gradient(90deg,rgba(2,5,20,0.72)_0%,rgba(2,5,20,0.45)_38%,transparent_62%)] max-md:bg-[linear-gradient(0deg,rgba(2,5,20,0.92)_0%,rgba(2,5,20,0.84)_52%,rgba(2,5,20,0.4)_70%,transparent_86%)]"
        />
        <Container className="relative z-10 flex h-full min-h-[100svh] flex-col items-center justify-end pt-24 pb-32 text-center md:items-start md:justify-center md:pb-24 md:text-left">
          <HeroCopyMotion>
            {/* WEB-1 review round 1 SHOULD-FIX 4: on a phone the eyebrow sits on the moon's
                reflection, so it is lighter and carries a dark halo there. */}
            <Eyebrow className="max-md:text-[#a9d6ff] max-md:[text-shadow:0_0_10px_rgba(2,5,20,0.95),0_1px_2px_rgba(2,5,20,0.9)]">
              {copy.eyebrow}
            </Eyebrow>
            <DisplayHeading as="h1" size="xl" className="mt-5">
              {copy.headline}
            </DisplayHeading>
            <LeadText className="mx-auto mt-6 max-w-[34rem] md:mx-0">{copy.sub}</LeadText>
            <div className="mt-9 flex flex-wrap items-center justify-center gap-3 md:justify-start">
              <CtaLink href={LINKS.install}>{cta.install}</CtaLink>
              <CtaLink href={LINKS.talk} variant="ghost">
                {cta.talk}
              </CtaLink>
            </div>
          </HeroCopyMotion>
        </Container>
        <ScrollCueFade>
          <ScrollCue label={copy.scrollCue} />
        </ScrollCueFade>
      </div>
    </ProgressSection>
  );
}

function HeroBackdrop() {
  return (
    <BackdropFade>
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
    </BackdropFade>
  );
}

function ScrollCue({ label }: { label: string }) {
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-6 z-10 flex flex-col items-center gap-3">
      <span className="text-[11px] tracking-[0.24em] text-[#E8F4FF]/60 uppercase">{label}</span>
      <span className="relative block h-12 w-px overflow-hidden bg-[#E8F4FF]/20">
        <span className="lk-drop absolute top-0 left-1/2 block size-1.5 -translate-x-1/2 rounded-full bg-[#44A6FA] shadow-[0_0_10px_2px_rgba(68,166,250,0.8)]" />
      </span>
    </div>
  );
}
