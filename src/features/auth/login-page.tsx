"use client";

import * as React from "react";
import Image from "next/image";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Eye, EyeOff, Lock, Mail, TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Field, FieldLabel } from "@/components/ui/field";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import { Spinner } from "@/components/ui/spinner";
import { Skeleton } from "@/components/ui/skeleton";
import { ThemeToggle } from "@/components/theme-toggle";
import * as authClient from "@/services/clients/auth";
import { toServiceError } from "@/services/errors";
import { useAuth } from "./auth-provider";
import { ChangePasswordStep } from "./change-password-step";
import { LoginHero } from "./login-hero";

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

function SsoButton({ disabled }: { disabled?: boolean }) {
  if (!ssoEnabled) return null;
  return (
    <>
      <div className="flex items-center gap-3">
        <span className="h-px flex-1 bg-border" />
        <span className="text-xs text-muted-foreground">or</span>
        <span className="h-px flex-1 bg-border" />
      </div>
      <Button
        type="button"
        variant="outline"
        size="lg"
        disabled={disabled}
        className="h-11 w-full"
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
  const [showPassword, setShowPassword] = React.useState(false);
  // Caps Lock silently breaks password entry more often than any other
  // input mistake, and the field is masked so the user cannot see why.
  const [capsLock, setCapsLock] = React.useState(false);

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
    } catch (err) {
      // Rejected credentials get ONE generic message regardless of cause
      // (no such email vs. wrong password), mirroring the backend's own
      // non-enumeration guarantee (see `routes/auth.rs`'s module doc).
      //
      // Everything else must NOT be flattened into that message: an
      // unreachable API or a 5xx would otherwise read as "your password is
      // wrong", sending users to reset a credential that was fine all
      // along. `authClient.parse` already maps 401 → `permission_denied`.
      const serviceError = toServiceError(err);
      setError(
        serviceError.code === "permission_denied"
          ? "Invalid email or password."
          : serviceError.message || "Could not sign in. Try again."
      );
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

  // While `/api/auth/me` is still in flight we do not yet know whether this
  // visitor already has a session. Rendering the form now means anyone with
  // a live cookie sees a sign-in prompt flash before the redirect above
  // fires — so hold the column's shape with a skeleton instead.
  const checkingSession = status === "loading";

  return (
    <div className="flex min-h-screen flex-col bg-background lg:flex-row">
      <LoginHero />

      <div className="flex flex-1 flex-col px-6 py-8 sm:px-12 lg:px-16 lg:py-10">
        <header className="flex items-center justify-between">
          <span className="flex items-center gap-2">
            <span className="relative size-7">
              <Image
                src="/logo-light.png"
                alt="Rantai Lake"
                fill
                sizes="28px"
                className="object-contain dark:hidden"
                priority
              />
              <Image
                src="/logo-dark.png"
                alt=""
                fill
                sizes="28px"
                className="hidden object-contain dark:block"
                priority
              />
            </span>
            <span className="text-sm font-semibold">Rantai Lake</span>
          </span>
          <ThemeToggle />
        </header>

        <div className="flex flex-1 items-center justify-center py-12">
          <div className="w-full max-w-[352px]">
            <h1 className="text-3xl leading-[1.2] font-semibold tracking-[-0.02em] text-foreground">
              Sign in to Rantai Lake
            </h1>
            <p className="mt-2 text-sm text-muted-foreground">
              Use your account credentials to continue.
            </p>

            {checkingSession ? (
              <div className="mt-8 flex flex-col gap-5" aria-label="Loading" role="status">
                <Skeleton className="h-[68px] w-full" />
                <Skeleton className="h-[68px] w-full" />
                <Skeleton className="h-11 w-full" />
              </div>
            ) : (
              <div className="mt-8 flex flex-col gap-5">
                <form onSubmit={onSubmit} className="flex flex-col gap-5" noValidate>
                  <Field data-invalid={!!error}>
                    <FieldLabel htmlFor="login-email" className="text-sm">
                      Email
                    </FieldLabel>
                    <InputGroup className="h-11">
                      <InputGroupAddon className="pl-3">
                        <Mail />
                      </InputGroupAddon>
                      <InputGroupInput
                        id="login-email"
                        type="email"
                        autoComplete="email"
                        autoFocus
                        required
                        placeholder="you@company.com"
                        className="text-sm"
                        aria-invalid={!!error}
                        aria-describedby={error ? "login-error" : undefined}
                        value={email}
                        onChange={(e) => setEmail(e.target.value)}
                        disabled={submitting}
                      />
                    </InputGroup>
                  </Field>

                  <Field data-invalid={!!error}>
                    <FieldLabel htmlFor="login-password" className="text-sm">
                      Password
                    </FieldLabel>
                    <InputGroup className="h-11">
                      <InputGroupAddon className="pl-3">
                        <Lock />
                      </InputGroupAddon>
                      <InputGroupInput
                        id="login-password"
                        type={showPassword ? "text" : "password"}
                        autoComplete="current-password"
                        required
                        placeholder="••••••••"
                        className="text-sm"
                        aria-invalid={!!error}
                        aria-describedby={error ? "login-error" : undefined}
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        disabled={submitting}
                        // Read the modifier off real key events rather than
                        // tracking CapsLock presses: this also catches the
                        // case where it was already on before focus landed.
                        onKeyDown={(e) => setCapsLock(e.getModifierState("CapsLock"))}
                        onKeyUp={(e) => setCapsLock(e.getModifierState("CapsLock"))}
                        onBlur={() => setCapsLock(false)}
                      />
                      <InputGroupAddon align="inline-end" className="pr-2">
                        <InputGroupButton
                          size="icon-sm"
                          // Skipped in the tab order: between password and
                          // submit, a reveal toggle is a detour for keyboard
                          // users, who can still reach it via shift-tab.
                          tabIndex={-1}
                          disabled={submitting}
                          onClick={() => setShowPassword((v) => !v)}
                          aria-label={showPassword ? "Hide password" : "Show password"}
                        >
                          {showPassword ? <EyeOff /> : <Eye />}
                        </InputGroupButton>
                      </InputGroupAddon>
                    </InputGroup>
                    {capsLock ? (
                      <p
                        role="alert"
                        className="flex items-center gap-1.5 text-sm text-amber-600 dark:text-amber-500"
                      >
                        <TriangleAlert className="size-3.5 shrink-0" aria-hidden />
                        Caps Lock is on
                      </p>
                    ) : null}
                  </Field>

                  {notice ? (
                    <p
                      role="status"
                      className="rounded-lg bg-emerald-500/10 px-3.5 py-2.5 text-sm text-emerald-600 dark:text-emerald-400"
                    >
                      {notice}
                    </p>
                  ) : null}

                  {error ? (
                    <p
                      id="login-error"
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
                    {submitting ? "Signing in…" : "Sign in"}
                  </Button>
                </form>

                <SsoButton disabled={submitting} />
              </div>
            )}
          </div>
        </div>

        <footer className="flex flex-col gap-1 text-sm text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
          <span>© {new Date().getFullYear()} Rantai Lake</span>
          {/* Placeholder destination — point this at the docs site (or
              wherever support lives) once that URL exists. */}
          <Link href="#" className="transition-colors hover:text-foreground">
            Documentation
          </Link>
        </footer>
      </div>
    </div>
  );
}
