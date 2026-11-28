"use client";

import dynamic from "next/dynamic";
import type * as React from "react";
import { DeferredLive, StillSlot } from "@/components/shared/deferred-live";
import type { Copy } from "./open-tables-view";

const Live = dynamic(() => import("./open-tables-readers"), { ssr: false, loading: () => <StillSlot /> });

/** The "one table, many readers" picture's live version arrives after the first paint (shared/deferred-live.tsx). */
export function OneTableManyReadersHost({ copy, children }: Readonly<{ copy: Copy; children: React.ReactNode }>) {
  return (
    <DeferredLive live={Live} liveProps={{ copy }}>
      {children}
    </DeferredLive>
  );
}
