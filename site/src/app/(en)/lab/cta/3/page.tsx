import type { Metadata } from "next";
import { LAB_META, LabShell } from "@/components/lab/cta/lab-shell";
import { CtaCard } from "@/components/lab/cta/3/cta-card";

export const metadata: Metadata = { title: "Lab: footer card 3", ...LAB_META };

export default function Page() {
  return <LabShell card={<CtaCard />} />;
}
