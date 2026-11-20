"use client";

/**
 * The landing page's name for the shared hook. It moved to
 * `src/hooks/use-reduced-motion.ts` (WEB-15 review BLOCKER 1) because the
 * console image excludes this folder and the sign-in page needs it too; the
 * landing sections keep importing it from here unchanged.
 */
export { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
