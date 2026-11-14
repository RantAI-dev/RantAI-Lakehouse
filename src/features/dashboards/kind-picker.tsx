"use client";

/**
 * `BI-58` T3: the chart type above the canvas. The eight most used types are
 * icons in a row; "More" opens a gallery of every type, grouped as the
 * builder groups them (`KIND_GROUPS`), with the search the old type list had.
 *
 * Each gallery card is a small picture of the chosen data in that type,
 * drawn by the dashboard's own `TileBody` from the rows the live preview
 * already fetched (decision L2: no request of its own, at most
 * `GALLERY_ROW_CAP` rows). A type that cannot be drawn from those rows shows
 * its icon until it is picked; one the chosen source cannot make at all is
 * greyed out with one line saying why (`lib/kind-gallery`).
 */
import * as React from "react";
import { ChevronDown, MoreHorizontal, Search } from "lucide-react";
import { Popover as PopoverPrimitive } from "@base-ui/react/popover";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { ChartKind, ChartRenderSpec } from "@/lib/dashboard-specs";
import { filterKindGroups } from "@/lib/chart-kind-search";
import { GALLERY_ROW_CAP, MAIN_KINDS, drawsFrom, kindNeed, previewShape, type SourceFacts } from "@/lib/kind-gallery";
import { cn } from "@/lib/utils";
import { KIND_DESCRIPTIONS, KIND_GROUPS, KIND_KEYWORDS, KIND_LABELS, kindIcon } from "./chart-kinds";
import { TileBody } from "./tile-body";

const EASE = "transition-[background-color,color,box-shadow,opacity,transform] duration-200 ease-[cubic-bezier(0.2,0,0,1)]";
const FOCUS = "outline-none focus-visible:ring-3 focus-visible:ring-ring/50";

/** The preview the gallery draws from: its spec (with the payload as `def`) and its rows. */
export type GalleryPreview = {
  spec: ChartRenderSpec & { def?: unknown };
  columns: string[];
  rows: Record<string, unknown>[];
};

export function KindPicker({ kind, onPick, facts, preview, dark }: {
  readonly kind: ChartKind;
  readonly onPick: (next: ChartKind) => void;
  /** The chosen source's columns as the pickers count them; null before a source is chosen. */
  readonly facts: SourceFacts | null;
  /** The live preview's result, when it is rows; null otherwise. */
  readonly preview: GalleryPreview | null;
  readonly dark: boolean;
}) {
  const [open, setOpen] = React.useState(false);
  const inMain = MAIN_KINDS.includes(kind);
  const choose = (next: ChartKind) => { onPick(next); setOpen(false); };
  return (
    <div role="group" aria-label="Chart type" className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto">
      {MAIN_KINDS.map((k) => {
        const on = k === kind;
        const need = kindNeed(k, facts);
        return (
          <Tooltip key={k}>
            <TooltipTrigger
              render={
                <button
                  type="button" aria-pressed={on} aria-label={KIND_LABELS[k]} onClick={() => onPick(k)}
                  className={cn(
                    "grid size-8 shrink-0 place-items-center rounded-md", EASE, FOCUS,
                    on ? "bg-primary/12 text-primary dark:text-sky-300" : "text-muted-foreground hover:bg-muted hover:text-foreground",
                    need && !on && "opacity-50",
                  )}
                />
              }
            >
              <KindIcon kind={k} className="size-4" />
            </TooltipTrigger>
            <TooltipContent>{KIND_LABELS[k]} · {need ?? KIND_DESCRIPTIONS[k]}</TooltipContent>
          </Tooltip>
        );
      })}
      <PopoverPrimitive.Root open={open} onOpenChange={setOpen}>
        <PopoverPrimitive.Trigger
          aria-label={inMain ? "More chart types" : `More chart types, now ${KIND_LABELS[kind]}`}
          className={cn(
            "inline-flex h-8 min-w-0 shrink-0 items-center gap-1.5 rounded-md px-2 text-sm", EASE, FOCUS,
            !inMain ? "bg-primary/12 text-primary dark:text-sky-300" : "text-muted-foreground hover:bg-muted hover:text-foreground data-popup-open:bg-muted",
          )}
        >
          {inMain ? <MoreHorizontal aria-hidden className="size-4 shrink-0" /> : <KindIcon kind={kind} className="size-4 shrink-0" />}
          <span className="truncate">{inMain ? "More" : KIND_LABELS[kind]}</span>
          <ChevronDown aria-hidden className={cn("size-3.5 shrink-0", EASE, open && "rotate-180")} />
        </PopoverPrimitive.Trigger>
        <PopoverPrimitive.Portal>
          <PopoverPrimitive.Positioner side="bottom" align="start" sideOffset={6} collisionPadding={16} className="z-[60]">
            <PopoverPrimitive.Popup
              aria-label="Chart types"
              className={cn(
                "flex max-h-[min(36rem,70vh)] w-[min(46rem,calc(100vw-2rem))] flex-col rounded-xl border border-border bg-popover text-popover-foreground shadow-lg outline-hidden",
                "origin-(--transform-origin) transition duration-200 ease-out data-ending-style:scale-[0.98] data-ending-style:opacity-0 data-starting-style:scale-[0.98] data-starting-style:opacity-0",
              )}
            >
              <KindGallery kind={kind} onPick={choose} facts={facts} preview={preview} dark={dark} />
            </PopoverPrimitive.Popup>
          </PopoverPrimitive.Positioner>
        </PopoverPrimitive.Portal>
      </PopoverPrimitive.Root>
    </div>
  );
}

