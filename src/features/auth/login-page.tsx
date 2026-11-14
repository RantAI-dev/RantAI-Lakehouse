"use client";

import * as React from "react";
import dynamic from "next/dynamic";
import Image from "next/image";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { CircleAlert, Eye, EyeOff, KeyRound, Lock, TriangleAlert, User } from "lucide-react";
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
import { LoginContours } from "./login-contours";
import { LOGIN_DISPLAY, loginDisplay } from "./login-fonts";
import { SsoNotice } from "./sso-notice";
import "./login.css";

/**
 * The sign-in page (`/login`), laid out as the owner's mockup A (`WEB-15`,
 * chosen 2026-11-14): the brand mark top-left, visible labels, a
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

/** How long the first look lasts (`login.css`, T9): the whole sequence is under it. */
const FIRST_LOOK_MS = 900;

/**
 * True for the page's first 900 ms (`WEB-15` T9). The reveal animations in
 * `login.css` apply only under `.login-first`, so a part that appears later
 * (the form after the session check, the page again after the
 * change-password step) simply appears, and nothing replays. The server
 * renders `true`, so without JavaScript the reveal runs once and ends visible.
 */
function useFirstLook(): boolean {
  const [first, setFirst] = React.useState(true);
  React.useEffect(() => {
    const id = window.setTimeout(() => setFirst(false), FIRST_LOOK_MS);
    return () => window.clearTimeout(id);
  }, []);
  return first;
}

/**
 * The console's interaction easing: the properties that change, never `all`.
 * 160 ms (`WEB-15` T5, emil-design-eng: press feedback 100 to 160 ms, inside
 * DESIGN.md's 150 to 250), on a strong ease-out so the response starts at once.
 * `scale` and `translate` by name: Tailwind 4's `scale-*` and `translate-*`
 * set those properties, not `transform`, so a list naming only `transform`
 * would let them jump.
 */
const EASE =
  "transition-[background-color,border-color,color,box-shadow,opacity,scale,translate] duration-[160ms] ease-[cubic-bezier(0.2,0,0,1)]";

/**
 * Press feedback (`WEB-15` T5, emil-design-eng): a button gives a little
 * under the pointer, scaling to 0.97, in place of the console's 1 px drop.
 * Not on a disabled button, and not with reduced motion.
 */
const PRESS = "active:translate-y-0 motion-safe:active:not-disabled:scale-[0.97]";

/**
 * The fields' focus ring eases in rather than snapping on (`WEB-15` T5):
 * the input group already eases its border colour; this adds the ring.
 */
const FIELD_EASE =
  "transition-[color,background-color,border-color,box-shadow] duration-150 ease-[cubic-bezier(0.2,0,0,1)] motion-reduce:transition-none";

/**
 * The Sign in button in the logo's navy (`WEB-15` T4, owner 2026-11-14):
 * `#050A2F` with white text on the light page (about 19:1), a shade lighter
 * on hover. On the dark page a navy button would sink into the background,
 * so it turns over like the logo's dark variant: near-white with navy text,
 * pure white on hover. The focus ring stands off the button by the page's
 * own colour so it reads against the navy as well as the white page. While
 * signing in the button is disabled but keeps its colours (the console's
 * button would fade to half), since the spinner already says it is busy.
 * This page only: the console's primary colour and tokens are unchanged.
 */
const SIGN_IN_COLOURS =
  "bg-[#050A2F] text-white hover:bg-[#161D4A] focus-visible:border-transparent focus-visible:ring-2 focus-visible:ring-[#050A2F]/45 focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:opacity-100 dark:bg-[#F1F4F9] dark:text-[#050A2F] dark:hover:bg-white dark:focus-visible:ring-white/55";

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
        className={cn("h-11 w-full rounded-lg text-sm", EASE, PRESS)}
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
 * The official RantAI Lakehouse mark (`public/logo-*.svg`, viewBox 196 x 167),
 * drawn whole in a box with the mark's own aspect ratio, so it is never
 * letterboxed or squashed (DESIGN.md §5). Height is set by the caller; width
 * follows. Decorative: the title beside it already names the product.
 */
