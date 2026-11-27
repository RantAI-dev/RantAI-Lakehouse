"use client";

import dynamic from "next/dynamic";
import type * as React from "react";
import { DeferredLive, StillSlot } from "@/components/shared/deferred-live";

const Live = dynamic(() => import("./lineage-trace"), { ssr: false, loading: () => <StillSlot /> });
const NONE = {};

/** The lineage picture's live version arrives after the first paint (shared/deferred-live.tsx). */
export function ColumnLineageHost({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <DeferredLive live={Live} liveProps={NONE}>
      {children}
    </DeferredLive>
  );
}
