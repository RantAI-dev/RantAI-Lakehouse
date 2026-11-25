"use client";

import * as React from "react";
import { Check, HardDrive, Table2 } from "lucide-react";
import { hoverColour, SourceIcon, type BrandId } from "@/components/shared/brand-icons";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import type { HomeCopy } from "@/lib/copy";
import "./open-tables-readers.css";

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


type Copy = HomeCopy["open"]["tables"];

const ENGINES: readonly { id: BrandId; name: string; builtIn: boolean }[] = [
  { id: "clickhouse", name: "ClickHouse", builtIn: true },
  { id: "duckdb", name: "DuckDB", builtIn: false },
  { id: "apachespark", name: "Apache Spark", builtIn: false },
  { id: "trino", name: "Trino", builtIn: false },
];
const COLUMNS = [
  ["order_id", "int"],
  ["city", "string"],
  ["amount", "decimal"],
  ["ts", "timestamp"],
] as const;


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

function Mark({ id, className }: { id: BrandId; className?: string }) {
  return (
    <span className={`lk-otr-mark ${className ?? ""}`} style={{ "--brand": hoverColour(id) } as React.CSSProperties}>
      <SourceIcon id={id} className="size-full" />
    </span>
  );
}

/* ---- A: one table, many readers -------------------------------------------------------------- */

const SIDES = ["tl", "tr", "bl", "br"] as const;
// One round: each engine sends (q) and gets its answer back (a), then two beats where all four
// answers light together: the same table gave the same answer.
const ROUND = ENGINES.length * 2 + 2;

export function OneTableManyReaders({ copy }: { copy: Copy }) {
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
    <div ref={ref} className="lk-otr lk-otr-a" data-armed={armed && !reduced ? "" : undefined} data-together={together ? "" : undefined}>
      <p className="sr-only">{copy.sr}</p>
      <div aria-hidden="true" className="lk-otr-a-body">
        <div className="lk-otr-a-main">
        <code className="lk-otr-code lk-otr-a-sql">SELECT sum(amount) FROM orders</code>
        <div className="lk-otr-a-grid">
          {ENGINES.map((e, i) => (
            <div
              key={e.id}
              className="lk-otr-a-eng"
              data-side={SIDES[i]}
              data-active={active === i ? "" : undefined}
              onMouseEnter={() => setHover(i)}
              onMouseLeave={() => setHover(null)}
            >
              <div className="lk-otr-a-node">
                <span className="lk-otr-a-who">
                  <Mark id={e.id} className="size-5" />
                  <span className="lk-otr-a-name">{e.name}</span>
                </span>
                {e.builtIn && <span className="lk-otr-badge">{copy.builtIn}</span>}
                <span
                  className="lk-otr-a-ans"
                  data-on={answered(i) ? "" : undefined}
                  data-flash={hover === null && active === i && phase === "a" ? "" : undefined}
                >
                  Rp 1.33M
                </span>
              </div>
              <span className="lk-otr-a-line">
                {active === i && phase && <span key={step} className="lk-otr-a-dot" data-phase={phase} />}
              </span>
            </div>
          ))}
          <div className="lk-otr-a-card" data-reading={reading ? "" : undefined}>
            <p className="lk-otr-a-head">
              <Table2 className="size-4 shrink-0 text-[#8fcbff]" strokeWidth={2} />
              <code className="lk-otr-code lk-otr-a-table">{copy.table}</code>
              <span className="lk-otr-badge lk-otr-badge-blue">{copy.format}</span>
            </p>
            <ul className="lk-otr-a-cols">
              {COLUMNS.map(([name, type]) => (
                <li key={name} data-hot={name === "amount" ? "" : undefined}>
                  <code className="lk-otr-code">{name}</code>
                  <code className="lk-otr-code lk-otr-type">{type}</code>
                </li>
              ))}
            </ul>
          </div>
        </div>
        <p className="lk-otr-a-caption">
          <Check className="size-3.5" strokeWidth={2.5} />
          {copy.sameAnswer}
        </p>
        </div>
        <div className="lk-otr-a-store" data-reading={reading ? "" : undefined}>
          <span className="lk-otr-label lk-otr-a-storelabel">
            <HardDrive className="size-3.5" strokeWidth={2} />
            {copy.storage}
          </span>
          <ul className="lk-otr-a-files">
            {["metadata/v3.metadata.json", "data/00001.parquet", "data/00002.parquet", "data/00003.parquet"].map((f, i) => (
              <li key={f} style={{ "--i": i } as React.CSSProperties}>
                <code className="lk-otr-code">{f}</code>
              </li>
            ))}
          </ul>
        </div>
      </div>
    </div>
  );
}
