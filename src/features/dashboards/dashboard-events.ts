"use client";

import * as React from "react";

/**
 * "A chart or board changed somewhere — reload." Copilot creates charts and
 * the board menus create/rename/delete boards from outside the open
 * dashboard, so the page listens for this rather than each caller knowing
 * about it. Both ends go through here so the channel is findable.
 */
const EVENT = "dashboards:changed";

export function notifyDashboardsChanged(): void {
  try {
    window.dispatchEvent(new Event(EVENT));
  } catch {
    /* not in a browser */
  }
}

export function useDashboardsChanged(onChange: () => void): void {
  React.useEffect(() => {
    window.addEventListener(EVENT, onChange);
    return () => window.removeEventListener(EVENT, onChange);
  }, [onChange]);
}
