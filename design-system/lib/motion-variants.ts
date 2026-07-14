/**
 * RantAI Design System — Motion Variants
 *
 * Reusable animation variant objects for motion/react.
 * Use with motion() components or pass to `variants` prop.
 *
 * @example
 * ```tsx
 * import { fadeInUp, defaultTransition, defaultViewport } from "@/design-system/lib/motion-variants";
 *
 * <motion.div
 *   {...fadeInUp}
 *   transition={defaultTransition}
 *   viewport={defaultViewport}
 * />
 * ```
 */

/** Fade in from below — most common section entrance */
export const fadeInUp = {
  initial: { opacity: 0, y: 20 },
  animate: { opacity: 1, y: 0 },
};

/** Fade in from the left */
export const fadeInLeft = {
  initial: { opacity: 0, x: -20 },
  animate: { opacity: 1, x: 0 },
};

/** Fade in from the right */
export const fadeInRight = {
  initial: { opacity: 0, x: 20 },
  animate: { opacity: 1, x: 0 },
};

/** Scale in from 90% */
export const scaleIn = {
  initial: { opacity: 0, scale: 0.9 },
  animate: { opacity: 1, scale: 1 },
};

/** Default eased transition — 0.6s easeOut */
export const defaultTransition = {
  duration: 0.6,
  ease: "easeOut" as const,
};

/** Fast transition — 0.4s easeOut */
export const fastTransition = {
  duration: 0.4,
  ease: "easeOut" as const,
};

/**
 * Default viewport config — triggers once, 80px inside viewport.
 * Pass to the `viewport` prop on whileInView animations.
 */
export const defaultViewport = {
  once: true,
  margin: "0px 0px -80px 0px",
};

/**
 * Stagger parent variants.
 * Use with StaggerContainer or motion.div variants.
 *
 * @example
 * ```tsx
 * <motion.ul
 *   initial="hidden"
 *   whileInView="visible"
 *   viewport={defaultViewport}
 *   variants={staggerContainer()}
 * >
 *   {items.map(item => (
 *     <motion.li key={item.id} variants={staggerItem}>...</motion.li>
 *   ))}
 * </motion.ul>
 * ```
 */
export const staggerContainer = (stagger = 0.1, delay = 0) => ({
  hidden: {},
  visible: {
    transition: {
      staggerChildren: stagger,
      delayChildren: delay,
    },
  },
});

/** Individual child variant for stagger lists */
export const staggerItem = {
  hidden: { opacity: 0, y: 24 },
  visible: {
    opacity: 1,
    y: 0,
    transition: { duration: 0.5, ease: "easeOut" as const },
  },
};
