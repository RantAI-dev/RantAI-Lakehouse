import type { Metadata } from "next";
import { LoginPage } from "@/features/auth/login-page";

export const metadata: Metadata = {
  title: "Sign in · RantAI Lakehouse",
  robots: { index: false, follow: false },
};

/** Public route: `/login`. Outside the authenticated shell (see `AppFrame`). */
export default function Page() {
  return <LoginPage />;
}
