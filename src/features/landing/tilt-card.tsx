"use client";

import * as React from "react";
import { motion, useMotionTemplate, useMotionValue, useSpring } from "motion/react";
import { cn } from "@/lib/utils";
import { useReducedMotionSafe } from "./use-reduced-motion";

/**
 * A card that leans towards the pointer (at most 8 degrees) with a glossy
 * sheen that follows it. Only a real mouse or pen triggers it: touch has no
 * hover, and tilting under a finger that is scrolling would fight the page.
 * With reduced motion the card is plain.
 */
export function TiltCard({
  className,
  children,
  max = 8,
}: {
  className?: string;
  children: React.ReactNode;
  max?: number;
}) {
  const reduced = useReducedMotionSafe();
  const rx = useSpring(0, { stiffness: 220, damping: 22, mass: 0.6 });
  const ry = useSpring(0, { stiffness: 220, damping: 22, mass: 0.6 });
  const mx = useMotionValue(50);
  const my = useMotionValue(50);
  const sheen = useSpring(0, { stiffness: 160, damping: 24 });
  const gloss = useMotionTemplate`radial-gradient(260px circle at ${mx}% ${my}%, rgba(232,244,255,0.20), rgba(68,166,250,0.06) 45%, transparent 70%)`;

  const onMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (reduced || e.pointerType === "touch") return;
    const r = e.currentTarget.getBoundingClientRect();
    const px = (e.clientX - r.left) / r.width;
    const py = (e.clientY - r.top) / r.height;
    ry.set((px - 0.5) * 2 * max);
    rx.set(-(py - 0.5) * 2 * max);
    mx.set(px * 100);
    my.set(py * 100);
    sheen.set(1);
  };
  const onLeave = () => {
    rx.set(0);
    ry.set(0);
    sheen.set(0);
  };

  return (
    <motion.div
      className={cn("relative [transform-style:preserve-3d]", className)}
      style={reduced ? undefined : { rotateX: rx, rotateY: ry, transformPerspective: 900 }}
      onPointerMove={onMove}
      onPointerLeave={onLeave}
    >
      {children}
      {!reduced && (
        <motion.span
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 rounded-[inherit]"
          style={{ background: gloss, opacity: sheen }}
        />
      )}
    </motion.div>
  );
}
