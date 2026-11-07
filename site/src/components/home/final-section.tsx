import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { Container, DisplayHeading } from "@/components/shared/primitives";
import { SplitWords } from "@/components/shared/reveal";
import { MascotRising } from "@/components/shared/mascot-rising";
import { FinalMascotStage } from "./final-mascot";

/**
 * S8 — the last call. WEB-5 O42 (owner, on O33's whale in the abyss: "aku
 * lebih suka yang lama, kek beneran pemandangan laut"; the owner chose a
 * night sea like the hero): the page closes where it began, on the night
 * lake. A sky with stars and the moon at the upper left, the moonlit water
 * below, and the whale's head floating at the waterline under the headline
 * (`shared/mascot-rising.tsx`, in SVG). WEB-5 O53 (owner: "bagian footer ini
 * kenapa jadi 2d gini"): where the live lake runs, the hero's own canvas is
 * moved in here and draws the same night sea and 3D whale (lake-engine.ts,
 * `setView`); the SVG scene stays for no fast WebGL and no JavaScript. The
 * sky starts on the page's own deepest colour (#020514), so the descent
 * runs into it without a seam; the depth gauge has faded out by here.
 *
 * Its eyes follow a mouse a little; on touch screens they glance around now
 * and then. With reduced motion it rests; without JavaScript the scene is
 * static.
 *
 * WEB-5 review fix R5: a server component; the creature and its eyes are
 * the client leaf in `final-mascot.tsx`.
 */

/** Deterministic star field (integer maths: the server and the browser must agree). */
const STARS = Array.from({ length: 40 }, (_, i) => {
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
      // O42: the night sky is the scene, so it shows with and without JavaScript (no
      // `lk-fallback-bg`): the page's deepest colour at the top, the hero's night blue
      // towards the horizon, where the stage's water begins.
      className="group/final relative isolate flex min-h-[100svh] flex-col items-center overflow-hidden bg-[linear-gradient(180deg,#020514_0%,#050a30_55%,#0a1f55_100%)] pt-28 max-md:min-h-0 max-md:gap-8 max-md:pb-6 md:pt-32"
    >
      {/* WEB-5 O53: where the live lake runs, the engine moves its canvas in here when the section
          is near, and marks the section `data-scene3d="on"` once it has drawn; the 2D sky, moon and
          sea below then fade out. The top and the bottom fade to the page's #020514, so the descent
          above and the footer below meet it without a seam. */}
      <div aria-hidden="true" data-final-scene="" className="pointer-events-none absolute inset-0 -z-10" />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 top-0 -z-10 h-[24%] bg-[linear-gradient(180deg,#020514_0%,rgba(2,5,20,0.6)_45%,rgba(2,5,20,0)_100%)] opacity-0 transition-opacity duration-400 group-data-[scene3d=on]/final:opacity-100"
      />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 bottom-0 -z-10 h-[12%] bg-[linear-gradient(0deg,#020514_0%,rgba(2,5,20,0)_100%)] opacity-0 transition-opacity duration-400 group-data-[scene3d=on]/final:opacity-100"
      />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 -z-20 transition-[opacity,visibility] duration-400 group-data-[scene3d=on]/final:invisible group-data-[scene3d=on]/final:opacity-0"
      >
        {STARS.map((s, i) => (
          <span
            key={i}
            className="lk-twinkle absolute rounded-full bg-[#E8F4FF]"
            style={{ left: `${s.left}%`, top: `${s.top}%`, width: s.size, height: s.size, animationDelay: `${s.delay}s` }}
          />
        ))}
        {/* A small moon at the upper left, centred in its own glow, as in the hero. */}
        <div className="absolute top-[13%] left-[13%]">
          <div className="absolute top-1/2 left-1/2 size-[34vmin] -translate-x-1/2 -translate-y-1/2 rounded-full bg-[radial-gradient(circle,rgba(232,244,255,0.18)_0%,rgba(68,166,250,0.07)_36%,transparent_68%)]" />
          <div className="absolute top-1/2 left-1/2 size-3 -translate-x-1/2 -translate-y-1/2 rounded-full bg-[#E8F4FF] shadow-[0_0_26px_7px_rgba(232,244,255,0.4)]" />
        </div>
      </div>

      <Container className="relative z-10 flex flex-col items-center text-center">
        <DisplayHeading className="max-w-[14ch]">
          <SplitWords text={copy.headline} />
        </DisplayHeading>
        {/* WEB-5 O7: no buttons here. This is the story's end; the footer's card right below is the action,
            so the page never shows two calls to action in a row. */}
      </Container>

      <FinalMascotStage>
        <MascotRising className="w-full transition-[opacity,visibility] duration-400 group-data-[scene3d=on]/final:invisible group-data-[scene3d=on]/final:opacity-0" />
      </FinalMascotStage>
    </section>
  );
}
