import * as React from "react";
import {
  ArrowUp,
  BarChart3,
  Check,
  FileCode2,
  GitBranch,
  MousePointer2,
  Plug,
  SlidersHorizontal,
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
        <span className="lk-open-license ml-auto inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] leading-4 font-medium max-lg:text-[12px]">
          {copy.license}
        </span>
      </div>
      {/* WEB-5 M4 (phone pass): below 1024 px the gutter is narrower and every line fits the panel;
          the one long line (the refusal's message) ends in an ellipsis, as an editor shows a line
          wider than its pane, instead of being cut by the panel's edge. */}
      <pre aria-hidden="true" className="lk-open-pre relative overflow-hidden py-2.5 text-[12px] leading-[19px]">
        {/* The band that walks down to the lines that refuse. */}
        <span className="lk-open-band pointer-events-none absolute inset-x-0 top-2.5 h-[19px]" />
        {CODE.map((row, i) =>
          "fold" in row ? (
            <span key={i} className="relative flex">
              <span className="w-11 shrink-0 pr-3 text-right text-[#E8F4FF]/30 select-none max-lg:w-9 max-lg:pr-2.5">⋯</span>
              <span className="lk-open-fold truncate rounded px-1.5 text-[11px] leading-[19px] max-lg:text-[12px]">{row.fold}</span>
            </span>
          ) : (
            <span key={i} className="relative flex">
              <span data-n={row.n} className="lk-open-ln w-11 shrink-0 pr-3 text-right text-[#E8F4FF]/30 tabular-nums select-none max-lg:w-9 max-lg:pr-2.5" />
              <code className="!text-[12px] !leading-[19px] whitespace-pre text-[#E8F4FF]/80 max-lg:min-w-0 max-lg:overflow-hidden max-lg:pr-3 max-lg:text-ellipsis">{tokens(row.text)}</code>
            </span>
          ),
        )}
      </pre>
      {action && <div className="absolute right-3 bottom-3">{action}</div>}
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
 *
 * Owner QA 2026-11-08 ("the animation still feels too standard"): it is a visitor using the
 * console now. `console-play.tsx` loads `console-script.ts` when the panel is shown; it types a
 * question, completes it with Tab, switches Ask to Build, moves a pointer to a shortcut, presses it
 * and morphs the composer into a small result (a dashboard assembling, or sources connecting),
 * then loops with another question. Hovering the panel pauses it and a click on a shortcut plays
 * its result at once. The markup below is the settled console: what reduced motion and no
 * JavaScript show, and what the script starts from.
 */
import { CONSOLE_PROMPTS } from "./console-prompts";

const SHORTCUTS = [
  { id: "src", title: "Connect a source", icon: Plug },
  { id: "pipe", title: "Create a pipeline", icon: GitBranch },
  { id: "dash", title: "Build a dashboard", icon: BarChart3 },
] as const;
const SOURCES = ["PostgreSQL", "MySQL", "HubSpot"] as const;

export function ConsoleHome({ windowTitle }: { windowTitle: string }) {
  return (
    <div className="lk-con @container relative flex h-full flex-col" data-mode="ask">
      {/* The window's title strip: the mark and what it shows. No traffic lights, no address bar. */}
      <div className="flex items-center gap-2 px-5 pt-4 pb-1">
        <MascotMark className="h-3 w-auto" />
        <span className="text-[13px] leading-5 font-medium text-[#E8F4FF]/75">{windowTitle}</span>
      </div>
      <div className="flex flex-1 flex-col justify-center p-4 sm:p-6">
        <div className="lk-con-stage relative">
          <div className="lk-con-main">
            <div className="lk-con-box rounded-2xl px-4 pt-3.5 pb-2.5">
              <p className="relative h-6 text-[15px] leading-6" aria-hidden="true">
                <span className="lk-con-caret absolute top-0.5 left-0 h-5 w-px bg-[#8fcbff]" />
                <span className="absolute inset-y-0 left-1.5 flex items-center gap-2 whitespace-nowrap">
                  <span>
                    <span data-typed className="text-[#E8F4FF]" />
                    <span data-ghost className="text-[#E8F4FF]/50">{CONSOLE_PROMPTS.ask}</span>
                  </span>
                  <span className="lk-con-tab rounded bg-[#E8F4FF]/[0.08] px-1.5 text-[10px] leading-4 text-[#E8F4FF]/55 max-sm:hidden">Tab</span>
                </span>
              </p>
              <div className="mt-5 flex items-center gap-3 text-[12px] leading-4 font-medium">
                <span className="lk-con-seg relative inline-flex rounded-lg bg-[#E8F4FF]/[0.06] p-0.5" aria-hidden="true">
                  <span className="lk-con-thumb absolute inset-y-0.5 left-0.5 w-[calc(50%-2px)] rounded-md bg-[#E8F4FF]/14" />
                  <span className="lk-con-opt relative px-2.5 py-1" data-m="ask">Ask</span>
                  <span className="lk-con-opt relative px-2.5 py-1" data-m="build">Build</span>
                </span>
                <span className="inline-flex items-center gap-1.5 text-[#E8F4FF]/60" aria-hidden="true">
                  <SlidersHorizontal className="size-3.5" strokeWidth={2} />
                  Tools
                </span>
                <span className="ml-auto grid size-8 place-items-center rounded-lg bg-[#E8F4FF]/10 text-[#E8F4FF]/55" aria-hidden="true">
                  <ArrowUp className="size-4" strokeWidth={2.25} />
                </span>
              </div>
            </div>
            {/* Three across only where the panel itself has room (a container query), so no shortcut is cut. */}
            <div className="mt-2.5 grid gap-2 @xl:grid-cols-3">
              {SHORTCUTS.map((s) => {
                const Icon = s.icon;
                return (
                  <button
                    key={s.id}
                    type="button"
                    data-sc={s.id}
                    className="lk-con-short relative flex min-h-10 items-stretch overflow-hidden rounded-xl text-left outline-none focus-visible:ring-2 focus-visible:ring-[#8fcbff]"
                  >
                    <span className="flex min-w-0 flex-1 items-center gap-2 px-3 py-2.5 text-[13px] leading-5 font-medium text-[#E8F4FF]/90">
                      <Icon aria-hidden="true" className="lk-con-short-icon size-4 shrink-0 text-[#E8F4FF]/60" strokeWidth={2} />
                      <span className="truncate">{s.title}</span>
                    </span>
                    <span aria-hidden="true" className="lk-con-ripple pointer-events-none absolute inset-0" />
                  </button>
                );
              })}
            </div>
          </div>

          {/* The results the composer morphs into (the script shows one at a time). */}
          <div className="lk-con-result absolute inset-0 rounded-2xl p-4" data-kind="" aria-hidden="true">
            <div className="lk-cv lk-cv-dash flex h-full flex-col">
              <div className="flex items-center justify-between">
                <span className="text-[13px] leading-5 font-medium text-[#E8F4FF]/90">Visits by region</span>
                <span className="rounded-full bg-[#E8F4FF]/[0.08] px-2 py-0.5 text-[10.5px] leading-4 text-[#E8F4FF]/60 max-lg:text-[12px]">Illustrative</span>
              </div>
              <div className="mt-3 grid flex-1 grid-cols-3 gap-2">
                <div className="lk-cv-tile flex flex-col justify-between rounded-xl bg-[#E8F4FF]/[0.06] p-2.5" style={{ "--i": 0 } as React.CSSProperties}>
                  <span className="text-[10.5px] leading-4 text-[#E8F4FF]/55 max-lg:text-[12px]">Visits</span>
                  <span data-count="12.4" className="text-[20px] leading-6 font-medium text-[#E8F4FF] tabular-nums">12.4k</span>
                </div>
                <div className="lk-cv-tile flex flex-col justify-between rounded-xl bg-[#E8F4FF]/[0.06] p-2.5" style={{ "--i": 1 } as React.CSSProperties}>
                  <span className="text-[10.5px] leading-4 text-[#E8F4FF]/55 max-lg:text-[12px]">Trend</span>
                  <svg viewBox="0 0 80 32" className="h-8 w-full overflow-visible" fill="none">
                    <path className="lk-cv-line" d="M2 26 C 14 24, 18 12, 30 14 S 48 24, 56 12 S 70 4, 78 4" stroke="#44A6FA" strokeWidth="2" strokeLinecap="round" pathLength={1} />
                  </svg>
                </div>
                <div className="lk-cv-tile flex flex-col justify-between rounded-xl bg-[#E8F4FF]/[0.06] p-2.5" style={{ "--i": 2 } as React.CSSProperties}>
                  <span className="text-[10.5px] leading-4 text-[#E8F4FF]/55 max-lg:text-[12px]">By region</span>
                  <span className="flex h-8 items-end gap-1">
                    {[100, 74, 62, 42].map((h, k) => (
                      <span key={h} className="lk-cv-bar w-full rounded-sm bg-[#44A6FA]" style={{ height: `${h}%`, opacity: 1 - k * 0.18, "--k": k } as React.CSSProperties} />
                    ))}
                  </span>
                </div>
              </div>
            </div>
            <div className="lk-cv lk-cv-src flex h-full flex-col">
              <div className="flex items-center justify-between">
                <span className="text-[13px] leading-5 font-medium text-[#E8F4FF]/90">Connect a source</span>
                <span className="rounded-full bg-[#E8F4FF]/[0.08] px-2 py-0.5 text-[10.5px] leading-4 text-[#E8F4FF]/60 max-lg:text-[12px]">Illustrative</span>
              </div>
              <ul className="mt-3 flex flex-1 flex-col justify-center gap-2">
                {SOURCES.map((name, i) => (
                  <li key={name} className="lk-cv-chip flex items-center gap-2 rounded-xl bg-[#E8F4FF]/[0.06] px-3 py-2 text-[13px] leading-5 text-[#E8F4FF]/90" data-state="connected" style={{ "--i": i } as React.CSSProperties}>
                    {name}
                    <span className="lk-cv-state ml-auto inline-flex items-center gap-1 text-[11.5px] font-medium max-lg:text-[12px]">
                      <span className="lk-cv-wait text-[#E8F4FF]/50">Connecting…</span>
                      <span className="lk-cv-ok inline-flex items-center gap-1 text-[#7be0b0]">
                        <Check aria-hidden="true" className="size-3" strokeWidth={2.75} />
                        Connected
                      </span>
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          </div>
        </div>
      </div>
      <span aria-hidden="true" className="lk-con-pointer pointer-events-none absolute top-0 left-0 z-20 opacity-0">
        <MousePointer2 className="size-5 fill-[#E8F4FF] text-[#0b1a2d]" strokeWidth={1.5} />
      </span>
    </div>
  );
}
