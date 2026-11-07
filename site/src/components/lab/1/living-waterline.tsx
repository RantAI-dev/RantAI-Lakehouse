"use client";

import * as React from "react";

/**
 * Lab 1's waterline: one SVG path across the hero, the water below it
 * filled. It swells gently on its own, bulges towards a mouse (up when the
 * pointer is in the sky, down when it is in the water, less the farther
 * away it is) and rings out from a press, on any pointer. It is redrawn
 * once a frame only while on screen; under reduced motion it is drawn once,
 * still, and does not react.
 *
 * The whale (children: its top half and its refracted bottom half) rides
 * the line at three quarters across, tilted with the wave's slope.
 */
const POINTS = 72;

type Ripple = { x: number; t: number };

export function LivingWaterline({ children }: Readonly<{ children: React.ReactNode }>) {
  const svgRef = React.useRef<SVGSVGElement>(null);
  const waterRef = React.useRef<SVGPathElement>(null);
  const foamRef = React.useRef<SVGPathElement>(null);
  const frontRef = React.useRef<SVGPathElement>(null);
  const frontSvgRef = React.useRef<SVGSVGElement>(null);
  const whaleRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    const svg = svgRef.current;
    const section = svg?.closest("section");
    if (!svg || !section) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let w = 0;
    let h = 0;
    let base = 0;
    let whaleW = 0;
    const size = () => {
      const r = section.getBoundingClientRect();
      w = r.width;
      h = r.height;
      base = h * (w < 768 ? 0.4 : 0.45);
      whaleW = whaleRef.current?.offsetWidth ?? 0;
      svg.setAttribute("viewBox", `0 0 ${w} ${h}`);
      frontSvgRef.current?.setAttribute("viewBox", `0 0 ${w} ${h}`);
    };
    size();

    let px = -1;
    let py = -1;
    let bulge = 0;
    let bulgeX = w / 2;
    const ripples: Ripple[] = [];
    const t0 = performance.now();

    const yAt = (x: number, t: number) => {
      let y = base + 6 * Math.sin(x * 0.008 + t * 0.9) + 4 * Math.sin(x * 0.017 - t * 1.3) + 2.2 * Math.sin(x * 0.031 + t * 2.1);
      const dx = x - bulgeX;
      y += bulge * Math.exp(-(dx * dx) / (2 * 130 * 130));
      for (const r of ripples) {
        const a = t - r.t;
        const d = Math.abs(x - r.x);
        if (d > 300 * a + 40) continue;
        y += 13 * Math.exp(-1.5 * a) * Math.exp(-d / 420) * Math.sin(d * 0.05 - a * 9);
      }
      return y;
    };

    const draw = (t: number) => {
      // Ease the bulge towards the pointer.
      let target = 0;
      if (px >= 0) {
        const dy = py - base;
        target = Math.max(-38, Math.min(38, dy * 0.32)) * Math.exp(-Math.abs(dy) / 340);
        bulgeX += (px - bulgeX) * 0.12;
      }
      bulge += (target - bulge) * 0.08;
      while (ripples.length && t - ripples[0].t > 3.2) ripples.shift();

      const pts: [number, number][] = [];
      for (let i = 0; i <= POINTS; i++) {
        const x = (i / POINTS) * w;
        pts.push([x, yAt(x, t)]);
      }
      let d = `M0 ${pts[0][1].toFixed(1)}`;
      for (let i = 1; i < pts.length; i++) {
        const [x0, y0] = pts[i - 1];
        const [x1, y1] = pts[i];
        d += ` Q${x0.toFixed(1)} ${y0.toFixed(1)} ${((x0 + x1) / 2).toFixed(1)} ${((y0 + y1) / 2).toFixed(1)}`;
      }
      d += ` L${w} ${pts[pts.length - 1][1].toFixed(1)}`;
      foamRef.current?.setAttribute("d", d);
      waterRef.current?.setAttribute("d", `${d} L${w} ${h} L0 ${h} Z`);
      frontRef.current?.setAttribute("d", `${d} L${w} ${h} L0 ${h} Z`);

      const wx = w * (w < 768 ? 0.7 : 0.76);
      const wy = yAt(wx, t);
      const slope = (yAt(wx + 20, t) - yAt(wx - 20, t)) / 40;
      // The drawing's waterline is at 318 of 480 units, its width 600.
      whaleRef.current?.style.setProperty(
        "transform",
        `translate(${(wx - whaleW / 2).toFixed(1)}px, ${(wy - whaleW * (318 / 600)).toFixed(1)}px) rotate(${(Math.atan(slope) * 57.3 * 0.8).toFixed(2)}deg)`,
      );
    };

    if (reduce) {
      draw(0);
      const onResize = () => {
        size();
        draw(0);
      };
      window.addEventListener("resize", onResize);
      return () => window.removeEventListener("resize", onResize);
    }

    let frame = 0;
    let visible = true;
    const loop = (now: number) => {
      draw((now - t0) / 1000);
      frame = visible ? requestAnimationFrame(loop) : 0;
    };
    const io = new IntersectionObserver(([e]) => {
      visible = e.isIntersecting;
      if (visible && !frame) frame = requestAnimationFrame(loop);
    });
    io.observe(section);

    const local = (e: PointerEvent) => {
      const r = section.getBoundingClientRect();
      return [e.clientX - r.left, e.clientY - r.top] as const;
    };
    const move = (e: PointerEvent) => {
      if (e.pointerType !== "mouse") return;
      [px, py] = local(e);
    };
    const leave = () => {
      px = -1;
    };
    const press = (e: PointerEvent) => {
      if ((e.target as Element).closest("a, button")) return;
      const [x] = local(e);
      ripples.push({ x, t: (performance.now() - t0) / 1000 });
    };
    section.addEventListener("pointermove", move);
    section.addEventListener("pointerleave", leave);
    section.addEventListener("pointerdown", press);
    window.addEventListener("resize", size);
    return () => {
      cancelAnimationFrame(frame);
      io.disconnect();
      section.removeEventListener("pointermove", move);
      section.removeEventListener("pointerleave", leave);
      section.removeEventListener("pointerdown", press);
      window.removeEventListener("resize", size);
    };
  }, []);

  return (
    <>
      <svg ref={svgRef} aria-hidden="true" className="pointer-events-none absolute inset-0 size-full" preserveAspectRatio="none">
        <defs>
          {/* The water bends what is under it: a slow, slight displacement. */}
          <filter id="l1-refract" x="-5%" y="-5%" width="110%" height="110%">
            <feTurbulence type="fractalNoise" baseFrequency="0.012 0.05" numOctaves="1" seed="3" result="n">
              <animate attributeName="baseFrequency" dur="9s" values="0.012 0.05;0.014 0.06;0.012 0.05" repeatCount="indefinite" />
            </feTurbulence>
            <feDisplacementMap in="SourceGraphic" in2="n" scale="9" xChannelSelector="R" yChannelSelector="G" />
          </filter>
        </defs>
        <path ref={waterRef} fill="#030a2c" />
      </svg>
      <div ref={whaleRef} aria-hidden="true" className="l1-whale pointer-events-none absolute top-0 left-0 z-10">
        {children}
      </div>
      {/* The water in front of the whale: what is under the line is seen through it. */}
      <svg ref={frontSvgRef} aria-hidden="true" className="pointer-events-none absolute inset-0 z-[11] size-full" preserveAspectRatio="none">
        <path ref={frontRef} fill="#030a2c" fillOpacity="0.6" />
        <path ref={foamRef} fill="none" stroke="#9fd0ff" strokeOpacity="0.6" strokeWidth="1.25" vectorEffect="non-scaling-stroke" />
      </svg>
    </>
  );
}
