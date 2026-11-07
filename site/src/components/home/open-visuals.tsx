import * as React from "react";
import {
  ArrowUp,
  BarChart3,
  Cloud,
  Database,
  FileCode2,
  GitBranch,
  NotebookPen,
  Plug,
  SlidersHorizontal,
  SquareTerminal,
  Table2,
} from "lucide-react";
import type { HomeCopy } from "@/lib/copy";
import { MascotMark } from "@/components/shared/mascot-svg";

/**
 * WEB-5 O22: the pictures of "Yours to keep", each one showing its promise
 * instead of telling it (owner: "masih kurang suka sama ui ini juga, harus
 * kita explore lagi deh"). All server-rendered; every move is CSS (`.lk-open-*`
 * in app/globals.css) and stops under reduced motion. The one control, the
 * hosting switch, is a pair of radio buttons, so it works with a keyboard
 * and without JavaScript.
 */

type OpenCopy = HomeCopy["open"];

// ---- Open source: the Lakehouse's own code --------------------------------

/**
 * Real lines of the repository, not a sample: `create_write_high_approval`
 * in rust/crates/lakehouse-api/src/routes/ai/gate.rs (lines 275-290 at
 * c99a6e41), the function that holds a risky assistant action for a person
 * (the assistant section's approval card) and refuses it when there is
 * nowhere durable to keep the approval. Its seven parameters are folded, as
 * an editor folds them. Code, so it is not translated.
 */
const CODE_FILE = "routes/ai/gate.rs";
type CodeRow = { n: number; text: string } | { fold: string };
const CODE: readonly CodeRow[] = [
  { n: 275, text: "pub async fn create_write_high_approval(" },
  { fold: "pg, actor, spec, raw_args, redacted_args, resource_id, requested_by" },
  { n: 283, text: ") -> Value {" },
  { n: 284, text: "    let Some(pool) = pg else {" },
  { n: 285, text: "        return json!({" },
  { n: 286, text: '            "error": "refused: approvals are unavailable (no Postgres configured)",' },
  { n: 287, text: '            "refused": true,' },
  { n: 288, text: '            "tool": spec.name,' },
  { n: 289, text: "        });" },
  { n: 290, text: "    };" },
];

const KEYWORDS = new Set(["pub", "async", "fn", "let", "else", "return", "true", "Some"]);

/** A tiny highlighter for the lines above: strings, keywords, types, macros. */
function tokens(line: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  const re = /("[^"]*")|\b([A-Za-z_][A-Za-z0-9_]*)(!?)|([^"A-Za-z_]+)/g;
  let m: RegExpExecArray | null;
  let i = 0;
  while ((m = re.exec(line))) {
    const [all, str, word, bang, other] = m;
    if (str) out.push(<span key={i++} className="lk-tok-str">{str}</span>);
    else if (word && bang) out.push(<span key={i++} className="lk-tok-mac">{word + bang}</span>);
    else if (word && KEYWORDS.has(word)) out.push(<span key={i++} className="lk-tok-kw">{word}</span>);
    else if (word && /^[A-Z]/.test(word)) out.push(<span key={i++} className="lk-tok-ty">{word}</span>);
    else out.push(<React.Fragment key={i++}>{word ?? other ?? all}</React.Fragment>);
  }
  return out;
}

