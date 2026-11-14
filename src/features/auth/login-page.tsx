"use client";

import * as React from "react";
import dynamic from "next/dynamic";
import Image from "next/image";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { CircleAlert, Eye, EyeOff, KeyRound, Lock, Mail, TriangleAlert } from "lucide-react";
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
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ThemeToggle } from "@/components/theme-toggle";
import { cn } from "@/lib/utils";
import * as authClient from "@/services/clients/auth";
import { toServiceError } from "@/services/errors";
import { useAuth } from "./auth-provider";
import { ChangePasswordStep } from "./change-password-step";
import { LOGIN_DISPLAY, loginDisplay } from "./login-fonts";
import { SsoNotice } from "./sso-notice";
import "./login.css";

/**
 * The sign-in page (`/login`), laid out as the owner's mockup A (`WEB-15`,
 * chosen 2026-11-14): the logo right above the title, visible labels, a
 * password eye with its tooltip, a Caps Lock warning, a refusal that opens in
 * place with one short shake and the password selected for retyping, and a
 * loading button. The requests, redirects, `next` handling, SSO flag and
 * notices are the ones this page had before; only the markup changed.
 */

/**
 * The sample dashboard beside the form (`login-showcase.tsx`): client-only
 * and code-split, so ECharts is neither in the page's first bundle nor on the
 * server. No `loading` placeholder: until it arrives its area shows the panel
 * behind it only, never a skeleton imitating content (DESIGN.md §3).
 */
const LoginShowcase = dynamic(() => import("./login-showcase").then((m) => m.LoginShowcase), {
  ssr: false,
  loading: () => null,
});

/**
 * True once the browser is idle after the page came up, so the sample
 * dashboard (and its chart code) is fetched after the form is on screen and
 * ready, never competing with the first keystrokes (`WEB-15` spec: "never
 * blocking typing"). Safari has no `requestIdleCallback`; a short timeout
 * stands in for it there.
 */
function useAfterIdle(): boolean {
  const [ready, setReady] = React.useState(false);
  React.useEffect(() => {
    if (typeof window.requestIdleCallback === "function") {
      const id = window.requestIdleCallback(() => setReady(true), { timeout: 1500 });
      return () => window.cancelIdleCallback(id);
    }
    const id = window.setTimeout(() => setReady(true), 300);
    return () => window.clearTimeout(id);
  }, []);
  return ready;
}

const DESKTOP_QUERY = "(min-width: 1024px)";
function subscribeDesktop(onChange: () => void): () => void {
  const mq = window.matchMedia(DESKTOP_QUERY);
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}
/**
 * Whether the page is in its two-column layout (Tailwind's `lg`). Only one
 * preview is mounted, the one the layout shows, so a phone never runs the
 * hidden desktop dashboard and a laptop never runs the phone's tile.
 */
function useDesktop(): boolean {
  return React.useSyncExternalStore(subscribeDesktop, () => window.matchMedia(DESKTOP_QUERY).matches, () => false);
}

