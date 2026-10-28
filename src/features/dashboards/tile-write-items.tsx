import { Pencil, Trash2 } from "lucide-react";

import type { TileMenuItem } from "./dashboard-grid";

/**
 * The write entries of a chart tile's ⋯ menu: "Edit chart" and "Delete chart".
 * A caller without `dashboard:write` gets neither (BI-18 part C review fix
 * (SHOULD-FIX) R18): they used to be gated by ownership only and failed on
 * click. `edit` goes before the "Move to" entries and `remove` after them.
 */
export function chartWriteItems(opts: {
  canWrite: boolean;
  /** The chart is the board's own (not a built-in tile). */
  own: boolean;
  /** The chart has a definition the builder can reopen. */
  hasDef: boolean;
  onEdit: () => void;
  onDelete: () => void;
}): { edit: TileMenuItem[]; remove: TileMenuItem[] } {
  if (!opts.canWrite || !opts.own) return { edit: [], remove: [] };
  return {
    edit: opts.hasDef ? [{ label: "Edit chart", icon: <Pencil />, separatorBefore: true, onSelect: opts.onEdit }] : [],
    remove: [{ label: "Delete chart", icon: <Trash2 />, destructive: true, separatorBefore: !opts.hasDef, onSelect: opts.onDelete }],
  };
}