export function CodePanel({ copy, action }: { copy: OpenCopy["code"]; action?: React.ReactNode }) {
  return (
    <div className="lk-open-code relative h-full overflow-hidden rounded-2xl">
      <p className="sr-only">{copy.sr}</p>
      <div aria-hidden="true" className="flex h-9 items-center gap-2 border-b border-[#E8F4FF]/8 px-3.5">
        <FileCode2 className="size-3.5 text-[#E8F4FF]/55" strokeWidth={2} />
        <code className="!text-[12px] !leading-4 text-[#E8F4FF]/75">{CODE_FILE}</code>
        <span className="lk-open-license ml-auto inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] leading-4 font-medium">
          <span className="size-1.5 rounded-full bg-current" />
          {copy.license}
        </span>
      </div>
      <pre aria-hidden="true" className="lk-open-pre relative overflow-hidden py-2.5 text-[12px] leading-[19px]">
        {/* The band that walks down to the lines that refuse. */}
        <span className="lk-open-band pointer-events-none absolute inset-x-0 top-2.5 h-[19px]" />
        {CODE.map((row, i) =>
          "fold" in row ? (
            <span key={i} className="relative flex">
              <span className="w-11 shrink-0 pr-3 text-right text-[#E8F4FF]/30 select-none">⋯</span>
              <span className="lk-open-fold truncate rounded px-1.5 text-[11px] leading-[19px]">{row.fold}</span>
            </span>
          ) : (
            <span key={i} className="relative flex">
              <span className="w-11 shrink-0 pr-3 text-right text-[#E8F4FF]/30 tabular-nums select-none">{row.n}</span>
              <code className="!text-[12px] !leading-[19px] whitespace-pre text-[#E8F4FF]/80">{tokens(row.text)}</code>
            </span>
          ),
        )}
      </pre>
      {action && <div className="absolute right-3 bottom-3">{action}</div>}
    </div>
  );
}

// ---- Open tables: one Iceberg table, several readers ----------------------

/**
 * Readers placed around the table, in percent of the diagram. ClickHouse is
 * the engine the Lakehouse itself reads Iceberg with (BLD-3: Bronze is read
 * through a `DataLakeCatalog` database); the other two are generic, because
 * the claim is "any engine that reads Iceberg" and no other engine is named
 * anywhere the backlog backs. No logos: there is no licensed mark in the
 * site for any of them (DESIGN.md: a neutral icon, never a fake logo).
 */
const READERS = [
  { id: "clickhouse", x: 17, y: 24, nx: 50, ny: 14, icon: Database },
  { id: "engine", x: 17, y: 76, nx: 27, ny: 86, icon: SquareTerminal },
  { id: "notebook", x: 84, y: 50, nx: 73, ny: 86, icon: NotebookPen },
] as const;

/** The wires, in the wide layout (readers left and right) or the narrow one (one above, two below). */
function Wires({ narrow }: { narrow?: boolean }) {
  return (
    <svg
      className={`absolute inset-0 size-full ${narrow ? "lk-open-wires-narrow" : "lk-open-wires-wide"}`}
      viewBox="0 0 100 100"
      preserveAspectRatio="none"
    >
      {READERS.map((r) => (
        <g key={r.id} className="lk-open-wire" data-for={r.id}>
          <line x1="50" y1="50" x2={narrow ? r.nx : r.x} y2={narrow ? r.ny : r.y} className="lk-open-wire-base" vectorEffect="non-scaling-stroke" />
          <line x1="50" y1="50" x2={narrow ? r.nx : r.x} y2={narrow ? r.ny : r.y} className="lk-open-wire-flow" vectorEffect="non-scaling-stroke" />
        </g>
      ))}
    </svg>
  );
}

export function TablesDiagram({ copy }: { copy: OpenCopy["tables"] }) {
  const label = { clickhouse: "ClickHouse", engine: copy.engine, notebook: copy.notebook } as const;
  return (
    <div className="lk-open-tables relative h-full min-h-[200px] overflow-hidden rounded-2xl">
      <p className="sr-only">{copy.sr}</p>
      <div aria-hidden="true" className="absolute inset-0">
        <Wires />
        <Wires narrow />
        {/* The table in the middle: one Iceberg table, its snapshots stacked behind it. */}
        <div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2">
          <div className="lk-open-table relative rounded-xl px-3.5 py-2.5">
            <span className="lk-open-table-back absolute inset-0 rounded-xl" />
            <span className="relative flex items-center gap-2">
              <Table2 className="size-4 text-[#8fcbff]" strokeWidth={2} />
              <code className="!text-[13px] !leading-5 font-medium text-[#E8F4FF]">{copy.table}</code>
            </span>
            <span className="relative mt-0.5 block text-[11px] leading-4 text-[#E8F4FF]/60">{copy.format}</span>
          </div>
        </div>
        {READERS.map((r) => {
          const Icon = r.icon;
          return (
            <span
              key={r.id}
              className="lk-open-reader absolute inline-flex -translate-x-1/2 -translate-y-1/2 items-center gap-1.5 rounded-full px-2.5 py-1 text-[12px] leading-4 whitespace-nowrap"
              data-for={r.id}
              style={{ "--x": `${r.x}%`, "--y": `${r.y}%`, "--nx": `${r.nx}%`, "--ny": `${r.ny}%` } as React.CSSProperties}
            >
              <Icon className="size-3.5 shrink-0 text-[#8fcbff]" strokeWidth={2} />
              <span className="text-[#E8F4FF]/90">{label[r.id]}</span>
              {r.id === "clickhouse" && <span className="text-[#E8F4FF]/50">· {copy.builtIn}</span>}
            </span>
          );
        })}
      </div>
    </div>
  );
}

