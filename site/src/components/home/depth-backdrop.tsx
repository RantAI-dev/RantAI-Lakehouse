"use client";

import * as React from "react";
import { depthAt, mixColour, steppedDepthAt, waterAt, type Anchor } from "@/lib/depth-journey";
import { onSceneColours, sceneColours } from "@/lib/scene-colour";
import { smoothstep } from "@/lib/scroll-timeline";
import { measureJourney } from "@/lib/measure-journey";
import { afterPaint } from "@/lib/after-paint";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * WEB-5 D14 S2: one water behind the second half of the page. From the
 * assistant to the final call the sections have no background of their
 * own; this fixed layer is the water they sit in, and its colour follows
 * the depth as the page scrolls, so section edges never show as bands.
 * WEB-5 O33: the page is one descent, so the water only ever darkens, down
 * to the abyss at the final call (the page's own background), where the
 * footer and the overscroll end on the same colour.
 *
 * - Before the assistant the 3D canvas or the scene zone's own gradient
 *   covers it, and its colour there is already the deep water those end on,
 *   so there is no seam where they stop.
 * - WEB-5 O5: over the live lake it is exact. The engine publishes the
 *   colours its water shows at the top and the bottom of the screen
 *   (lib/scene-colour.ts); until the canvas has faded out at the zone's end
 *   this layer shows those, and over the next viewport and a half of
 *   scroll it blends to its own, so the hand-over is never a band or a
 *   jump in colour.
 * - The colour is written to two CSS variables through a ref, from a
 *   passive scroll listener, once per frame and only when it changes. There
 *   is no React state per scroll event (D5).
 * - Reduced motion: the colour steps from chapter to chapter instead of
 *   blending.
 * - Without JavaScript the sections keep their own backgrounds
 *   (`.lk-fallback-bg` in app/globals.css), so the page reads as before.
 *
 * O33 removed S5's ascent: the bubbles rising to the surface and the
 * caustics coming back.
 *
 * WEB-5 O43 (owner: "balikin lah kalo gitu permainan gradient dan partikel
 * partikel airnya"): the water's life is back, as water, not as a climb:
 * marine snow in two layers drifting down at different speeds (parallax),
 * and a few bubbles rising gently at the screen's edges. They show in the
 * water sections only, from where the scene zone ends (whose 3D water has
 * its own snow) to where the final call's night sea begins, fading in and
 * out over half a screen (`--lk-life`). Mounted once the page is usable
 * (`afterPaint`); CSS only (transforms and opacity), with fixed positions
 * and timings; none under reduced motion or without JavaScript.
 */
export function DepthBackdrop() {
  const ref = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();

  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let anchors: Anchor[] = [];
    let frame = 0;
    let last = "";
    // Where the canvas is gone: the scroll at which the scene zone's pinned
    // layer starts to leave the screen.
    let zoneOut = 0;
    // O43: where the final call's night sea begins (the water's life stops there).
    let finalTop = Infinity;
    let lastLife = "";

    // R9: the same measurement as the gauge. No hold through the hero: the
    // water is covered there (see `journeyFromBoxes`).
    const measure = () => {
      ({ anchors } = measureJourney({ holdSurfaceThroughHero: false }));
      const zone = document.getElementById("scene-zone");
      zoneOut = zone ? zone.getBoundingClientRect().bottom + window.scrollY - window.innerHeight : 0;
      const final = document.getElementById("final");
      finalTop = final ? final.getBoundingClientRect().top + window.scrollY : Infinity;
      paint();
    };
    const paint = () => {
      frame = 0;
      // Stepped (reduced motion): a chapter's water applies as soon as its top
      // enters the screen, half a screen before its blended anchor, so no
      // chapter shows in the water of the one above it.
      const depth = reduced
        ? steppedDepthAt(window.scrollY + window.innerHeight / 2, anchors)
        : depthAt(window.scrollY, anchors);
      const own = waterAt(depth);
      const scene = reduced ? null : sceneColours();
      const handOver = scene ? 1 - smoothstep(zoneOut, zoneOut + 1.5 * window.innerHeight, window.scrollY) : 0;
      const water =
        scene && handOver > 0
          ? { top: mixColour(own.top, scene.top, handOver), bottom: mixColour(own.bottom, scene.bottom, handOver) }
          : own;
      const vh = window.innerHeight;
      const y = window.scrollY;
      const life = (
        smoothstep(zoneOut - 0.5 * vh, zoneOut + 0.5 * vh, y) *
        (1 - smoothstep(finalTop - vh, finalTop - 0.4 * vh, y))
      ).toFixed(2);
      if (life !== lastLife) {
        lastLife = life;
        el.style.setProperty("--lk-life", life);
      }
      const key = `${water.top}${water.bottom}`;
      if (key === last) return;
      last = key;
      el.style.setProperty("--lk-water-top", water.top);
      el.style.setProperty("--lk-water-bottom", water.bottom);
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(paint);
    };

    measure();
    const offScene = onSceneColours(onScroll);
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", measure);
    // Fonts, the live hero's longer height and the dashboards rail all move
    // the chapters after load.
    const ro = new ResizeObserver(measure);
    ro.observe(document.body);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      offScene();
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", measure);
      ro.disconnect();
    };
  }, [reduced]);

  const [alive, setAlive] = React.useState(false);
  React.useEffect(() => afterPaint(() => setAlive(true)), []);

  return (
    <div
      ref={ref}
      aria-hidden="true"
      className="lk-water pointer-events-none fixed inset-0 overflow-hidden"
      style={{ "--lk-life": 0 } as React.CSSProperties}
    >
      {alive && !reduced && <WaterLife />}
    </div>
  );
}

