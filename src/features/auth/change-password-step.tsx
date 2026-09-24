"use client";

import * as React from "react";
import { Eye, EyeOff, Lock } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Field, FieldLabel } from "@/components/ui/field";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import { Spinner } from "@/components/ui/spinner";
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
/**
 * One masked field with its own reveal toggle. Each field owns its toggle
 * rather than sharing one for the whole form: revealing "new password" to
 * check a typo should not also expose the current one on a shared screen.
 */
function PasswordField({
  id,
  label,
  value,
  onChange,
  autoComplete,
  disabled,
  invalid,
  describedBy,
  minLength,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  autoComplete: string;
  disabled: boolean;
  invalid: boolean;
  describedBy?: string;
  minLength?: number;
}) {
  const [show, setShow] = React.useState(false);
  return (
    <Field data-invalid={invalid}>
      <FieldLabel htmlFor={id} className="text-sm">
        {label}
      </FieldLabel>
      <InputGroup className="h-11">
        <InputGroupAddon className="pl-3">
          <Lock />
        </InputGroupAddon>
        <InputGroupInput
          id={id}
          type={show ? "text" : "password"}
          autoComplete={autoComplete}
          required
          minLength={minLength}
          placeholder="••••••••"
          className="text-sm"
          aria-invalid={invalid}
          aria-describedby={describedBy}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          disabled={disabled}
        />
        <InputGroupAddon align="inline-end" className="pr-2">
          <InputGroupButton
            size="icon-sm"
            tabIndex={-1}
            disabled={disabled}
            onClick={() => setShow((v) => !v)}
            aria-label={show ? `Hide ${label.toLowerCase()}` : `Show ${label.toLowerCase()}`}
          >
            {show ? <EyeOff /> : <Eye />}
          </InputGroupButton>
        </InputGroupAddon>
      </InputGroup>
    </Field>
  );
}

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
          <form onSubmit={onSubmit} className="flex flex-col gap-5" noValidate>
            {!forced ? (
              <PasswordField
                id="old-password"
                label="Current password"
                autoComplete="current-password"
                value={oldPassword}
                onChange={setOldPassword}
                disabled={submitting}
                invalid={!!error}
                describedBy={error ? "change-password-error" : undefined}
              />
            ) : null}

            <PasswordField
              id="new-password"
              label="New password"
              autoComplete="new-password"
              minLength={8}
              value={newPassword}
              onChange={setNewPassword}
              disabled={submitting}
              invalid={!!error}
              describedBy={error ? "change-password-error" : undefined}
            />

            <PasswordField
              id="confirm-password"
              label="Confirm new password"
              autoComplete="new-password"
              minLength={8}
              value={confirm}
              onChange={setConfirm}
              disabled={submitting}
              invalid={!!error}
              describedBy={error ? "change-password-error" : undefined}
            />

            {error ? (
              <p
                id="change-password-error"
                role="alert"
                className="rounded-lg bg-destructive/10 px-3.5 py-2.5 text-sm text-destructive"
              >
                {error}
              </p>
            ) : null}

            <Button
              type="submit"
              size="lg"
              className="h-11 w-full"
              disabled={submitting}
            >
              {submitting ? <Spinner /> : null}
              {submitting ? "Updating…" : "Update password"}
            </Button>
          </form>
        </CardContent>
      </Card>
  );

  if (embedded) return card;
  return <div className="flex min-h-screen items-center justify-center bg-muted/30 px-4">{card}</div>;
}
