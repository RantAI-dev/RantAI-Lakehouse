"use client";

import * as React from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import * as authClient from "@/services/clients/auth";
import { toServiceError } from "@/services/errors";

/**
 * Change-password step. Rendered inline by `LoginPage` right after a login
 * response comes back with `mustChangePassword: true` — the bootstrap
 * admin (and any temp-credential account) is stuck here until the
 * rotation completes; there is no "skip" affordance, mirroring the backend
 * treating the bootstrap password as "good for logging in and nothing
 * else" (see `routes/auth.rs`'s `change_password` doc comment).
 *
 * `forced` mode never asks for the old password — `POST
 * /api/auth/change-password` doesn't require/read `oldPassword` for a
 * forced rotation, so this component doesn't collect it either.
 */
export function ChangePasswordStep({
  forced,
  onDone,
  embedded = false,
}: {
  forced: boolean;
  onDone: () => void | Promise<void>;
  /** True when rendered inside the authenticated console shell (`/account/change-password`) rather than full-page over `/login`. */
  embedded?: boolean;
}) {
  const [oldPassword, setOldPassword] = React.useState("");
  const [newPassword, setNewPassword] = React.useState("");
  const [confirm, setConfirm] = React.useState("");
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (newPassword.length < 8) {
      setError("New password must be at least 8 characters.");
      return;
    }
    if (newPassword !== confirm) {
      setError("Passwords do not match.");
      return;
    }
    setSubmitting(true);
    try {
      await authClient.changePassword({
        oldPassword: forced ? undefined : oldPassword,
        newPassword,
      });
      await onDone();
    } catch (err) {
      setError(toServiceError(err).message || "Failed to change password.");
    } finally {
      setSubmitting(false);
    }
  }

  const card = (
      <Card className="w-full max-w-sm">
        <CardHeader>
          <CardTitle>Set a new password</CardTitle>
          <CardDescription>
            {forced
              ? "Your account was created with a temporary password. Choose a new one to continue."
              : "Choose a new password for your account."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={onSubmit} className="flex flex-col gap-4">
            {!forced ? (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="old-password">Current password</Label>
                <Input
                  id="old-password"
                  type="password"
                  autoComplete="current-password"
                  required
                  value={oldPassword}
                  onChange={(e) => setOldPassword(e.target.value)}
                  disabled={submitting}
                />
              </div>
            ) : null}
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="new-password">New password</Label>
              <Input
                id="new-password"
                type="password"
                autoComplete="new-password"
                required
                minLength={8}
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                disabled={submitting}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="confirm-password">Confirm new password</Label>
              <Input
                id="confirm-password"
                type="password"
                autoComplete="new-password"
                required
                minLength={8}
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                disabled={submitting}
              />
            </div>
            {error ? (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            ) : null}
            <Button type="submit" className="w-full" disabled={submitting}>
              {submitting ? <Loader2 className="size-4 animate-spin" /> : null}
              Update password
            </Button>
          </form>
        </CardContent>
      </Card>
  );

  if (embedded) return card;
  return <div className="flex min-h-screen items-center justify-center bg-muted/30 px-4">{card}</div>;
}