/** The console's interaction easing: the properties that change, never `all`. */
const EASE =
  "transition-[background-color,border-color,color,box-shadow,opacity,transform] duration-200 ease-[cubic-bezier(0.2,0,0,1)]";

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
  // The redirect takes a moment to leave the page; the button says so
  // instead of sitting still (DESIGN.md §3: no button is silent when pressed).
  const [redirecting, setRedirecting] = React.useState(false);
  React.useEffect(() => {
    // Back from the provider through the browser's page cache: the page is
    // restored as it was left, "Redirecting…" included, so reset it.
    const onPageShow = (e: PageTransitionEvent) => {
      if (e.persisted) setRedirecting(false);
    };
    window.addEventListener("pageshow", onPageShow);
    return () => window.removeEventListener("pageshow", onPageShow);
  }, []);
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
        disabled={disabled || redirecting}
        className={cn("h-11 w-full rounded-lg text-sm", EASE)}
        onClick={() => {
          setRedirecting(true);
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
        {redirecting ? <Spinner /> : <KeyRound aria-hidden />}
        {redirecting ? "Redirecting…" : "Continue with SSO"}
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

/**
 * The official Rantai Lake mark (`public/logo-*.png`), drawn whole in a
 * square box with `object-contain`, so its own 196 x 167 proportion is kept
 * (DESIGN.md §5: never squashed). Decorative: the title beside it already
 * names the product.
 */
function Logo({ className }: { className?: string }) {
  return (
    <span className={cn("relative inline-block size-7 shrink-0", className)}>
      <Image src="/logo-light.png" alt="" fill sizes="64px" className="object-contain dark:hidden" priority />
      <Image src="/logo-dark.png" alt="" fill sizes="64px" className="hidden object-contain dark:block" priority />
    </span>
  );
}

export function LoginPage() {
  const router = useRouter();
  const { refresh, status, user } = useAuth();
  const [email, setEmail] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  // The refusal's text stays while its box closes, so it folds away instead
  // of emptying first and then collapsing.
  const [shownError, setShownError] = React.useState<string | null>(null);
  const [shaking, setShaking] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [pendingPasswordChange, setPendingPasswordChange] = React.useState(false);
  const [showPassword, setShowPassword] = React.useState(false);
  // Caps Lock silently breaks password entry more often than any other
  // input mistake, and the field is masked so the user cannot see why.
  const [capsLock, setCapsLock] = React.useState(false);
  const passwordRef = React.useRef<HTMLInputElement>(null);
  const showcaseReady = useAfterIdle();
  const desktop = useDesktop();

  if (error !== null && error !== shownError) setShownError(error);

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

  // After a refusal the password is selected, ready to be typed again
  // (WEB-15 spec: "the password is selected for retyping"). This runs once
  // the inputs are enabled again, which is the same render as the error.
  React.useEffect(() => {
    if (!error) return;
    passwordRef.current?.focus();
    passwordRef.current?.select();
  }, [error]);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (submitting) return;
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
      if (serviceError.code === "too_many_requests") {
        // QA LH-SEC-2-02 (ADM-14 D13): ONE sentence, the console's, when the
        // server said how long to wait. It used to join the server's
        // sentence to the console's — "Try again later. Try again in about 5
        // minutes." — which says the same thing twice. Without a wait, the
        // server's sentence stands as it is.
        if (serviceError.retryAfterSecs && serviceError.retryAfterSecs > 0) {
          const minutes = Math.ceil(serviceError.retryAfterSecs / 60);
          const minutesText = minutes === 1 ? "1 minute" : `${minutes} minutes`;
          setError(`Too many failed sign-in attempts: try again in about ${minutesText}.`);
        } else {
          setError(serviceError.message);
        }
      } else {
        setError(
          serviceError.code === "permission_denied"
            ? "Invalid username/email or password."
            : serviceError.message || "Could not sign in. Try again."
        );
      }
      setShaking(true);
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
  // fires — so hold the column's shape with a skeleton instead. This is a
  // real load, so a skeleton is the honest placeholder (DESIGN.md §3).
  const checkingSession = status === "loading";
  const invalid = !!error;

  return (
    <div className={cn(loginDisplay.variable, "flex min-h-svh flex-col bg-background text-foreground lg:flex-row lg:gap-3 lg:p-3")}>
      {/* Left: the product, bleeding off the panel's lower right like a
          product shot (mockup A). The panel and its caption render with the
          page; the dashboard inside arrives after the form. */}
      <section
        aria-label="A preview of the console (sample data)"
        className="relative hidden flex-[1.15] overflow-hidden rounded-3xl bg-muted/55 lg:block dark:bg-card/45"
      >
        <div aria-hidden className="absolute top-[11%] left-[8%] h-[74%] w-[104%] min-w-[640px]">
          {showcaseReady && desktop ? <LoginShowcase /> : null}
        </div>
        <div className="absolute bottom-8 left-[8%] flex items-center gap-3">
          <p className="text-sm font-medium text-foreground/80">Every dataset, pipeline and dashboard in one place.</p>
          <span className="rounded-full bg-foreground/6 px-2 py-0.5 text-[11px] text-muted-foreground">Sample data</span>
        </div>
      </section>

      {/* The form, centred, with the logo right above the title (mockup A). */}
      <main className="relative flex min-h-svh flex-1 flex-col px-5 pt-5 pb-6 sm:px-10 lg:min-h-0 lg:px-14">
        <div className="flex justify-end">
          <ThemeToggle />
        </div>

        <div className="flex flex-1 items-start justify-center py-4 lg:items-center lg:py-6">
          <div className="w-full max-w-[360px]">
            {/* On a phone the product still shows: one live tile above the
                form, its height held so the form never moves when it loads. */}
            <div aria-hidden className="mb-8 h-[168px] lg:hidden">
              {showcaseReady && !desktop ? <LoginShowcase compact /> : null}
            </div>
            {/* `SEC-65` D2: a callback refused because the provider did
                not confirm the email redirects here with
                `ssoNotice=email_not_verified`, so the sentence the person
                needs is on the page they land on. `useSearchParams` needs
                the boundary. */}
            <React.Suspense fallback={null}>
              <SsoNotice />
            </React.Suspense>
            <Logo className="size-9" />
            <h1 className={cn(LOGIN_DISPLAY, "mt-5 text-[30px] leading-[1.1] text-foreground")}>
              Sign in to Rantai Lake
            </h1>
            <p className="mt-2 text-sm text-muted-foreground">Use your account to continue.</p>

            {checkingSession ? (
              <div className="mt-7 flex flex-col gap-5" aria-label="Loading" role="status">
                <Skeleton className="h-[72px] w-full" />
                <Skeleton className="h-[72px] w-full" />
                <Skeleton className="h-11 w-full" />
              </div>
            ) : (
              <div className="mt-7 flex flex-col gap-5">
                <form
                  onSubmit={onSubmit}
                  noValidate
                  className={cn("flex flex-col gap-5", shaking && "login-shake")}
                  onAnimationEnd={(e) => {
                    // Only the form's own shake: the spinner and the tooltip
                    // inside it animate too, and their events bubble here.
                    if (e.target === e.currentTarget) setShaking(false);
                  }}
                >
                  <Field data-invalid={invalid}>
                    <FieldLabel htmlFor="login-email" className="text-sm">
                      Username or email
                    </FieldLabel>
                    <InputGroup className="h-11 bg-background dark:bg-input/30">
                      <InputGroupAddon className="pl-3">
                        <Mail />
                      </InputGroupAddon>
                      <InputGroupInput
                        id="login-email"
                        type="text"
                        autoComplete="username"
                        autoCapitalize="none"
                        spellCheck={false}
                        autoFocus
                        required
                        placeholder="you or you@company.com"
                        // 16 px on a phone, so iOS Safari does not zoom
                        // into the field when it takes focus.
                        className="text-base md:text-sm"
                        aria-invalid={invalid}
                        aria-describedby={invalid ? "login-error" : undefined}
                        value={email}
                        onChange={(e) => {
                          setEmail(e.target.value);
                          setError(null);
                        }}
                        disabled={submitting}
                      />
                    </InputGroup>
                  </Field>

                  <Field data-invalid={invalid}>
                    <FieldLabel htmlFor="login-password" className="text-sm">
                      Password
                    </FieldLabel>
                    <InputGroup className="h-11 bg-background dark:bg-input/30">
                      <InputGroupAddon className="pl-3">
                        <Lock />
                      </InputGroupAddon>
                      <InputGroupInput
                        ref={passwordRef}
                        id="login-password"
                        type={showPassword ? "text" : "password"}
                        autoComplete="current-password"
                        required
                        className="text-base md:text-sm"
                        aria-invalid={invalid}
                        aria-describedby={invalid ? "login-error" : undefined}
                        value={password}
                        onChange={(e) => {
                          setPassword(e.target.value);
                          setError(null);
                        }}
                        disabled={submitting}
                        // Read the modifier off real key events rather than
                        // tracking CapsLock presses: this also catches the
                        // case where it was already on before focus landed.
                        onKeyDown={(e) => setCapsLock(e.getModifierState("CapsLock"))}
                        onKeyUp={(e) => setCapsLock(e.getModifierState("CapsLock"))}
                        onBlur={() => setCapsLock(false)}
                      />
                      <InputGroupAddon align="inline-end" className="pr-2">
                        <Tooltip>
                          <TooltipTrigger
                            render={
                              <InputGroupButton
                                size="icon-sm"
                                className="size-8"
                                // Skipped in the tab order: between password and
                                // submit, a reveal toggle is a detour for keyboard
                                // users, who can still reach it via shift-tab.
                                tabIndex={-1}
                                disabled={submitting}
                                onClick={() => setShowPassword((v) => !v)}
                                aria-label={showPassword ? "Hide password" : "Show password"}
                              />
                            }
                          >
                            {showPassword ? <EyeOff /> : <Eye />}
                          </TooltipTrigger>
                          <TooltipContent>{showPassword ? "Hide password" : "Show password"}</TooltipContent>
                        </Tooltip>
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

                  {/* The refusal opens in place (200 ms) rather than jumping
                      the button down, and folds away the same way once the
                      person types again. `role="alert"` is on it only while
                      there is an error, so the closing text is not announced. */}
                  <div
                    className={cn(
                      "grid transition-[grid-template-rows,opacity,margin] duration-200 ease-[cubic-bezier(0.2,0,0,1)] motion-reduce:transition-none",
                      error ? "mt-0 grid-rows-[1fr] opacity-100" : "-mt-5 grid-rows-[0fr] opacity-0",
                    )}
                  >
                    <div className="min-h-0 overflow-hidden">
                      <p
                        id={error ? "login-error" : undefined}
                        role={error ? "alert" : undefined}
                        aria-hidden={error ? undefined : true}
                        className="flex items-start gap-2 rounded-lg bg-destructive/10 px-3.5 py-2.5 text-sm text-destructive"
                      >
                        <CircleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
                        <span>{error ?? shownError}</span>
                      </p>
                    </div>
                  </div>

                  <Button
                    type="submit"
                    size="lg"
                    className={cn("h-11 w-full rounded-lg text-sm", EASE)}
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

        <footer className="flex items-center justify-center gap-2 text-xs text-muted-foreground lg:justify-start">
          <span>© {new Date().getFullYear()} Rantai Lake</span>
          <span aria-hidden>·</span>
          {/* Placeholder destination — point this at the docs site (or
              wherever support lives) once that URL exists. */}
          <Link href="#" className={cn("rounded-sm outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50", EASE)}>
            Documentation
          </Link>
        </footer>
      </main>
    </div>
  );
}
