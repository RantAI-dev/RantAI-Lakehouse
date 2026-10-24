"use client";

import * as React from "react";

/** The message the host page listens for (`BI-26` plan D7). */
export const EMBED_SIZE_MESSAGE = "rantai-embed:size";

/** How long a change in height waits for a quieter moment, in ms. */
export const HEIGHT_DEBOUNCE_MS = 100;

/**
 * Tell the host page how tall the embed's content is, so its frame can follow
 * (`BI-26` plan D7, feature decision 9).
 *
 * The messages go only to the sites the dashboard lists as allowed
 * (`origins`), each as its own `targetOrigin`: a page that is not listed never
 * receives the height, and with no site listed nothing is posted. They are sent
 * when the rounded height of the measured element changes, after
 * {@link HEIGHT_DEBOUNCE_MS}.
 *
 * Measure an element whose height comes from its content, never from the
 * frame's height (no `min-h-screen`, no `h-screen`): a height that follows the
 * frame would grow every time the host grew the frame, and the two would chase
 * each other.
 */
export function useEmbedHeight(
  target: React.RefObject<HTMLElement | null>,
  origins: readonly string[],
  enabled: boolean,
): void {
  const key = origins.join(" ");
  React.useEffect(() => {
    const el = target.current;
    if (!enabled || !el || origins.length === 0 || typeof window === "undefined") return;
    if (window.parent === window) return;
    let last = -1;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const send = () => {
      const height = Math.round(el.getBoundingClientRect().height);
      if (height === last) return;
      last = height;
      for (const origin of origins) {
        window.parent.postMessage({ type: EMBED_SIZE_MESSAGE, height }, origin);
      }
    };
    const observer = new ResizeObserver(() => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(send, HEIGHT_DEBOUNCE_MS);
    });
    observer.observe(el);
    timer = setTimeout(send, HEIGHT_DEBOUNCE_MS);
    return () => {
      observer.disconnect();
      if (timer) clearTimeout(timer);
    };
    // `key` stands for the list of origins, so a new array with the same sites does not restart it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, key, enabled]);
}

/**
 * The sites that may frame this embed page, from `POST /api/embed/frame` with
 * the token the page was opened with (the same answer `src/proxy.ts` turns into
 * `frame-ancestors`). Any failure is no site, so nothing is posted.
 */
export async function fetchFrameOrigins(body: { jwt?: string; token?: string }): Promise<string[]> {
  try {
    const res = await fetch("/api/embed/frame", {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), cache: "no-store",
    });
    if (!res.ok) return [];
    const json = await res.json();
    return Array.isArray(json?.origins) ? json.origins.filter((o: unknown): o is string => typeof o === "string") : [];
  } catch {
    return [];
  }
}
