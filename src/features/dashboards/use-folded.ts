"use client";

import * as React from "react";

/**
 * The sections a viewer folded on this dashboard (BI-18 part C). Kept in this
 * browser only: a fold is a reading choice, not part of the dashboard, so
 * nothing is saved to it. Storage can be missing or throw (a private window,
 * blocked site data); the page then folds for the session and forgets.
 */
export function useFolded(key: string | undefined): [ReadonlySet<string>, (id: string) => void] {
  const storageKey = key ? `dash-folded:${key}` : undefined;
  const [folded, setFolded] = React.useState<ReadonlySet<string>>(new Set());
  // Read after mount so the server-rendered page and the first client render agree.
  React.useEffect(() => {
    if (!storageKey) return;
    try {
      const raw = window.localStorage.getItem(storageKey);
      const parsed: unknown = raw ? JSON.parse(raw) : [];
      setFolded(new Set(Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === "string") : []));
    } catch {
      setFolded(new Set());
    }
  }, [storageKey]);
  const toggle = React.useCallback((id: string) => {
    setFolded((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      if (storageKey) {
        try { window.localStorage.setItem(storageKey, JSON.stringify([...next])); } catch { /* the fold lasts for the session */ }
      }
      return next;
    });
  }, [storageKey]);
  return [folded, toggle];
}
