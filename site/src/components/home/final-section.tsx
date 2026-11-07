import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { Container, DisplayHeading } from "@/components/shared/primitives";
import { SplitWords } from "@/components/shared/reveal";
import { MascotRising } from "@/components/shared/mascot-rising";
import { FinalMascotStage } from "./final-mascot";

/**
 * S8 — the last call, at the bottom of the dive (WEB-5 O33: one descent, so
 * no night sky and no moon here any more). The whale swims down to greet
 * the visitor and settles below the headline (O24, in SVG: lighter than a
 * second canvas; O39: on solid water, with no halo). Its eyes follow a mouse a
 * little; on touch screens, where there is no pointer to follow, they
 * glance around now and then. With reduced motion it rests.
 *
 * WEB-5 review fix R5: a server component; the creature and its eye are
 * the client leaf in `final-mascot.tsx`.
 */

/**
 * Sparse marine snow over the section, very dim (O33: specks in the deep,
 * not stars). Deterministic (integer maths: the server and the browser must
 * agree).
 */
const STARS = Array.from({ length: 22 }, (_, i) => {
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

export function FinalSection({ copy }: { copy: HomeCopy["final"] }) {
  return (
    <section
      id="final"
      // O7 + O33: without the buttons, a full-screen section left a phone a screen of empty
      // water between the headline and the whale (the stage sits at the bottom, `mt-auto`).
      // On phones the section is as tall as its content, so the whale follows the headline.
      className="lk-fallback-bg relative isolate flex min-h-[100svh] flex-col items-center overflow-hidden bg-[#020514] pt-28 max-md:min-h-0 max-md:gap-8 max-md:pb-6 md:pt-32"
    >
      <div aria-hidden="true" className="pointer-events-none absolute inset-0 -z-10">
        {STARS.map((s, i) => (
          <span
            key={i}
            className="absolute rounded-full bg-[#9fc4ee]/25"
            style={{ left: `${s.left}%`, top: `${s.top}%`, width: s.size - 0.5, height: s.size - 0.5 }}
          />
        ))}
      </div>

      <Container className="relative z-10 flex flex-col items-center text-center">
        <DisplayHeading className="max-w-[14ch]">
          <SplitWords text={copy.headline} />
        </DisplayHeading>
        {/* WEB-5 O7: no buttons here. This is the story's end; the footer's card right below is the action,
            so the page never shows two calls to action in a row. */}
      </Container>

      <FinalMascotStage>
        <MascotRising className="w-full" />
      </FinalMascotStage>
    </section>
  );
}
