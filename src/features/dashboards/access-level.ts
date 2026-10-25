/**
 * What the caller may do with one dashboard (`BI-19` part B2, feature B1): the
 * payload's `access.level`. The server checks every write against the same
 * level, so this only decides what the page offers; an answer that cannot be
 * read never widens it.
 */
export type DashboardLevel = "view" | "edit" | "manage";

const ORDER: readonly string[] = ["view", "edit", "manage"];

/** Whether `level` includes `needed` (an unknown word includes nothing). */
export function levelAtLeast(level: string | undefined, needed: DashboardLevel): boolean {
  const have = level ? ORDER.indexOf(level) : -1;
  return have >= 0 && have >= ORDER.indexOf(needed);
}

/**
 * The offer for one level. A server older than the payload field sends none:
 * then the caller's global `dashboard:write` decides, as it did before.
 */
export function offerFor(level: string | undefined, hasWrite: boolean): { readonly canEdit: boolean; readonly canManage: boolean } {
  if (level === undefined) return { canEdit: hasWrite, canManage: hasWrite };
  return { canEdit: levelAtLeast(level, "edit"), canManage: levelAtLeast(level, "manage") };
}
