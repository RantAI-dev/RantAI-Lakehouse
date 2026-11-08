import * as React from "react";
import { cn } from "@/lib/utils";
import type { CommonCopy, HomeCopy } from "@/lib/copy";
import { Container, CtaLink, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { WhaleSvg } from "@/components/shared/whale-svg";
import { ProgressSection } from "@/components/shared/progress-section";
import { BackdropFade, HeroCopyMotion, HeroWashFade, ScrollCueFade } from "./hero-motion";

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
export function HeroSection({
  copy,
  cta,
  contactHref,
}: {
  copy: HomeCopy["hero"];
  cta: CommonCopy["cta"];
  /** WEB-5 D15: "Talk to us" goes to the contact page, in the page's language. */
  contactHref: string;
}) {
  return (
    <ProgressSection range={["start start", "end start"]} id="hero" className="relative h-[150vh] motion-reduce:h-auto">
      <div className="sticky top-0 isolate flex h-[100svh] flex-col overflow-hidden motion-reduce:relative motion-reduce:min-h-[100svh]">
        <HeroBackdrop />
        {/* A soft dark wash behind the copy keeps it above 4.5:1 over any frame of the canvas.
            WEB-5 O5: it leaves with the copy over the live lake (HeroWashFade). */}
        <HeroWashFade>
          <div className="absolute inset-0 bg-[linear-gradient(90deg,rgba(2,5,20,0.72)_0%,rgba(2,5,20,0.45)_38%,transparent_62%)] max-md:bg-[linear-gradient(0deg,rgba(2,5,20,0.92)_0%,rgba(2,5,20,0.84)_52%,rgba(2,5,20,0.4)_70%,transparent_86%)]" />
        </HeroWashFade>
        {/* Task A: the DOM hero's stage ends on one colour across its width (#030b2a, the colour the
            scene zone's fallback gradient starts on, scene-zone.tsx), so its bottom edge leaves no line. */}
        <BackdropFade className="pointer-events-none absolute inset-x-0 bottom-0 h-[28%] bg-[linear-gradient(180deg,rgba(3,11,42,0)_0%,rgba(3,11,42,0.7)_55%,#030b2a_100%)]">
          {null}
        </BackdropFade>
        <HeroCreature />
        <Container className="relative z-10 flex flex-none flex-col items-center pt-4 pb-32 text-center short:pb-24 md:min-h-[100svh] md:items-start md:justify-center md:pt-24 md:pb-24 md:text-left">
          <HeroCopyMotion>
            {/* WEB-1 review round 1 SHOULD-FIX 4: on a phone the eyebrow sits on the moon's
                reflection, so it is lighter and carries a dark halo there. */}
            {/* O50 (audit #7): who it is for, not the product name the top bar already shows. */}
            <Eyebrow className="text-balance max-md:text-[#a9d6ff] max-md:[text-shadow:0_0_10px_rgba(2,5,20,0.95),0_1px_2px_rgba(2,5,20,0.9)]">
              {copy.eyebrow}
            </Eyebrow>
            <DisplayHeading as="h1" size="xl" className="mt-5 short:mt-3 short:text-[2.75rem]">
              {copy.headline}
            </DisplayHeading>
            <LeadText className="mx-auto mt-6 max-w-[34rem] short:mt-4 short:text-[15px] short:leading-[1.5] md:mx-0">{copy.sub}</LeadText>
            <div className="mt-9 short:mt-6 flex flex-wrap items-center justify-center gap-3 md:justify-start">
              {/* WEB-5 D20: the install path is on the page now; it links on to the README. */}
              <CtaLink href="#get-started">{cta.install}</CtaLink>
              <CtaLink href={contactHref} variant="ghost">
                {cta.talk}
              </CtaLink>
            </div>
            {/* WEB-5 D21 + O50 (audit #7): the doubts answered where people decide, in one quiet line
                with a dot between the parts; each part is a claim. On a phone the first part takes the
                first line and the other two share the second, so no part is ever left alone. */}
            <ul className="mt-6 flex flex-wrap items-center justify-center gap-y-1 text-[13px] leading-5 text-[#E8F4FF]/70 short:mt-4 short:text-[12px] md:justify-start">
              {copy.reassurance.map((part, i) => (
                <li
                  key={part}
                  className={cn(
                    "whitespace-nowrap",
                    i > 0 && "before:mx-2 before:text-[#44A6FA] before:content-['·']",
                    i === 0 && "max-sm:basis-full max-sm:text-center",
                    i === 1 && "max-sm:before:hidden",
                  )}
                >
                  {part}
                </li>
              ))}
            </ul>
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
      {/* WEB-5 audit #11: the waterline is wavy and feathered, and kept out of the text column
          (only under the creature on the right; none on phones, where the copy spans the width). */}
      <svg
        viewBox="0 0 1200 12"
        preserveAspectRatio="none"
        className="absolute inset-x-0 top-[calc(61.2%-6px)] h-3 w-full max-md:hidden [mask-image:linear-gradient(90deg,transparent_0%,transparent_50%,#000_64%,#000_90%,transparent_100%)]"
        aria-hidden="true"
      >
        <path
          d="M0 6 Q 75 3 150 6 T 300 6 T 450 6 T 600 6 T 750 6 T 900 6 T 1050 6 T 1200 6"
          fill="none"
          stroke="#44A6FA"
          strokeOpacity="0.35"
          strokeWidth="1"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
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
      // O2: the official mark is 196 x 167, so a box as wide as before is
      // 0.117 of its width less tall; the extra top offset keeps the horizon
      // gap where it was against the backdrop's waterline.
      className="relative -z-[5] flex min-h-0 flex-1 flex-col md:absolute md:top-[calc(18%+min(4.45vw,54px))] md:right-[9%] md:block md:w-[min(38vw,460px)]"
    >
      <div className="h-[14svh] min-h-[4.5rem] md:hidden" />
      <div
        data-hero-creature=""
        className="relative left-1/2 aspect-[196/167] h-[min(46vw,230px)] min-h-0 -translate-x-1/2 self-start md:static md:h-auto md:w-full md:translate-x-0"
      >
        {/* WEB-5 audit #27: the same character as the live lake (the O29 head at its waterline,
            plain white eyes, no glow), not the logo's dome and bowl, with a little water of its own.
            Phones: the head is in the room above the copy, so its own water is under it (waterline
            at 72 % of the box). From md: the waterline is at 96 % of the box, just below the
            backdrop's horizon, so the head sits in the lake rather than above it. */}
        <WhaleSvg surface water spout={false} idPrefix="hero-m" viewBox="104 78 392 334" className="lk-bob size-full md:hidden" />
        <WhaleSvg surface water spout={false} idPrefix="hero-d" viewBox="104 -3 392 334" className="lk-bob size-full max-md:hidden" />
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
