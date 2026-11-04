import * as React from "react";
import type { CommonCopy, HomeCopy } from "@/lib/copy";
import { LINKS } from "@/lib/home-content";
import { Container, CtaLink, DisplayHeading } from "@/components/shared/primitives";
import { Reveal, SplitWords } from "@/components/shared/reveal";
import { FinalMascotStage } from "./final-mascot";

/**
 * S8 — the last call. Back to the night lake: a CSS sky with a few stars and
 * the creature in SVG (lighter than a second canvas), half sunk, its eye
 * turned up to the headline. The eye follows a mouse a little; on touch
 * screens, where there is no pointer to follow, it glances around now and then.
 * With reduced motion it rests.
 *
 * WEB-5 review fix R5: a server component; the creature and its eye are
 * the client leaf in `final-mascot.tsx`.
 */

/** Deterministic star field (integer maths: the server and the browser must agree). */
const STARS = Array.from({ length: 44 }, (_, i) => {
  const h = (n: number) => {
    let x = Math.imul(i + 1, 374761393) ^ Math.imul(n, 668265263);
    x = Math.imul(x ^ (x >>> 13), 1274126177);
    return ((x ^ (x >>> 16)) >>> 0) / 4294967296;
  };
  return {
    left: Math.round(h(1) * 1000) / 10,
    top: Math.round(h(2) * 520) / 10,
    size: h(3) > 0.88 ? 3 : 2,
    delay: Math.round(h(4) * 30) / 10,
  };
});

export function FinalSection({ copy, cta }: { copy: HomeCopy["final"]; cta: CommonCopy["cta"] }) {
  return (
    <section
      id="final"
      className="lk-fallback-bg lk-grain relative isolate flex min-h-[100svh] flex-col items-center overflow-hidden bg-[linear-gradient(180deg,#020514_0%,#050A30_45%,#0A1F55_72%,#0b2a66_100%)] pt-28 md:pt-32"
    >
      <div aria-hidden="true" className="pointer-events-none absolute inset-0 -z-10">
        {STARS.map((s, i) => (
          <span
            key={i}
            className="lk-twinkle absolute rounded-full bg-[#E8F4FF]"
            style={{ left: `${s.left}%`, top: `${s.top}%`, width: s.size, height: s.size, animationDelay: `${s.delay}s` }}
          />
        ))}
        {/* A small moon at the upper left, centred in its own glow. */}
        <div className="absolute top-[13%] left-[13%]">
          <div className="absolute top-1/2 left-1/2 size-[34vmin] -translate-x-1/2 -translate-y-1/2 rounded-full bg-[radial-gradient(circle,rgba(232,244,255,0.2)_0%,rgba(68,166,250,0.08)_36%,transparent_68%)]" />
          <div className="absolute top-1/2 left-1/2 size-3 -translate-x-1/2 -translate-y-1/2 rounded-full bg-[#E8F4FF] shadow-[0_0_26px_7px_rgba(232,244,255,0.4)]" />
        </div>
      </div>

      <Container className="relative z-10 flex flex-col items-center text-center">
        <DisplayHeading className="max-w-[14ch]">
          <SplitWords text={copy.headline} />
        </DisplayHeading>
        <Reveal className="mt-9 flex flex-wrap items-center justify-center gap-3" delay={0.25}>
          <CtaLink href={LINKS.install}>{cta.install}</CtaLink>
          <CtaLink href={LINKS.talk} variant="ghost">
            {cta.talk}
          </CtaLink>
        </Reveal>
      </Container>

      <FinalMascotStage />
    </section>
  );
}
