"use client";

import * as React from "react";
import { domAnimation } from "framer-motion";

/**
 * Hands motion's `domAnimation` features to `MotionFeatures` once mounted.
 * Its own module so `next/dynamic` can load it after hydration (WEB-5 review
 * fix R1): this file is the only one that imports `domAnimation`, so the
 * feature code lives in its chunk and not in the first JavaScript.
 */
export default function DomAnimationLoader({ onLoad }: { onLoad: (features: typeof domAnimation) => void }) {
  React.useEffect(() => onLoad(domAnimation), [onLoad]);
  return null;
}
