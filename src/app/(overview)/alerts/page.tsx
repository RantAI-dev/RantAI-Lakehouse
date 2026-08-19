import type { Metadata } from "next";
import { AlertsPage } from "@/features/alerts/alerts-page";

export const metadata: Metadata = { title: "Alerts & Digests · Rantai Lake" };

/** Alerts & scheduled digests (real backend: console.alert_rule + webhook/email). */
export default function Page() {
  return <AlertsPage />;
}
