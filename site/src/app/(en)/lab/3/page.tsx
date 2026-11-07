import { setRequestLocale } from "next-intl/server";
import { ConsoleLakeHero } from "@/components/lab/3/console-lake-hero";
import { LabShell, labMetadata } from "@/components/lab/lab-shell";
import { routing } from "@/i18n/routing";

export const metadata = labMetadata("Lab 3 · Console on the lake");

export default function Page() {
  setRequestLocale(routing.defaultLocale);
  return <LabShell>{(copy) => <ConsoleLakeHero copy={copy} />}</LabShell>;
}