// ---- Your servers or ours: the same stack, two homes ----------------------

/**
 * A two-way switch built from radio buttons: no script, a real keyboard
 * control (arrow keys move between the two), and the picture follows
 * through `:has(:checked)` in CSS. The stack slides from a rack to a cloud.
 * "Run for you" says only what the copy claims: talk to us about it.
 */
export function HostingSwitch({ copy }: { copy: OpenCopy["hosting"] }) {
  return (
    <div className="lk-open-host flex h-full flex-col gap-3 p-4">
      <fieldset className="lk-open-seg relative inline-grid w-full grid-cols-2 rounded-xl p-1 text-[13px] leading-5 font-medium">
        <legend className="sr-only">{copy.label}</legend>
        <span aria-hidden="true" className="lk-open-seg-thumb absolute inset-y-1 left-1 w-[calc(50%-4px)] rounded-lg" />
        <label className="lk-open-seg-opt relative z-10 cursor-pointer rounded-lg px-3 py-1.5 text-center">
          <input type="radio" name="lk-host" value="yours" defaultChecked className="sr-only" />
          {copy.yours}
        </label>
        <label className="lk-open-seg-opt relative z-10 cursor-pointer rounded-lg px-3 py-1.5 text-center">
          <input type="radio" name="lk-host" value="ours" className="sr-only" />
          {copy.ours}
        </label>
      </fieldset>
      <div aria-hidden="true" className="lk-open-stage relative min-h-[150px] flex-1 overflow-hidden rounded-2xl">
        {/* Your servers: a rack whose lights blink. */}
        <span className="lk-open-rack absolute bottom-7 left-4 flex w-[42%] max-w-[220px] flex-col gap-1.5">
          {[0, 1, 2].map((i) => (
            <span key={i} className="lk-open-unit flex h-5 items-center gap-1.5 rounded-md px-2">
              <span className="lk-open-led size-1 rounded-full" style={{ animationDelay: `${i * 0.7}s` }} />
              <span className="lk-open-led size-1 rounded-full" style={{ animationDelay: `${i * 0.7 + 0.35}s` }} />
              <span className="ml-auto h-px w-1/3 bg-[#E8F4FF]/15" />
            </span>
          ))}
        </span>
        {/* Ours: a cloud drifting. */}
        <span className="lk-open-cloud absolute top-4 right-3 grid w-[44%] max-w-[170px] place-items-center">
          <Cloud className="h-auto w-full text-[#8fcbff]" strokeWidth={1} />
        </span>
        {/* The stack itself: the same Lakehouse in either home. */}
        <span className="lk-open-stack absolute left-4 flex w-[42%] max-w-[220px] flex-col items-center gap-1 rounded-xl px-2 py-2">
          <MascotMark className="h-4 w-auto" />
          <span className="flex w-full flex-col gap-0.5">
            <span className="h-1 rounded-full bg-[#C8834A]" />
            <span className="h-1 rounded-full bg-[#B9C6D3]" />
            <span className="h-1 rounded-full bg-[#F2C14E]" />
          </span>
        </span>
        <span className="lk-open-note absolute inset-x-0 bottom-1.5 text-center text-[11px] leading-4 text-[#E8F4FF]/65">
          <span className="lk-open-note-yours">{copy.yoursNote}</span>
          <span className="lk-open-note-ours">{copy.oursNote}</span>
        </span>
      </div>
    </div>
  );
}

// ---- The console's home, recreated ----------------------------------------

