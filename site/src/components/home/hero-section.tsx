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
 * The section has a 100 vh sticky stage. With the live lake it is 220 vh
 * tall (WEB-5 D14 S1, `app/globals.css`): the stage stays pinned while the
 * camera descends, lingers at the waterline and goes under, and the copy
 * fades just before the surface closes overhead. Without it (the DOM hero)
 * it is 150 vh, as before. With `prefers-reduced-motion` the section is a
 * plain 100 vh screen (the CSS `motion-reduce:` variants), so no pinned
 * scroll remains.
 *
 * WEB-5 review fix R5: a server component. The copy, the backdrop's drawing
 * and the cue render on the server; `hero-motion.tsx` holds the three
 * client leaves that move them.
 */
export function HeroSection({ copy, cta }: { copy: HomeCopy["hero"]; cta: CommonCopy["cta"] }) {
  return (
    <ProgressSection range={["start start", "end start"]} id="hero" className="relative h-[150vh] motion-reduce:h-auto">
      <div className="sticky top-0 isolate flex h-[100svh] flex-col overflow-hidden motion-reduce:relative motion-reduce:min-h-[100svh]">
        <HeroBackdrop />
        {/* A soft dark wash behind the copy keeps it above 4.5:1 over any frame of the canvas. */}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 bg-[linear-gradient(90deg,rgba(2,5,20,0.72)_0%,rgba(2,5,20,0.45)_38%,transparent_62%)] max-md:bg-[linear-gradient(0deg,rgba(2,5,20,0.92)_0%,rgba(2,5,20,0.84)_52%,rgba(2,5,20,0.4)_70%,transparent_86%)]"
        />
        <HeroCreature />
        <Container className="relative z-10 flex flex-none flex-col items-center pt-4 pb-32 text-center short:pb-24 md:min-h-[100svh] md:items-start md:justify-center md:pt-24 md:pb-24 md:text-left">
          <HeroCopyMotion>
            {/* WEB-1 review round 1 SHOULD-FIX 4: on a phone the eyebrow sits on the moon's
                reflection, so it is lighter and carries a dark halo there. */}
            <Eyebrow className="max-md:text-[#a9d6ff] max-md:[text-shadow:0_0_10px_rgba(2,5,20,0.95),0_1px_2px_rgba(2,5,20,0.9)]">
              {copy.eyebrow}
            </Eyebrow>
            <DisplayHeading as="h1" size="xl" className="mt-5 short:mt-3 short:text-[3.25rem]">
              {copy.headline}
            </DisplayHeading>
            <LeadText className="mx-auto mt-6 max-w-[34rem] short:mt-4 short:text-[15px] short:leading-[1.5] md:mx-0">{copy.sub}</LeadText>
            <div className="mt-9 short:mt-6 flex flex-wrap items-center justify-center gap-3 md:justify-start">
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
      <div className="absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-t from-[#020514] via-[#020514]/55 to-transparent md:hidden" />
    </BackdropFade>
  );
}

/**
 * The creature of the DOM hero.
 *
 * WEB-5 review fix R12 (BLOCKER): on a phone it used to have a fixed place
 * (14 % down) and size (46 vw) while the copy was set at the bottom of
 * the stage, so on a short screen the copy rode up over it (360×740 EN: the
 * eyebrow 128 px above the creature's bottom edge). Now, below `md`, the
 * stage is a column: this slot takes the height the copy leaves free, and
 * the copy sits under it with a 16 px gap (the copy's `pt-4`: 12 px, plus the
 * bob's 0.6° tilt, which widens the drawing's box by about 1 px). The gap
 * above the creature (14 svh, at least 4.5 rem to clear the top bar) and
 * the creature itself shrink only when that free height runs out, so where
 * there was room (390×844 and up) both keep their old place and size. On
 * short screens (`short`, app/globals.css) the copy is set tighter too, so
 * the creature keeps a readable size and both buttons stay on the first
 * screen. The live lake frames its 3D creature into this box when its own
 * framing would reach the copy (lake-engine.ts, `data-hero-creature`).
 *
 * From `md` up it is placed on the right of the stage as before. It stays
 * under the copy's dark wash (`-z-[5]`, between the backdrop and the wash),
 * where it was when it lived in the backdrop.
 */
function HeroCreature() {
  return (
    <BackdropFade
      data-hero-room=""
      className="relative -z-[5] flex min-h-0 flex-1 flex-col md:absolute md:top-[18%] md:right-[9%] md:block md:w-[min(38vw,460px)]"
    >
      <div className="h-[14svh] min-h-[4.5rem] md:hidden" />
      <div
        data-hero-creature=""
        className="relative left-1/2 aspect-square h-[min(46vw,230px)] min-h-0 -translate-x-1/2 self-start md:static md:h-auto md:w-full md:translate-x-0"
      >
        <MascotLogo className="lk-bob size-full drop-shadow-[0_0_60px_rgba(68,166,250,0.25)]" />
      </div>
    </BackdropFade>
  );
}

function ScrollCue({ label }: { label: string }) {
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-6 z-10 flex flex-col items-center gap-3">
      <span className="text-[11px] tracking-[0.24em] text-[#E8F4FF]/60 uppercase">{label}</span>
      <span className="relative block h-12 w-px short:h-8 overflow-hidden bg-[#E8F4FF]/20">
        <span className="lk-drop absolute top-0 left-1/2 block size-1.5 -translate-x-1/2 rounded-full bg-[#44A6FA] shadow-[0_0_10px_2px_rgba(68,166,250,0.8)]" />
      </span>
    </div>
  );
}
