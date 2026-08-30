"use client";

import { useRouter } from "next/navigation";
import { PageHeader } from "@/components/patterns/page-header";
import { ChangePasswordStep } from "./change-password-step";

/**
 * Voluntary password change (as opposed to the forced rotation
 * `LoginPage` renders inline for `mustChangePassword` accounts). Reachable
 * from the navbar user menu at any time while signed in.
 */
export function AccountChangePassword() {
  const router = useRouter();
  return (
    <div className="flex flex-col gap-6">
      <PageHeader title="Change password" description="Update the password for your account." />
      <ChangePasswordStep
        embedded
        forced={false}
        onDone={() => {
          // Same as the forced flow on /login: `POST
          // /api/auth/change-password` revokes every session for the
          // account, including this one, so the current cookie is dead
          // the instant this succeeds. Send the user to a fresh login
          // rather than back into the (now-unauthenticated) console.
          router.push("/login?passwordChanged=1");
        }}
      />
    </div>
  );
}
