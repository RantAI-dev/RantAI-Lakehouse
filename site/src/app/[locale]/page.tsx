import { notFound } from "next/navigation";
import { hasLocale } from "next-intl";
import { setRequestLocale } from "next-intl/server";
import { HomePage } from "@/components/home/home-page";
import { prefixedLocales } from "@/i18n/routing";

export default async function Page({ params }: Readonly<{ params: Promise<{ locale: string }> }>) {
  const { locale } = await params;
  if (!hasLocale(prefixedLocales, locale)) notFound();
  setRequestLocale(locale);
  return <HomePage locale={locale} />;
}