function KindGallery({ kind, onPick, facts, preview, dark }: {
  readonly kind: ChartKind;
  readonly onPick: (next: ChartKind) => void;
  readonly facts: SourceFacts | null;
  readonly preview: GalleryPreview | null;
  readonly dark: boolean;
}) {
  const [query, setQuery] = React.useState("");
  const groups = React.useMemo(() => filterKindGroups(KIND_GROUPS, KIND_DESCRIPTIONS, query, KIND_KEYWORDS), [query]);
  const def = (preview?.spec.def ?? undefined) as Record<string, unknown> | undefined;
  const shape = preview ? previewShape(preview.spec, Array.isArray(def?.layers) && def.layers.length > 0) : null;
  // Drawn at most once per open gallery: the rows are cut here, not per card.
  const rows = React.useMemo(() => preview?.rows.slice(0, GALLERY_ROW_CAP) ?? [], [preview]);
  return (
    <>
      <div className="relative border-b border-border/60 p-3">
        <Search aria-hidden className="pointer-events-none absolute top-1/2 left-5.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={query} onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            // Enter picks the first match the source can make, so "sank⏎" is enough.
            const first = groups.flatMap((g) => g.items).find((i) => !kindNeed(i.value, facts));
            if (e.key === "Enter" && first) { e.preventDefault(); onPick(first.value); }
          }}
          placeholder="Search chart types" aria-label="Search chart types" className="h-8 bg-background pl-8"
        />
      </div>
      <div className="grid min-h-0 gap-5 overflow-y-auto p-3">
        {groups.length === 0 ? <p className="px-1 py-2 text-sm text-muted-foreground">No chart type matches “{query.trim()}”.</p> : null}
        {groups.map((g) => (
          <section key={g.group} aria-label={g.group} className="grid gap-2">
            <h3 className="px-1 text-xs font-medium text-muted-foreground">{g.group}</h3>
            <ul className="grid grid-cols-2 gap-2 sm:grid-cols-3 md:grid-cols-4">
              {g.items.map((item) => (
                <li key={item.value}>
                  <KindCard
                    kind={item.value} label={item.label} on={item.value === kind} need={kindNeed(item.value, facts)}
                    picture={preview && drawsFrom(item.value, shape) ? { ...preview, rows, def } : null}
                    dark={dark} onPick={onPick}
                  />
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </>
  );
}

function KindCard({ kind, label, on, need, picture, dark, onPick }: {
  readonly kind: ChartKind;
  readonly label: string;
  readonly on: boolean;
  readonly need: string | null;
  readonly picture: (GalleryPreview & { def: Record<string, unknown> | undefined }) | null;
  readonly dark: boolean;
  readonly onPick: (next: ChartKind) => void;
}) {
  const id = React.useId();
  const disabled = !!need && !on;
  // The picture drops the axes, the labels and the lines: at this size they only crowd the shape.
  const spec = React.useMemo(() => picture ? {
    ...picture.spec, kind,
    def: { ...picture.def, kind, axes: { x: { show: false }, y: { show: false } }, labels: { show: false }, referenceLines: undefined, annotations: undefined },
  } : null, [picture, kind]);
  return (
    <button
      type="button" aria-pressed={on} aria-label={label} aria-describedby={id} disabled={disabled} title={label}
      onClick={() => onPick(kind)}
      className={cn(
        "grid w-full gap-2 rounded-xl p-2 text-left", EASE, FOCUS,
        on ? "bg-primary/8 ring-2 ring-primary dark:ring-sky-400" : "bg-muted/50 hover:-translate-y-0.5 hover:bg-muted hover:shadow-sm",
        "disabled:pointer-events-none disabled:opacity-50",
      )}
    >
      <span aria-hidden className="pointer-events-none block h-16 overflow-hidden rounded-lg bg-card [&_.text-4xl]:text-2xl">
        {spec && picture ? (
          <TileBody spec={spec} cell={{ columns: picture.columns, rows: picture.rows }} dark={dark} loading={false} hideLegend />
        ) : (
          <span className="grid h-full place-items-center text-muted-foreground/70"><KindIcon kind={kind} className="size-6" /></span>
        )}
      </span>
      <span className="flex min-w-0 items-center gap-2 px-0.5">
        <KindIcon kind={kind} className={cn("size-4 shrink-0", on ? "text-primary dark:text-sky-300" : "text-muted-foreground")} />
        <span className="grid min-w-0">
          <span className="truncate text-sm font-medium">{label}</span>
          <span id={id} className="truncate text-xs text-muted-foreground">{need ?? KIND_DESCRIPTIONS[kind]}</span>
        </span>
      </span>
    </button>
  );
}

/** A kind's icon; the icon is looked up, not created, so it is the same component on every render. */
export function KindIcon({ kind, className }: { readonly kind: ChartKind; readonly className?: string }) {
  return React.createElement(kindIcon(kind), { className, "aria-hidden": true });
}
