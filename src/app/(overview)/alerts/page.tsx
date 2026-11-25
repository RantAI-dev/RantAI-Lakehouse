import { Suspense } from "react";
import type { Metadata } from "next";
import { AlertsView } from "@/features/alerts/alerts-view";
import { LoadingSkeleton } from "@/components/patterns/page-states";

export const metadata: Metadata = { title: "Alerts & Digests" };

/** Alerts & scheduled digests (real backend: console.alert_rule + webhook/email) and platform incident triage. Suspense because the view reads `?tab=` via `useSearchParams`. */
export default function Page() {
  return (
    <Suspense fallback={<LoadingSkeleton />}>
      <AlertsView />
    </Suspense>
  );
}