/**
 * The console's own strings, word for word (they are its English UI, so
 * they are not translated): the examples its Home placeholder cycles
 * through (src/lib/home-prompts.ts, Ask mode), the Ask / Build / Tools
 * controls and the "Tab" hint (src/features/copilot/chat-composer.tsx), and
 * the three default shortcuts (src/features/home/home-shortcuts.ts). The
 * console puts a sparkle button beside each shortcut that hands it to the
 * assistant; the recreation leaves it out (O22 follow-up: DESIGN.md lists
 * sparkles among the signs of "AI slop", and the page may simplify the
 * console), and keeps each shortcut's hover.
 *
 * In the console's dark theme rather than the light one of the old crop: the
 * console ships both, and the dark one sits on this page without a bright
 * slab, which is what the owner disliked in the crop. O38: the showcase
 * panel is its window, with the title strip drawn here.
 */
const PROMPTS = [
  "Which tables changed in the last day?",
  "What data do we have?",
  "Row counts for every gold mart",
  "Summarize yesterday's pipeline runs",
];
const SHORTCUTS = [
  { title: "Connect a source", icon: Plug },
  { title: "Create a pipeline", icon: GitBranch },
  { title: "Build a dashboard", icon: BarChart3 },
] as const;

export function ConsoleHome({ windowTitle }: { windowTitle: string }) {
  return (
    <div className="lk-con flex h-full flex-col">
      {/* The window's title strip: the mark and what it shows. No traffic lights, no address bar. */}
      <div className="flex items-center gap-2 px-5 pt-4 pb-1">
        <MascotMark className="h-3 w-auto" />
        <span className="text-[13px] leading-5 font-medium text-[#E8F4FF]/75">{windowTitle}</span>
      </div>
      <div className="flex flex-1 flex-col justify-center p-4 sm:p-6">
      <div className="lk-con-box rounded-2xl px-4 pt-3.5 pb-2.5">
        <p className="relative h-6 text-[15px] leading-6">
          <span className="lk-con-caret absolute top-0.5 left-0 h-5 w-px bg-[#8fcbff]" />
          {PROMPTS.map((p, i) => (
            <span
              key={p}
              className="lk-con-prompt absolute inset-y-0 left-1.5 flex items-center gap-2 whitespace-nowrap"
              data-first={i === 0 || undefined}
              style={{ "--i": i } as React.CSSProperties}
            >
              <span className="lk-con-text text-[#E8F4FF]/55">{p}</span>
              <span className="lk-con-tab rounded border border-[#E8F4FF]/15 px-1 text-[10px] leading-4 text-[#E8F4FF]/45 max-sm:hidden">Tab</span>
            </span>
          ))}
        </p>
        <div className="mt-5 flex items-center gap-3 text-[12px] leading-4 font-medium">
          <span className="inline-flex rounded-lg bg-[#E8F4FF]/[0.06] p-0.5">
            <span className="rounded-md bg-[#E8F4FF]/14 px-2.5 py-1 text-[#E8F4FF]">Ask</span>
            <span className="px-2.5 py-1 text-[#E8F4FF]/60">Build</span>
          </span>
          <span className="inline-flex items-center gap-1.5 text-[#E8F4FF]/60">
            <SlidersHorizontal className="size-3.5" strokeWidth={2} />
            Tools
          </span>
          <span className="ml-auto grid size-8 place-items-center rounded-lg bg-[#E8F4FF]/10 text-[#E8F4FF]/55">
            <ArrowUp className="size-4" strokeWidth={2.25} />
          </span>
        </div>
      </div>
      <div className="mt-2.5 grid gap-2 sm:grid-cols-3">
        {SHORTCUTS.map((s) => {
          const Icon = s.icon;
          return (
            <span key={s.title} className="lk-con-short flex items-stretch overflow-hidden rounded-xl">
              <span className="flex min-w-0 flex-1 items-center gap-2 px-3 py-2.5 text-[13px] leading-5 font-medium text-[#E8F4FF]/90">
                <Icon className="lk-con-short-icon size-4 shrink-0 text-[#E8F4FF]/60" strokeWidth={2} />
                <span className="truncate">{s.title}</span>
              </span>
            </span>
          );
        })}
      </div>
      </div>
    </div>
  );
}
