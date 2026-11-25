"use client";

import * as React from "react";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import { ENGINES, OneTableView, ROUND, type Copy } from "./open-tables-view";

/**
 * "Open tables" picture, "one table, many readers" (owner pick, 2026-11-24,
 * WEB-5; the earlier anatomy graphic left him unsatisfied): the Iceberg
 * table in the middle, four engines around it, each sending the same query
 * and getting the same answer, with the table's files in your object
 * storage underneath. Pointing at an engine holds the loop on it.
 *
 * Engine marks are Simple Icons (CC0, shared/brand-icons.tsx): lake blue,
 * the brand's own on hover. Only ClickHouse is claimed as built in; the
 * others are examples of engines that read Iceberg. The loop runs only
 * while the picture is on screen (and, from 1024 px, while it is the
 * showcase's current one). Under reduced motion, and in the server markup,
 * every engine shows its answer: the settled picture. Flat and borderless;
 * monospace only inside <code>. Styles: `open-tables-readers.css`.
 */


const SHOWCASE = "(min-width: 64rem) and (min-height: 44rem)";

/** True while the picture is on screen and, in the desktop showcase, the panel shown; never under reduced motion. */
function useLive(ref: React.RefObject<HTMLElement | null>): boolean {
  const reduced = useReducedMotionSafe();
  const [live, setLive] = React.useState(false);
  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const panel = el.closest<HTMLElement>(".lk-show-visual");
    const mq = window.matchMedia(SHOWCASE);
    let inView = false;
    const update = () => setLive(inView && (!panel || !mq.matches || panel.hasAttribute("data-on")));
    const io = new IntersectionObserver(
      ([e]) => {
        inView = e.isIntersecting;
        update();
      },
      { threshold: 0.35 },
    );
    io.observe(el);
    const mo = new MutationObserver(update);
    if (panel) mo.observe(panel, { attributes: true, attributeFilter: ["data-on"] });
    mq.addEventListener("change", update);
    return () => {
      io.disconnect();
      mo.disconnect();
      mq.removeEventListener("change", update);
    };
  }, [ref]);
  return live && !reduced;
}

/** A step counter that advances every `ms` while `run` holds. */
function useStep(run: boolean, ms: number): number {
  const [step, setStep] = React.useState(0);
  React.useEffect(() => {
    if (!run) return;
    const id = window.setInterval(() => setStep((s) => s + 1), ms);
    return () => window.clearInterval(id);
  }, [run, ms]);
  return step;
}

export default function OneTableManyReadersLive({ copy }: { copy: Copy }) {
  const ref = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();
  const live = useLive(ref);
  // Without JavaScript and under reduced motion every answer shows: the settled picture. Once JS
  // runs and the graphic is off screen, it is armed: the answers wait hidden and reveal as the loop
  // asks. If it is already on screen at mount (a page loaded scrolled here) it is never armed, so
  // nothing visible disappears and the loop just goes on from the full picture.
  const [armed, setArmed] = React.useState(false);
  React.useEffect(() => {
    const el = ref.current;
    const r = el?.getBoundingClientRect();
    // Same rule as `useLive`: in the desktop showcase only the current panel is visible.
    const panel = el?.closest<HTMLElement>(".lk-show-visual");
    const shown = !panel || !window.matchMedia(SHOWCASE).matches || panel.hasAttribute("data-on");
    if (shown && r && r.bottom > 0 && r.top < window.innerHeight && r.right > 0 && r.left < window.innerWidth) return;
    setArmed(true);
  }, []);
  const [hover, setHover] = React.useState<number | null>(null);
  const step = useStep(live && hover === null, 900);
  const t = step - 1; // -1 until the first beat
  const inRound = t >= 0 ? t % ROUND : -1;
  const auto = inRound >= 0 && inRound < ENGINES.length * 2 ? Math.floor(inRound / 2) : -1;
  const phase = inRound >= 0 && inRound < ENGINES.length * 2 ? (inRound % 2 === 0 ? "q" : "a") : null;
  const active = hover ?? auto;
  const answered = (i: number) => reduced || hover === i || t >= 2 * i + 1;
  const together = reduced || (t >= 0 && inRound >= ENGINES.length * 2);
  const reading = hover !== null || phase === "q";

  return (
    <OneTableView
      copy={copy}
      rootRef={ref}
      onHover={setHover}
      state={{ armed: armed && !reduced, together, active, phase, reading, hover, step, answered }}
    />
  );
}