function Logo({ className }: { className?: string }) {
  return (
    <span className={cn("relative inline-block aspect-[196/167] h-7 shrink-0 self-start", className)}>
      <Image src="/logo-light.svg" alt="" fill unoptimized sizes="64px" className="object-contain dark:hidden" priority />
      <Image src="/logo-dark.svg" alt="" fill unoptimized sizes="64px" className="hidden object-contain dark:block" priority />
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
  const firstLook = useFirstLook();

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
        // WEB-15 review SHOULD-FIX 1: the owner's short sentence from mockup
        // A (DESIGN.md: help and status text is one short line). Still one
        // sentence for every refusal, so it does not say which part was wrong.
        setError(
          serviceError.code === "permission_denied"
            ? "Wrong username or password."
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
    <div className={cn(loginDisplay.variable, firstLook && "login-first", "flex min-h-svh flex-col bg-background text-foreground lg:flex-row lg:gap-3 lg:p-3")}>
      {/* Left: the product, bleeding off the panel's lower right like a
          product shot (mockup A). The panel and its caption render with the
          page; the dashboard inside arrives after the form. The panel is the
          lake: a solid deep navy from the logo's #050A2F, one step lighter
          so the card's lift still reads on it (T7, owner 2026-11-14: "jangan
          putih dong, biar kontras"); on the dark page a deeper blue-navy,
          still distinct from the page's neutral black. */}
      <section
        aria-label="A preview of the console (sample data)"
        className="relative hidden flex-[1.15] overflow-hidden rounded-3xl bg-[#0B1440] lg:block dark:bg-[#0A1030]"
      >
        {/* The lake under the product: depth lines, server-rendered, behind the card. */}
        <LoginContours />
        <div aria-hidden className="absolute top-[11%] left-[8%] h-[74%] w-[104%] min-w-[640px]">
          {showcaseReady && desktop ? <LoginShowcase /> : null}
        </div>
        <div className="login-reveal login-reveal-caption absolute bottom-8 left-[8%] flex items-center gap-3">
          <p className="text-sm font-medium text-white/85">Every dataset, pipeline and dashboard in one place.</p>
        </div>
      </section>

      {/* The form, centred; the brand mark sits top-left in the panel's own padding. */}
      <main className="relative flex min-h-svh flex-1 flex-col px-5 pt-5 pb-6 sm:px-10 lg:min-h-0 lg:px-14">
        {/* T9: the form side arrives in one short stagger after the lake
            (`login.css`, `.login-reveal-*`); first paint only, nothing moves
            with reduced motion, and the fields take focus and typing at once. */}
        <div className="login-reveal login-reveal-0 flex items-center justify-between">
          <Logo className="h-7" />
          <ThemeToggle className={cn(EASE, PRESS)} />
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
            {/* Centred over the form column (T8, owner 2026-11-14); the
                labels, fields and notices stay left-aligned for reading. */}
            <h1 className={cn(LOGIN_DISPLAY, "login-reveal login-reveal-1 text-center text-[28px] leading-[1.05] text-foreground sm:text-[36px]")}>
              Sign in to Lakehouse
            </h1>

            {checkingSession ? (
              <div className="login-reveal login-reveal-2 mt-8 flex flex-col gap-5" aria-label="Loading" role="status">
                <Skeleton className="h-[72px] w-full" />
                <Skeleton className="h-[72px] w-full" />
                <Skeleton className="h-11 w-full" />
              </div>
            ) : (
              <div className="mt-8 flex flex-col gap-5">
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
                  <Field data-invalid={invalid} className="login-reveal login-reveal-2">
                    <FieldLabel htmlFor="login-email" className="text-sm">
                      Username or email
                    </FieldLabel>
                    <InputGroup className={cn("h-11 bg-background dark:bg-input/30", FIELD_EASE)}>
                      <InputGroupAddon className="pl-3">
                        <User />
                      </InputGroupAddon>
                      <InputGroupInput
                        id="login-email"
                        type="text"
                        autoComplete="username"
                        autoCapitalize="none"
                        spellCheck={false}
                        autoFocus
                        required
                        placeholder="Enter your username or email"
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

                  <Field data-invalid={invalid} className="login-reveal login-reveal-3">
                    <FieldLabel htmlFor="login-password" className="text-sm">
                      Password
                    </FieldLabel>
                    <InputGroup className={cn("h-11 bg-background dark:bg-input/30", FIELD_EASE)}>
                      <InputGroupAddon className="pl-3">
                        <Lock />
                      </InputGroupAddon>
                      <InputGroupInput
                        ref={passwordRef}
                        id="login-password"
                        type={showPassword ? "text" : "password"}
                        autoComplete="current-password"
                        required
                        placeholder="Enter your password"
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
                            // A short wait before the label shows (emil-design-eng:
                            // tooltips delay so a pointer passing over does not
                            // flash one); the console's provider shows them at once.
                            delay={400}
                            render={
                              <InputGroupButton
                                size="icon-sm"
                                className={cn("size-8", EASE, PRESS)}
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
                            {/* The icons swap with a short fade and settle from 90 %,
                                so the change reads as one control turning over. */}
                            {showPassword ? (
                              <EyeOff key="hide" className="animate-in fade-in-0 zoom-in-90 duration-150 ease-out motion-reduce:animate-none" />
                            ) : (
                              <Eye key="show" className="animate-in fade-in-0 zoom-in-90 duration-150 ease-out motion-reduce:animate-none" />
                            )}
                          </TooltipTrigger>
                          <TooltipContent>{showPassword ? "Hide password" : "Show password"}</TooltipContent>
                        </Tooltip>
                      </InputGroupAddon>
                    </InputGroup>
                    {capsLock ? (
                      <p
                        role="alert"
                        // Arrives with a 150 ms fade and a 4 px settle, not a jump
                        // (WEB-15 T5); still with reduced motion.
                        className="flex items-center gap-1.5 text-sm text-amber-600 transition-[opacity,translate] duration-150 ease-[cubic-bezier(0.2,0,0,1)] starting:-translate-y-1 starting:opacity-0 motion-reduce:transition-none dark:text-amber-500"
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
                      "grid transition-[grid-template-rows,opacity,margin] ease-[cubic-bezier(0.2,0,0,1)] motion-reduce:transition-none",
                      // Opens in 200 ms, folds away faster, 150 ms (WEB-15 T5,
                      // emil-design-eng: the exit is quicker than the entrance).
                      error ? "mt-0 grid-rows-[1fr] opacity-100 duration-200" : "-mt-5 grid-rows-[0fr] opacity-0 duration-150",
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
                    className={cn("login-reveal login-reveal-4 h-11 w-full rounded-lg text-sm", SIGN_IN_COLOURS, EASE, PRESS)}
                    disabled={submitting}
                  >
                    {/* Idle and busy labels fade into each other (150 ms)
                        instead of swapping in one frame (WEB-15 T5). */}
                    <span
                      key={submitting ? "busy" : "idle"}
                      className="inline-flex items-center gap-1.5 animate-in fade-in-0 duration-150 ease-out motion-reduce:animate-none"
                    >
                      {submitting ? <Spinner /> : null}
                      {submitting ? "Signing in…" : "Sign in"}
                    </span>
                  </Button>
                </form>

                <SsoButton disabled={submitting} />
              </div>
            )}
          </div>
        </div>

        <footer className="login-reveal login-reveal-5 flex items-center justify-between gap-4 text-xs text-muted-foreground">
          <span>© {new Date().getFullYear()} RantAI Lakehouse</span>
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
