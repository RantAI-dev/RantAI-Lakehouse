import { setRequestLocale } from "next-intl/server";
import { ContactPage } from "@/components/contact/contact-page";
import { routing } from "@/i18n/routing";
import { pageMetadata } from "@/lib/seo";

/** `out/contact/index.html`, in English (WEB-5 D15). */
export function generateMetadata() {
  return pageMetadata({ locale: routing.defaultLocale, path: "/contact", page: "contact" });
}

export default function Page() {
  setRequestLocale(routing.defaultLocale);
  return <ContactPage locale={routing.defaultLocale} />;
}
