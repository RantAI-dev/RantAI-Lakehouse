import type { Metadata } from "next";
import { PublicDashboard } from "@/features/dashboards/public-dashboard";

export const metadata: Metadata = {
  title: "Shared dashboard · Rantai Lake",
  robots: { index: false, follow: false },
};

/** Public read-only route: /public/dashboard/<token>. No console chrome (AppFrame skips it). */
export default async function PublicDashboardPage({ params }: { params: Promise<{ token: string }> }) {
  const { token } = await params;
  return <PublicDashboard token={token} />;
}
