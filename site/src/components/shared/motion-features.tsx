"use client";

import dynamic from "next/dynamic";
import { LazyMotion } from "framer-motion";
import { QuietBoundary } from "./quiet-boundary";
import type { domAnimation } from "framer-motion";

/**
 * WEB-5 D5: the page's animations use `m.*` components with only the
 * `domAnimation` features (animate, exit, variants, hover, in-view),
 * instead of `motion.*`, which bundles every feature (layout, drag) into
 * the first JavaScript. `strict` makes a stray `motion.*` an error at
 * runtime rather than a silent return of the full bundle.
 *
 * WEB-5 review fix R1 (BLOCKER): the features arrive after hydration.
 * motion's async form, `features={() => import(...)}`, was measured first:
 * Turbopack still writes a bare `import()` target into the HTML as an
 * `async` script, so the chunk stayed in the first load (210.1 KB). A
 * `next/dynamic` component with `ssr: false` is not written into the HTML
 * (the 3D scene loads the same way), so the features come through
 * `DomAnimationLoader`, which resolves the promise `LazyMotion` waits on.
 *
 * Until then every `m.*` element shows what the server rendered. Reveals
 * start hidden only once the features are in: `markReady` puts
 * `motion-ready` on <html>, and `app/globals.css` keeps every
 * `[data-reveal]` visible without it. If the chunk never loads, nothing
 * stays at opacity 0, and `QuietBoundary` keeps its load error from taking
 * the page down (both measured with the chunk blocked). The hero's h1 and calls to action use no reveal and
 * no feature, so the largest paint never waits on this chunk.
 */
type Features = typeof domAnimation;

let resolveFeatures: (features: Features) => void = () => {};
const features = new Promise<Features>((resolve) => {
  resolveFeatures = resolve;
});
const loadFeatures = () => features;

function markReady(loaded: Features): void {
  resolveFeatures(loaded);
  document.documentElement.classList.add("motion-ready");
}

const DomAnimationLoader = dynamic(() => import("./dom-animation-loader"), { ssr: false });

export function MotionFeatures({ children }: { children: React.ReactNode }) {
  return (
    <LazyMotion features={loadFeatures} strict>
      {children}
      <QuietBoundary>
        <DomAnimationLoader onLoad={markReady} />
      </QuietBoundary>
    </LazyMotion>
  );
}
