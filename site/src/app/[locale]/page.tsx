import { notFound } from "next/navigation";
import { hasLocale } from "next-intl";
import { setRequestLocale } from "next-intl/server";
import { HomePage } from "@/components/home/home-page";
import { prefixedLocales } from "@/i18n/routing";
import { pageMetadata } from "@/lib/seo";

export async function generateMetadata({ params }: Readonly<{ params: Promise<{ locale: string }> }>) {
  const { locale } = await params;
  // `dynamicParams = false` in the layout: no other value is ever written.
  if (!hasLocale(prefixedLocales, locale)) return {};
  return pageMetadata({ locale, path: "/" });
}

export default async function Page({ params }: Readonly<{ params: Promise<{ locale: string }> }>) {
  const { locale } = await params;
  if (!hasLocale(prefixedLocales, locale)) notFound();
  setRequestLocale(locale);
  return <HomePage locale={locale} />;
}
