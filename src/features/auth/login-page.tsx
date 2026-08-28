"use client";

import * as React from "react";
import Image from "next/image";
import { useRouter } from "next/navigation";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import * as authClient from "@/services/clients/auth";
import { useAuth } from "./auth-provider";
import { ChangePasswordStep } from "./change-password-step";

/**
 * Whether an SSO button should render at all.
 *
 * The frontend has no way to read the Rust service's `OIDC_ISSUER` /
 * `OIDC_CLIENT_ID` env vars (see `rust/crates/lakehouse-auth/README.md` —
 * OIDC is a resource-server-side concern, enabled only when both are set on
 * the API process, not exposed through any `/api/auth/*` response). Rather
 * than guess or render a button that 404s, this reads a **build-time**
 * frontend flag, `NEXT_PUBLIC_SSO_ENABLED`, that an operator sets alongside
 * `OIDC_*` when they actually wire up a provider. Unset (the default), no
 * button renders and this whole branch is dead code eliminated at build
 * time — matching "OIDC inactive when unconfigured" on the backend side.
 *
 * Turning SSO on later is exactly "flip this flag + fill in the redirect",
 * not a redesign: `ssoEnabled` is the only gate in this file, and
 * `SsoButton` below is where the actual authorization-code redirect goes.
 */
const ssoEnabled = process.env.NEXT_PUBLIC_SSO_ENABLED === "true";

function SsoButton() {
  if (!ssoEnabled) return null;
  return (
    <>
      <div className="relative my-2 flex items-center gap-3 text-xs text-muted-foreground">
        <div className="h-px flex-1 bg-border" />
        <span>or</span>
        <div className="h-px flex-1 bg-border" />
      </div>
      <Button
        type="button"
        variant="outline"
        className="w-full"
        onClick={() => {
          // The authorization-code redirect itself is deliberately not
          // implemented here — `lakehouse-auth`'s `OidcAuthenticator` is a
          // resource server, not an OIDC client (see its README's "What
          // this crate needs from that flow is only the resulting
          // `id_token`"). Wiring a real provider means pointing this at
          // that provider's `/authorize` endpoint (or a small `/api/auth/
          // oidc/start` redirect route, if one gets added) with
          // `OIDC_CLIENT_ID` as `client_id` — not a change to this
          // component beyond this handler.
          window.location.assign("/api/auth/oidc/start");
        }}
      >
        Continue with SSO
      </Button>
    </>
  );
}

function nextPathFromQuery(): string {
  if (typeof window === "undefined") return "/";
  const raw = new URLSearchParams(window.location.search).get("next");
  // Only ever accept an in-app relative path — never redirect off-origin
  // and never bounce back into /login itself.
  if (!raw || !raw.startsWith("/") || raw.startsWith("//") || raw.startsWith("/login")) return "/";
  return raw;
}

export function LoginPage() {
  const router = useRouter();
  const { refresh, status, user } = useAuth();
  const [email, setEmail] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [pendingPasswordChange, setPendingPasswordChange] = React.useState(false);

  // Already signed in (e.g. opened /login in a second tab) — bounce home
  // instead of showing the form again.
  React.useEffect(() => {
    if (status === "authenticated" && user && !pendingPasswordChange) {
      router.replace(nextPathFromQuery());
    }
  }, [status, user, pendingPasswordChange, router]);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    if (new URLSearchParams(window.location.search).get("passwordChanged") === "1") {
      setNotice("Password updated. Sign in with your new password.");
    }
  }, []);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setNotice(null);
    setSubmitting(true);
    try {
      const result = await authClient.login(email, password);
      if (result.mustChangePassword) {
        // Route to the change-password step rather than dumping the user
        // into a console they cannot meaningfully use yet — the bootstrap
        // admin (and any account created with a temp credential) always
        // has this flag set. Do NOT refresh() / redirect into the app
        // until the rotation completes.
        setPendingPasswordChange(true);
        return;
      }
      await refresh();
      router.replace(nextPathFromQuery());
    } catch {
      // One generic message regardless of cause (no such email vs. wrong
      // password) — mirrors the backend's own non-enumeration guarantee.
      setError("Invalid email or password.");
    } finally {
      setSubmitting(false);
    }
  }

  if (pendingPasswordChange) {
    return (
      <ChangePasswordStep
        forced
        onDone={() => {
          // `POST /api/auth/change-password` revokes EVERY session for the
          // account — including the one this very request was authenticated
          // with (see `routes/auth.rs`'s doc comment: "a credential
          // rotation should not leave old sessions valid"). There is no new
          // cookie to pick up, so this can't `refresh()`/redirect into the
          // app — the only correct next step is a fresh login with the new
          // password.
          setPendingPasswordChange(false);
          setPassword("");
          setNotice("Password updated. Sign in with your new password.");
        }}
      />
    );
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-muted/30 px-4">
      <Card className="w-full max-w-sm">
        <CardHeader className="items-center text-center">
          <div className="relative mb-2 h-8 w-32">
            <Image src="/logo-light.png" alt="Rantai Lake" fill sizes="128px" className="object-contain dark:hidden" priority />
            <Image src="/logo-dark.png" alt="" fill sizes="128px" className="hidden object-contain dark:block" priority />
          </div>
          <CardTitle>Sign in</CardTitle>
          <CardDescription>Sign in to your Rantai Lake workspace.</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={onSubmit} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="login-email">Email</Label>
              <Input
                id="login-email"
                type="email"
                autoComplete="email"
                required
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                disabled={submitting}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="login-password">Password</Label>
              <Input
                id="login-password"
                type="password"
                autoComplete="current-password"
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                disabled={submitting}
              />
            </div>
            {notice ? (
              <p role="status" className="text-sm text-emerald-600 dark:text-emerald-400">
                {notice}
              </p>
            ) : null}
            {error ? (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            ) : null}
            <Button type="submit" className="w-full" disabled={submitting}>
              {submitting ? <Loader2 className="size-4 animate-spin" /> : null}
              Sign in
            </Button>
          </form>
          <SsoButton />
        </CardContent>
      </Card>
    </div>
  );
}
