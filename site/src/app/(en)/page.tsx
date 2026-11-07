import { setRequestLocale } from "next-intl/server";
import { HomePage } from "@/components/home/home-page";
import { routing } from "@/i18n/routing";
import { pageMetadata } from "@/lib/seo";

export function generateMetadata() {
  return pageMetadata({ locale: routing.defaultLocale, path: "/" });
}

export default function Page() {
  setRequestLocale(routing.defaultLocale);
  return <HomePage locale={routing.defaultLocale} />;
}