/** Marine snow in two layers (far: small and slow; near: larger and quicker), and bubbles at the edges. */
const SNOW = Array.from({ length: 30 }, (_, i) => {
  const near = i % 3 === 0;
  return {
    left: (3 + ((i * 37) % 94)).toFixed(1),
    top: ((i * 29) % 100).toFixed(1),
    size: near ? 2.5 : 1.5,
    alpha: near ? 0.45 : 0.28,
    dur: near ? 16 + ((i * 7) % 6) : 28 + ((i * 11) % 10),
    delay: -((i * 3.7) % 30),
  };
});
const BUBBLES = Array.from({ length: 12 }, (_, i) => ({
  left: i % 2 === 0 ? 2 + ((i * 7) % 11) : 87 + ((i * 5) % 11),
  size: 3 + ((i * 3) % 6),
  dur: 11 + ((i * 7) % 8),
  delay: -((i * 13) % 19),
}));
const LIFE_CSS = `
.lk-life { opacity: var(--lk-life); }
.lk-life-snow { animation: lk-life-snow linear infinite; }
.lk-life-bubble { animation: lk-life-bubble linear infinite; }
@keyframes lk-life-snow {
  0% { transform: translate3d(0, -6vh, 0); }
  100% { transform: translate3d(1.5vw, 106vh, 0); }
}
@keyframes lk-life-bubble {
  0% { transform: translate3d(0, 8vh, 0); opacity: 0; }
  12% { opacity: 0.85; }
  50% { transform: translate3d(5px, -48vh, 0); }
  86% { opacity: 0.6; }
  100% { transform: translate3d(-4px, -104vh, 0); opacity: 0; }
}
`;

function WaterLife() {
  return (
    <div className="lk-life absolute inset-0">
      <style dangerouslySetInnerHTML={{ __html: LIFE_CSS }} />
      {SNOW.map((p, i) => (
        <span
          key={`s${i}`}
          className="lk-life-snow absolute -top-[6vh] rounded-full bg-[#cfe6ff]"
          style={{
            left: `${p.left}%`,
            width: p.size,
            height: p.size,
            opacity: p.alpha,
            animationDuration: `${p.dur}s`,
            animationDelay: `${p.delay}s`,
          }}
        />
      ))}
      {BUBBLES.map((b, i) => (
        <span
          key={`b${i}`}
          className="lk-life-bubble absolute bottom-0 rounded-full border border-[#E8F4FF]/45 bg-[#E8F4FF]/10"
          style={{
            left: `${b.left}%`,
            width: b.size,
            height: b.size,
            animationDuration: `${b.dur}s`,
            animationDelay: `${b.delay}s`,
          }}
        />
      ))}
    </div>
  );
}
