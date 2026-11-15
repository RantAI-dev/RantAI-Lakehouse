"use client";

import * as React from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { KPI_FIT_NATURAL, fitKpi, sameKpiFit, type KpiFit } from "@/lib/kpi-fit";
import { cn } from "@/lib/utils";

/*
 * Tile text in a narrow space (owner QA 2026-11-15: with the AI Copilot panel
 * open, four KPI tiles in a row are about 310 px each; a title ended in "…"
 * with no way to read it, and a number was cut on both sides).
 *
 * `OverflowText` is a line that ends in "…" (or a block clamped to two lines)
 * and, only when it is actually cut, reads in full in a tooltip on hover and on
 * keyboard focus. DESIGN.md: no tooltip that repeats what is visible, so a line
 * that fits has none and is not a tab stop.
 *
 * `KpiValue` is a headline number that always fits its tile (`fitKpi`).
 */

/** Whether `el`'s text is cut: wider than its box on one line, taller than it when clamped. */
function isCut(el: HTMLElement, lines: 1 | 2): boolean {
  return lines === 1 ? el.scrollWidth > el.clientWidth + 1 : el.scrollHeight > el.clientHeight + 1;
}

/**
 * Tracks whether the element is cut. A ResizeObserver catches the tile, the
 * window or a side panel changing the width, and a header's hover tools
 * taking their place (the title then gets shorter while it is hovered, and
 * its tooltip turns on).
 */
function useCut(ref: React.RefObject<HTMLElement | null>, lines: 1 | 2, text: string): boolean {
  const [cut, setCut] = React.useState(false);
  React.useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const check = () => setCut(isCut(el, lines));
    check();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(check);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref, lines, text]);
  return cut;
}

export function OverflowText({
  text, lines = 1, as = "span", className, children,
}: {
  /** The full text, said by the tooltip. */
  readonly text: string;
  /** 1: one line ending in "…"; 2: at most two lines, the second ending in "…". */
  readonly lines?: 1 | 2;
  readonly as?: "span" | "p";
  readonly className?: string;
  /** What is drawn, when it is not `text` itself. */
  readonly children?: React.ReactNode;
}) {
  const ref = React.useRef<HTMLElement>(null);
  const cut = useCut(ref, lines, text);
  const classes = cn(lines === 1 ? "truncate" : "line-clamp-2 break-words", className);
  return (
    <Tooltip disabled={!cut}>
      <TooltipTrigger
        // Focusable only when there is something to read in the tooltip.
        tabIndex={cut ? 0 : undefined}
        render={as === "p"
          ? <p ref={ref as React.RefObject<HTMLParagraphElement | null>} className={classes} />
          : <span ref={ref as React.RefObject<HTMLSpanElement | null>} className={classes} />}
      >
        {children ?? text}
      </TooltipTrigger>
      <TooltipContent className="max-w-80 break-words">{text}</TooltipContent>
    </Tooltip>
  );
}

type Measured = { full: number; compact: number | null; basePx: number };

/**
 * The widths of the two texts at the element's base size (its classes, not
 * the size a fit last gave it), measured on a hidden copy that lives only for
 * the measure: the number is never written twice in the page.
 */
function measure(el: HTMLElement, full: string, compact: string | null): Measured {
  const probe = el.cloneNode(false) as HTMLElement;
  probe.removeAttribute("id");
  probe.removeAttribute("tabindex");
  probe.setAttribute("aria-hidden", "true");
  Object.assign(probe.style, {
    fontSize: "", position: "absolute", visibility: "hidden", pointerEvents: "none",
    width: "max-content", maxWidth: "none", overflow: "visible", whiteSpace: "nowrap", left: "0", top: "0",
  });
  (el.parentElement ?? el).appendChild(probe);
  probe.textContent = full;
  const fullWidth = probe.getBoundingClientRect().width;
  let compactWidth: number | null = null;
  if (compact) {
    probe.textContent = compact;
    compactWidth = probe.getBoundingClientRect().width;
  }
  const basePx = Number.parseFloat(getComputedStyle(probe).fontSize) || 0;
  probe.remove();
  return { full: fullWidth, compact: compactWidth, basePx };
}

/** The width the number may use: its box's content width, which never depends on the number. */
function roomOf(box: HTMLElement): number {
  const s = getComputedStyle(box);
  return box.clientWidth - (Number.parseFloat(s.paddingLeft) || 0) - (Number.parseFloat(s.paddingRight) || 0);
}

/**
 * A KPI's headline number (owner QA 2026-11-15). It is measured before the
 * first paint (layout effect) and again when its box changes size or the
 * web font arrives, so it never paints cut and then jumps. The fit reads only
 * the box's width and the two texts' widths at the base size, none of which
 * the fit itself changes, so a resize cannot loop.
 *
 * The box is the element's parent: it must be as wide as the tile's content
 * whatever the number's width (a `grid-cols-[minmax(0,1fr)]` or block parent,
 * not a shrink-to-fit one).
 */
export function KpiValue({
  full, compact, className, style,
}: {
  /** The value in full, with its thousand separators. */
  readonly full: string;
  /** The value's compact form ("13.3 M"), or null when it has none shorter. */
  readonly compact: string | null;
  /** Classes of the number, including its base size (`text-4xl`). */
  readonly className?: string;
  readonly style?: React.CSSProperties;
}) {
  const ref = React.useRef<HTMLParagraphElement>(null);
  const [{ fit, basePx }, setShown] = React.useState<{ fit: KpiFit; basePx: number }>({ fit: KPI_FIT_NATURAL, basePx: 0 });
  React.useLayoutEffect(() => {
    const el = ref.current;
    const box = el?.parentElement;
    if (!el || !box) return;
    let widths = measure(el, full, compact);
    const update = () => {
      const next = fitKpi(roomOf(box) - 1, widths.full, widths.compact);
      setShown((prev) => (sameKpiFit(prev.fit, next) && prev.basePx === widths.basePx ? prev : { fit: next, basePx: widths.basePx }));
    };
    update();
    const ro = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    ro?.observe(box);
    const fonts = typeof document !== "undefined" ? document.fonts : undefined;
    const remeasure = () => {
      widths = measure(el, full, compact);
      update();
    };
    fonts?.addEventListener?.("loadingdone", remeasure);
    return () => {
      ro?.disconnect();
      fonts?.removeEventListener?.("loadingdone", remeasure);
    };
  }, [full, compact]);

  const shortened = fit.mode === "compact" && compact !== null;
  const sized: React.CSSProperties | undefined = fit.scale < 1 && basePx > 0
    ? { ...style, fontSize: `${basePx * fit.scale}px` }
    : style;
  return (
    <Tooltip disabled={!shortened}>
      <TooltipTrigger
        tabIndex={shortened ? 0 : undefined}
        // As wide as the number (a conditional colour's fill hugs it), never wider than the tile.
        render={<p ref={ref} className={cn("w-fit max-w-full justify-self-center truncate whitespace-nowrap", className)} style={sized} />}
      >
        {shortened ? (
          <>
            <span aria-hidden>{compact}</span>
            <span className="sr-only">{full}</span>
          </>
        ) : full}
      </TooltipTrigger>
      <TooltipContent className="tabular-nums">{full}</TooltipContent>
    </Tooltip>
  );
}
