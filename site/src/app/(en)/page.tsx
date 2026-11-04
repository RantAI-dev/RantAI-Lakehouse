import { setRequestLocale } from "next-intl/server";
import { HomePage } from "@/components/home/home-page";
import { routing } from "@/i18n/routing";

export default function Page() {
  setRequestLocale(routing.defaultLocale);
  return <HomePage locale={routing.defaultLocale} />;
}
