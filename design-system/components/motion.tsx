"use client";

/**
 * RantAI Design System — Motion Wrapper Components
 *
 * Lightweight wrappers around motion/react for common
 * viewport-driven entrance animations.
 *
 * All animations trigger once when the element enters the viewport.
 *
 * Dependencies: motion (npm install motion)
 *
 * @example
 * ```tsx
 * import { FadeUp, StaggerContainer, StaggerItem } from "@/design-system/components/motion";
 *
 * <FadeUp>
 *   <h2>Section heading</h2>
 * </FadeUp>
 *
 * <StaggerContainer>
 *   {items.map(item => <StaggerItem key={item.id}>{item.name}</StaggerItem>)}
 * </StaggerContainer>
 * ```
 */

import { motion } from "motion/react";
import type { ReactNode } from "react";

/* Pre-create motion elements outside of render to avoid
   "Cannot create components during render" errors. */
const motionElements = {
  div:     motion.create("div"),
  section: motion.create("section"),
  article: motion.create("article"),
  header:  motion.create("header"),
  footer:  motion.create("footer"),
  aside:   motion.create("aside"),
} as const;

/* ─────────────────────────────────────────────
   FadeUp
   Fades in from below — default entrance for sections
───────────────────────────────────────────── */
type FadeUpProps = {
  children: ReactNode;
  delay?: number;
  duration?: number;
  className?: string;
  as?: keyof typeof motionElements;
};

export function FadeUp({
  children,
  delay = 0,
  duration = 0.6,
  className,
  as = "div",
}: FadeUpProps) {
  const Component = motionElements[as];
  return (
    <Component
      initial={{ opacity: 0, y: 32 }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true, margin: "-80px" }}
      transition={{ duration, delay, ease: "easeOut" }}
      className={className}
    >
      {children}
    </Component>
  );
}

/* ─────────────────────────────────────────────
   StaggerContainer
   Parent that staggers child animations
───────────────────────────────────────────── */
type StaggerContainerProps = {
  children: ReactNode;
  stagger?: number;
  delay?: number;
  className?: string;
};

export function StaggerContainer({
  children,
  stagger = 0.1,
  delay = 0,
  className,
}: StaggerContainerProps) {
  return (
    <motion.div
      initial="hidden"
      whileInView="visible"
      viewport={{ once: true, margin: "-80px" }}
      variants={{
        hidden: {},
        visible: {
          transition: {
            staggerChildren: stagger,
            delayChildren: delay,
          },
        },
      }}
      className={className}
    >
      {children}
    </motion.div>
  );
}

/* ─────────────────────────────────────────────
   StaggerItem
   Child of StaggerContainer — fades in from below
───────────────────────────────────────────── */
type StaggerItemProps = {
  children: ReactNode;
  className?: string;
};

export function StaggerItem({ children, className }: StaggerItemProps) {
  return (
    <motion.div
      variants={{
        hidden: { opacity: 0, y: 24 },
        visible: {
          opacity: 1,
          y: 0,
          transition: { duration: 0.5, ease: "easeOut" },
        },
      }}
      className={className}
    >
      {children}
    </motion.div>
  );
}

/* ─────────────────────────────────────────────
   ScaleIn
   Scales from 90% — for cards, badges, images
───────────────────────────────────────────── */
type ScaleInProps = {
  children: ReactNode;
  delay?: number;
  duration?: number;
  className?: string;
};

export function ScaleIn({
  children,
  delay = 0,
  duration = 0.5,
  className,
}: ScaleInProps) {
  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.9 }}
      whileInView={{ opacity: 1, scale: 1 }}
      viewport={{ once: true, margin: "-60px" }}
      transition={{ duration, delay, ease: "easeOut" }}
      className={className}
    >
      {children}
    </motion.div>
  );
}

/* ─────────────────────────────────────────────
   SlideIn
   Slides in from left or right — for split layouts
───────────────────────────────────────────── */
type SlideInProps = {
  children: ReactNode;
  direction?: "left" | "right";
  delay?: number;
  duration?: number;
  className?: string;
};

export function SlideIn({
  children,
  direction = "left",
  delay = 0,
  duration = 0.6,
  className,
}: SlideInProps) {
  return (
    <motion.div
      initial={{ opacity: 0, x: direction === "left" ? -48 : 48 }}
      whileInView={{ opacity: 1, x: 0 }}
      viewport={{ once: true, margin: "-80px" }}
      transition={{ duration, delay, ease: "easeOut" }}
      className={className}
    >
      {children}
    </motion.div>
  );
}
