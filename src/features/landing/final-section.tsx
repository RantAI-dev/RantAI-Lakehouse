"use client";

import * as React from "react";
import { useInView, useMotionValue, useSpring } from "motion/react";
import { CTA, FINAL, LINKS } from "./landing-copy";
import { Container, CtaLink, DisplayHeading } from "./landing-ui";
import { MascotRising } from "./mascot-svg";
import { Reveal, SplitWords } from "./reveal";
import { useReducedMotionSafe } from "./use-reduced-motion";

/**
 * S8 — the last call. Back to the night lake: a CSS sky with a few stars and
 * the creature in SVG (lighter than a second canvas), half sunk, its eye
 * turned up to the headline. The eye follows a mouse a little; on touch
 * screens, where there is no pointer to follow, it glances around now and then.
 * With reduced motion it rests.
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

export function FinalSection() {
  const ref = React.useRef<HTMLElement>(null);
  const stageRef = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();
  const inView = useInView(ref, { amount: 0.2 });
  // Resting gaze: up and a little left, towards the headline.
  const tx = useMotionValue(-6);
  const ty = useMotionValue(-22);
  const eyeX = useSpring(tx, { stiffness: 120, damping: 18 });
  const eyeY = useSpring(ty, { stiffness: 120, damping: 18 });

  React.useEffect(() => {
    if (reduced || !inView) return;
    const stage = stageRef.current;
    if (!stage) return;
    let lastMove = 0;
    const onMove = (e: PointerEvent) => {
      if (e.pointerType === "touch") return;
      lastMove = performance.now();
      const r = stage.getBoundingClientRect();
      const dx = (e.clientX - (r.left + r.width / 2)) / window.innerWidth;
      const dy = (e.clientY - (r.top + r.height * 0.35)) / window.innerHeight;
      tx.set(Math.max(-18, Math.min(18, -6 + dx * 46)));
      ty.set(Math.max(-26, Math.min(8, -22 + dy * 40)));
    };
    window.addEventListener("pointermove", onMove, { passive: true });
    // With no pointer in play, glance somewhere every few seconds, then back up.
    const id = window.setInterval(() => {
      if (performance.now() - lastMove < 6000) return;
      const glance = Math.random() < 0.5;
      tx.set(glance ? -6 + (Math.random() - 0.5) * 28 : -6);
      ty.set(glance ? -22 + (Math.random() - 0.3) * 18 : -22);
    }, 3200);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.clearInterval(id);
    };
  }, [reduced, inView, tx, ty]);

  return (
    <section
      ref={ref}
      id="final"
      className="lk-grain relative isolate flex min-h-[100svh] flex-col items-center overflow-hidden bg-[linear-gradient(180deg,#020514_0%,#050A30_45%,#0A1F55_72%,#0b2a66_100%)] pt-28 md:pt-32"
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
          <SplitWords text={FINAL.headline} />
        </DisplayHeading>
        <Reveal className="mt-9 flex flex-wrap items-center justify-center gap-3" delay={0.25}>
          <CtaLink href={LINKS.console}>{CTA.console}</CtaLink>
          <CtaLink href={LINKS.talk} variant="ghost">
            {CTA.talk}
          </CtaLink>
        </Reveal>
      </Container>

      <div ref={stageRef} className="relative z-0 mt-auto w-full max-w-[760px] px-2">
        <MascotRising className="w-full" eyeX={reduced ? tx : eyeX} eyeY={reduced ? ty : eyeY} />
      </div>
    </section>
  );
}
