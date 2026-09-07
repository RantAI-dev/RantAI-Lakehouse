"use client";

import * as React from "react";

/**
 * React Grab — a development-only helper for handing UI context to coding
 * agents. Hover any element, press ⌘C / Ctrl+C, and the clipboard gets the
 * element plus its component stack with source locations, e.g.
 * `[<button …> in DataTableSearch (at components/data-table/…:58:7)]`.
 *
 * Renders nothing. It exists purely for the side effect of the import.
 *
 * Two deliberate choices here:
 *
 * 1. **Local import, not the CDN `<Script>` the README suggests.** Upstream
 *    points a `<script>` at `//unpkg.com/react-grab/dist/index.global.js`,
 *    which is unpinned and unverified — every dev machine would silently
 *    pick up whatever is published at that path. The dependency is in
 *    `package.json` instead, so it is pinned by the lockfile and reviewed
 *    like any other.
 *
 * 2. **`await import()` inside an effect guarded by `NODE_ENV`.** The
 *    condition is statically `false` in a production build, so the bundler
 *    drops the dynamic import and none of the ~1 MB package reaches the
 *    client bundle. An effect (rather than a top-level import) also keeps
 *    it off the server render path — the library touches `document` on
 *    load.
 *
 * Remove the `<ReactGrabDev />` line in `app/layout.tsx` to disable it.
 */
export function ReactGrabDev() {
  React.useEffect(() => {
    if (process.env.NODE_ENV !== "development") return;

    let cancelled = false;
    void (async () => {
      try {
        await import("react-grab");
      } catch (err) {
        // A missing or broken devtool must never take the app down with
        // it — this is the one place a swallowed error is the right call,
        // so it is logged rather than rethrown.
        if (!cancelled) {
          console.warn("[react-grab] failed to load; continuing without it", err);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  return null;
}
