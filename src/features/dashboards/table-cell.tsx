"use client";

import * as React from "react";
import { cellContent } from "@/lib/cell-format";
import { cn } from "@/lib/utils";
import type { ColumnSetting } from "@/lib/table-types";

/**
 * One table cell's content under its column's setting (`BI-16` part A).
 * A link is an anchor only for http(s) and opens in a new tab; an image is
 * loaded only from https, sends no referrer and has a fixed maximum height;
 * everything else is text (`lib/cell-format`).
 */
export function CellValue({ value, setting }: { readonly value: unknown; readonly setting?: ColumnSetting }) {
  const c = cellContent(value, setting);
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
