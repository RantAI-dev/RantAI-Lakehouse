import type { Metadata } from "next";
import { AccountChangePassword } from "@/features/auth/account-change-password";

export const metadata: Metadata = {
  title: "Change password · RantAI Lakehouse",
};

/** Voluntary (not forced) password change, reachable from the navbar user menu. */
export default function Page() {
  return <AccountChangePassword />;
}
