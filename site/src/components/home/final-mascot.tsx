"use client";

import * as React from "react";
import { useMotionValue, useSpring } from "framer-motion";
import { MascotRising } from "@/components/shared/mascot-rising";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * The final call's creature and the eye that follows the pointer: the one
 * client leaf of `final-section.tsx` (WEB-5 review fix R5). The section
 * itself, its stars, moon, headline and calls to action render on the
 * server.
 *
 * The eye is active while at least 20 % of the enclosing section is on
 * screen, as before (`useInView(section, { amount: 0.2 })`); the section is
 * a server element now, so it is observed through the stage's
 * `closest("section")`.
 *
 * WEB-5 D14 S5, the surfacing: as the final section scrolls in, the
 * creature rises from 30 % lower in its stage to its resting place, so the
 * return to the surface ends with it coming up through the water. Written
 * to the inner wrapper's transform from a passive listener (no React state
 * per scroll event); under reduced motion it simply rests.
 */
export function FinalMascotStage() {
  const stageRef = React.useRef<HTMLDivElement>(null);
  const riseRef = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();
  const [inView, setInView] = React.useState(false);
  // Resting gaze: up and a little left, towards the headline.
  const tx = useMotionValue(-6);
  const ty = useMotionValue(-22);
  const eyeX = useSpring(tx, { stiffness: 120, damping: 18 });
  const eyeY = useSpring(ty, { stiffness: 120, damping: 18 });

  React.useEffect(() => {
    const section = stageRef.current?.closest("section");
    if (!section) return;
    const io = new IntersectionObserver(
      ([entry]) => setInView(entry.isIntersecting && entry.intersectionRatio >= 0.2),
      { threshold: [0, 0.2] },
    );
    io.observe(section);
    return () => io.disconnect();
  }, []);

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

  React.useEffect(() => {
    const rise = riseRef.current;
    const section = stageRef.current?.closest("section");
    if (!rise || !section) return;
    if (reduced) {
      rise.style.transform = "";
      return;
    }
    let frame = 0;
    const apply = () => {
      frame = 0;
      // 0 when the section's top meets the screen's bottom, 1 when it meets the top.
      const top = section.getBoundingClientRect().top;
      const p = Math.min(1, Math.max(0, 1 - top / window.innerHeight));
      const eased = 1 - Math.pow(1 - p, 3);
      rise.style.transform = `translate3d(0, ${((1 - eased) * 30).toFixed(2)}%, 0)`;
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(apply);
    };
    apply();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
    };
  }, [reduced]);

  return (
    <div ref={stageRef} className="relative z-0 mt-auto w-full max-w-[760px] px-2">
      <div ref={riseRef}>
        <MascotRising className="w-full" eyeX={reduced ? tx : eyeX} eyeY={reduced ? ty : eyeY} />
      </div>
    </div>
  );
}
