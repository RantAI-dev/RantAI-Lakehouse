import * as React from "react";
import { Check, HardDrive, Table2 } from "lucide-react";
import { hoverColour, SourceIcon, type BrandId } from "@/components/shared/brand-icons";
import type { HomeCopy } from "@/lib/copy";
import "./open-tables-readers.css";

/**
 * The Open tables picture's markup (WEB-5 T3). It has no state and no
 * hooks: the server renders it as the settled picture the page ships
 * (`open-tables.tsx`), and the live version (`open-tables-readers.tsx`,
 * loaded after the first paint) renders the same elements and adds the loop
 * and the pointer.
 */

export type Copy = HomeCopy["open"]["tables"];

export const ENGINES: readonly { id: BrandId; name: string; builtIn: boolean }[] = [
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
export const ROUND = ENGINES.length * 2 + 2;


export type OneTableState = {
  armed: boolean;
  together: boolean;
  active: number;
  phase: "q" | "a" | null;
  reading: boolean;
  hover: number | null;
  step: number;
  answered: (i: number) => boolean;
};

export const SETTLED: OneTableState = {
  armed: false,
  together: false,
  active: -1,
  phase: null,
  reading: false,
  hover: null,
  step: 0,
  answered: () => false,
};

export function OneTableView({
  copy,
  state,
  rootRef,
  onHover,
}: Readonly<{ copy: Copy; state: OneTableState; rootRef?: React.Ref<HTMLDivElement>; onHover?: (i: number | null) => void }>) {
  const { armed, together, active, phase, reading, hover, step, answered } = state;
  return (
    <div ref={rootRef} className="lk-otr lk-otr-a" data-armed={armed ? "" : undefined} data-together={together ? "" : undefined}>
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
              onMouseEnter={onHover && (() => onHover(i))}
              onMouseLeave={onHover && (() => onHover(null))}
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
