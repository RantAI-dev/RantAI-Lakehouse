import { setRequestLocale } from "next-intl/server";
import { WaterlineHero } from "@/components/lab/1/waterline-hero";
import { LabShell, labMetadata } from "@/components/lab/lab-shell";
import { routing } from "@/i18n/routing";

export const metadata = labMetadata("Lab 1 · Waterline");

export default function Page() {
  setRequestLocale(routing.defaultLocale);
  return <LabShell>{(copy) => <WaterlineHero copy={copy} />}</LabShell>;
}
