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
 */
export function FinalMascotStage() {
  const stageRef = React.useRef<HTMLDivElement>(null);
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

  return (
    <div ref={stageRef} className="relative z-0 mt-auto w-full max-w-[760px] px-2">
      <MascotRising className="w-full" eyeX={reduced ? tx : eyeX} eyeY={reduced ? ty : eyeY} />
    </div>
  );
}
