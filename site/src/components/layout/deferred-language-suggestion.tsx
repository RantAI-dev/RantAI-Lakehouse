"use client";

import * as React from "react";
import dynamic from "next/dynamic";
import { afterPaint } from "@/lib/after-paint";
import { QuietBoundary } from "@/components/shared/quiet-boundary";
import type { SuggestionText } from "./language-suggestion";

const LanguageSuggestion = dynamic(() => import("./language-suggestion"), { ssr: false });

/**
 * The language offer appears 1.5 s after load and only for some visitors,
 * so its code is fetched once the page is usable (`afterPaint`), not with
 * the first JavaScript (WEB-5 D5).
 */
export function DeferredLanguageSuggestion({ text }: Readonly<{ text: SuggestionText }>) {
  const ready = React.useSyncExternalStore(subscribeReady, () => isReady, () => false);
  if (!ready) return null;
  // A chunk that fails to load just means no offer (review fix R1).
  return (
    <QuietBoundary>
      <LanguageSuggestion text={text} />
    </QuietBoundary>
  );
}

let isReady = false;
let scheduled = false;
const listeners = new Set<() => void>();
function subscribeReady(onChange: () => void): () => void {
  listeners.add(onChange);
  if (!scheduled) {
    scheduled = true;
    afterPaint(() => {
      isReady = true;
      for (const l of listeners) l();
    });
  }
  return () => {
    listeners.delete(onChange);
  };
}
