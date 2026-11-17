"use client";

import * as React from "react";

/**
 * The PDF print view's hooks into the charts (`BI-1`, plan D4). Present only
 * under `/render/dashboard/<token>`: a chart then draws without animation (a
 * frame caught half-way would print half a bar) and reports when it has
 * mounted and when ECharts says it has finished drawing, so the view knows
 * when every tile is done and can set `window.__lakehouseRenderReady`.
 * Absent everywhere else, where `useRenderMode()` is `null` and nothing changes.
 */
export type RenderMode = {
  /** A chart was created. */
  readonly chartMounted: () => void;
  /** A chart finished its first draw (or was removed before it did). */
  readonly chartFinished: () => void;
};

const RenderModeContext = React.createContext<RenderMode | null>(null);

export const RenderModeProvider = RenderModeContext.Provider;

export function useRenderMode(): RenderMode | null {
  return React.useContext(RenderModeContext);
}
