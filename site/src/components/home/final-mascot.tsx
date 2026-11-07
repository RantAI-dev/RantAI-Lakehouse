"use client";

import * as React from "react";
import { useMotionValue, useSpring } from "framer-motion";
import { FinalWhaleProvider } from "@/components/shared/whale-motion";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * The final call's whale and the eyes that follow the pointer: the one
 * client leaf of `final-section.tsx` (WEB-5 review fix R5). The section
 * itself, its headline and calls to action, and the whale's art (O24,
 * `shared/mascot-rising.tsx`) render on the server.
 *
 * WEB-5 O33: the final call is the bottom of the dive, and the whale swims
 * down to greet the visitor there. As the section scrolls in, it comes in
 * from the right and a little above, nose down, and levels out in its
 * place, all inside its own band below the buttons, so it never passes
 * over the text; when it arrives it gives a brief glow (`data-arrived`). Its
 * eyes (O29: plain whites) shift a little towards the pointer, in the
 * whale's units: ±4 across, -5 to +2 up and down. Written to the swim
 * layer's style from a passive listener (no React state per scroll event);
 * under reduced motion it simply rests in place.
 *
 * The eyes follow while at least 20 % of the enclosing section is on
 * screen (observed through the stage's `closest("section")`).
 */
export function FinalMascotStage({ children }: { children: React.ReactNode }) {
  const stageRef = React.useRef<HTMLDivElement>(null);
  const swimRef = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();
  const [inView, setInView] = React.useState(false);
  // Resting gaze: up and a little left, towards the headline.
  const tx = useMotionValue(-1);
  const ty = useMotionValue(-3);
  const eyeX = useSpring(tx, { stiffness: 120, damping: 18 });
  const eyeY = useSpring(ty, { stiffness: 120, damping: 18 });
  const motion = React.useMemo(
    () => ({ eyeX: reduced ? tx : eyeX, eyeY: reduced ? ty : eyeY, swimRef }),
    [reduced, tx, ty, eyeX, eyeY],
  );

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
      tx.set(Math.max(-4, Math.min(4, -1 + dx * 11)));
      ty.set(Math.max(-5, Math.min(2, -3 + dy * 9)));
    };
    window.addEventListener("pointermove", onMove, { passive: true });
    // With no pointer in play, glance somewhere every few seconds, then back up.
    const id = window.setInterval(() => {
      if (performance.now() - lastMove < 6000) return;
      const glance = Math.random() < 0.5;
      tx.set(glance ? -1 + (Math.random() - 0.5) * 6 : -1);
      ty.set(glance ? -3 + (Math.random() - 0.3) * 4 : -3);
    }, 3200);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.clearInterval(id);
    };
  }, [reduced, inView, tx, ty]);

  React.useEffect(() => {
    const swim = swimRef.current;
    const stage = stageRef.current;
    const section = stage?.closest("section");
    if (!swim || !stage || !section) return;
    if (reduced) {
      swim.style.transform = "";
      swim.style.opacity = "";
      stage.dataset.arrived = "off";
      return;
    }
    let frame = 0;
    const apply = () => {
      frame = 0;
      // 0 when the section's top meets the screen's bottom, 1 when it meets the top.
      const top = section.getBoundingClientRect().top;
      const p = Math.min(1, Math.max(0, 1 - top / window.innerHeight));
      const eased = 1 - Math.pow(1 - p, 3);
      // From the right and a little above, nose down, out of the dark.
      const k = 1 - eased;
      swim.style.transform = `translate3d(${(k * 58).toFixed(2)}%, ${(k * -9).toFixed(2)}%, 0) rotate(${(k * -14).toFixed(2)}deg)`;
      swim.style.opacity = Math.min(1, eased * 1.6).toFixed(3);
      if (eased > 0.985 && stage.dataset.arrived !== "on") stage.dataset.arrived = "on";
      else if (p < 0.4 && stage.dataset.arrived === "on") stage.dataset.arrived = "off";
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
    <div ref={stageRef} data-arrived="off" className="relative z-0 mt-auto w-full max-w-[760px] overflow-x-clip px-2">
      {/* O24/O33: the whale and its light are server-rendered children; they read the gaze and the swim from here. */}
      <FinalWhaleProvider value={motion}>{children}</FinalWhaleProvider>
    </div>
  );
}
