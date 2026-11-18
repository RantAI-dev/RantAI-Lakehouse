import type { Metadata } from "next";
import { AlertsView } from "@/features/alerts/alerts-view";

export const metadata: Metadata = { title: "Alerts & Digests · Rantai Lake" };

/** Alerts & scheduled digests (real backend: console.alert_rule + webhook/email) and platform incident triage. */
export default function Page() {
  return <AlertsView />;
}
