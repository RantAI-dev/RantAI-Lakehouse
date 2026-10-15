"use client";

import * as React from "react";
import { asNumber, cellContent } from "@/lib/cell-format";
import type { CondRule, ValueFormat } from "@/lib/chart-style";
import { conditionalStyle, type Domains } from "@/lib/conditional-format";
import { decimalsFor } from "@/lib/chart-axis";
import { formatValue } from "@/lib/value-format";
import { useReporting } from "./reporting-context";
import { cn } from "@/lib/utils";
import type { ColumnSetting } from "@/lib/table-types";

/**
 * One table cell's content under its column's setting (`BI-16` part A).
 * A link is an anchor only for http(s) and opens in a new tab; an image is
 * loaded only from https, sends no referrer and has a fixed maximum height;
 * everything else is text (`lib/cell-format`).
 */
export function CellValue({ value, setting, format }: { readonly value: unknown; readonly setting?: ColumnSetting; readonly format?: ValueFormat }) {
  const { numberLocale } = useReporting();
  // BI-17: the chart's number format reaches a number the column setting leaves to `auto`.
  const auto = !setting?.format || setting.format === "auto";
  if (format && auto && typeof value === "number" && asNumber(value) !== null) {
    return <>{formatValue(value, format, numberLocale, setting?.decimals ?? decimalsFor([value]))}</>;
  }
  const c = cellContent(value, setting, numberLocale);
  if (c.kind === "link") {
    return <a href={c.href} target="_blank" rel="noopener noreferrer" className="text-primary underline-offset-2 hover:underline">{c.text}</a>;
  }
  if (c.kind === "image") {
    // eslint-disable-next-line @next/next/no-img-element -- a remote image the data names; the Next image loader would need every host allowed.
    return <img src={c.src} alt={c.alt} referrerPolicy="no-referrer" loading="lazy" className="max-h-10 w-auto max-w-40 object-contain" />;
  }
  return <>{c.text}</>;
}

/** The style a column's width and wrapping ask for. */
export function columnStyle(setting?: ColumnSetting): React.CSSProperties | undefined {
  return setting?.width ? { width: setting.width, minWidth: setting.width, maxWidth: setting.width } : undefined;
}

export function cellClass(setting?: ColumnSetting, extra?: string): string {
  return cn("px-2 py-1 tabular-nums", setting?.wrap ? "whitespace-normal break-words" : "whitespace-nowrap", extra);
}

/** The inline style a conditional rule gives one cell (`BI-17` part A); `column` is `null` for a KPI. */
export function conditionalCss(rules: readonly CondRule[] | undefined, column: string | null, value: unknown, domains?: Domains): React.CSSProperties | undefined {
  const st = conditionalStyle(rules, column, value, domains);
  if (!st) return undefined;
  return { ...(st.background ? { backgroundColor: st.background } : {}), ...(st.color ? { color: st.color } : {}) };
}
