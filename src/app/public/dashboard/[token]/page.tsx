import type { Metadata } from "next";
import { PublicDashboard } from "@/features/dashboards/public-dashboard";

export const metadata: Metadata = {
  title: "Shared dashboard · Rantai Lake",
  robots: { index: false, follow: false },
};

/** Rute publik read-only: /public/dashboard/<token>. Tanpa chrome konsol (AppFrame melewatinya). */
export default async function PublicDashboardPage({ params }: { params: Promise<{ token: string }> }) {
  const { token } = await params;
  return <PublicDashboard token={token} />;
}
