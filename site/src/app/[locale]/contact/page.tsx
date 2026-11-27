import { notFound } from "next/navigation";
import { hasLocale } from "next-intl";
import { setRequestLocale } from "next-intl/server";
import { ContactPage } from "@/components/contact/contact-page";
import { prefixedLocales } from "@/i18n/routing";
import { pageMetadata } from "@/lib/seo";

/** `out/<locale>/contact/index.html`: today `/id/contact/` (WEB-5 D15). */
export async function generateMetadata({ params }: Readonly<{ params: Promise<{ locale: string }> }>) {
  const { locale } = await params;
  // `dynamicParams = false` in the layout: no other value is ever written.
  if (!hasLocale(prefixedLocales, locale)) return {};
  return pageMetadata({ locale, path: "/contact", page: "contact" });
}

export default async function Page({ params }: Readonly<{ params: Promise<{ locale: string }> }>) {
  const { locale } = await params;
  if (!hasLocale(prefixedLocales, locale)) notFound();
  setRequestLocale(locale);
  return <ContactPage locale={locale} />;
}
