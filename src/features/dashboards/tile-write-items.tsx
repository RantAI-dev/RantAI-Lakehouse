import { Copy, Pencil, Trash2 } from "lucide-react";

import type { TileMenuItem } from "./dashboard-grid";

/**
 * The write entries of a chart tile's ⋯ menu: "Edit chart", "Duplicate" and
 * "Delete chart". A caller without `dashboard:write` gets none of them (BI-18
 * part C review fix (SHOULD-FIX) R18): they used to be gated by ownership only
 * and failed on click. `edit` goes before the "Move to" entries and `remove`
 * after them.
 *
 * `BI-48` (QA `LH-BI-6-40`): "Duplicate" comes after "Edit chart" and is for
 * the same charts that can be edited — the dashboard's own, with a definition —
 * in view mode and in layout editing alike; it copies the stored definition
 * into a new chart titled "<title> (copy)".
 */
export function chartWriteItems(opts: {
  canWrite: boolean;
  /** The chart is the board's own (not a built-in tile). */
  own: boolean;
  /** The chart has a definition the builder can reopen. */
  hasDef: boolean;
  onEdit: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
}): { edit: TileMenuItem[]; remove: TileMenuItem[] } {
  if (!opts.canWrite || !opts.own) return { edit: [], remove: [] };
  return {
    edit: opts.hasDef
      ? [
          { label: "Edit chart", icon: <Pencil />, separatorBefore: true, onSelect: opts.onEdit },
          { label: "Duplicate", icon: <Copy />, onSelect: opts.onDuplicate },
        ]
      : [],
    remove: [{ label: "Delete chart", icon: <Trash2 />, destructive: true, separatorBefore: !opts.hasDef, onSelect: opts.onDelete }],
  };
}
