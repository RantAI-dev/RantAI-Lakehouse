"use client";

import * as React from "react";
import { ArrowRight, Check, Copy } from "lucide-react";
import type { ContactCopy } from "@/lib/copy";
import { contactMailto, TOPICS, validateContact, type Topic } from "@/lib/contact-mail";
import { cn } from "@/lib/utils";

/**
 * The contact form (WEB-5 D15, after the RantAI site's `contact-form.tsx`,
 * in the night palette). There is no form backend and the site sends
 * nothing (D1): "Send" lays the message out and opens the visitor's own
 * e-mail app on a `mailto:` link (`lib/contact-mail.ts`). The address is
 * then shown with a copy button, for anyone without an e-mail app.
 *
 * Keyboard and screen readers: every field has a `<label>`, the topics are
 * a radio group in a `<fieldset>` with a `<legend>` (arrow keys move the
 * choice), and a failed send moves focus to the first field in error,
 * whose message is tied to it with `aria-describedby` and announced with
 * the field. The text arrives as props from the server, so next-intl's
 * client runtime does not ship (D5).
 */

/**
 * DESIGN.md §3: every state is drawn, eased over 200 ms. Hover lifts the
 * ring, focus turns it lake blue and two pixels wide, an error turns it
 * coral, and the error text sits under the field. All of it is CSS, and
 * still under reduced motion.
 */
const field =
  "w-full rounded-xl bg-[#020514]/70 px-4 text-[16px] text-[#E8F4FF] ring-1 ring-[#E8F4FF]/15 transition-[box-shadow,background-color] duration-200 ease-out outline-none placeholder:text-[#E8F4FF]/45 hover:ring-[#E8F4FF]/30 focus:bg-[#020514] focus:ring-2 focus:ring-[#44A6FA] aria-[invalid=true]:ring-[#ff9b9b]/80 motion-reduce:transition-none sm:text-[15px]";
const label = "mb-2 flex items-baseline justify-between text-[13px] font-medium text-[#E8F4FF]/85";

export function ContactForm({ text, to }: Readonly<{ text: ContactCopy["form"]; to: string }>) {
  const id = React.useId();
  const [name, setName] = React.useState("");
  const [email, setEmail] = React.useState("");
  const [company, setCompany] = React.useState("");
  const [topic, setTopic] = React.useState<Topic>("install");
  const [message, setMessage] = React.useState("");
  const [tried, setTried] = React.useState(false);
  const [sent, setSent] = React.useState(false);
  const [copied, setCopied] = React.useState<"idle" | "copied" | "failed">("idle");
  const addressRef = React.useRef<HTMLSpanElement>(null);

  React.useEffect(() => {
    if (copied !== "copied") return;
    const t = window.setTimeout(() => setCopied("idle"), 2000);
    return () => window.clearTimeout(t);
  }, [copied]);

  // No clipboard (an insecure context, a refused permission): select the
  // address instead and say so, so the button never does nothing.
  const copy = () => {
    const fail = () => {
      const el = addressRef.current;
      const sel = window.getSelection();
      if (el && sel) {
        const range = document.createRange();
        range.selectNodeContents(el);
        sel.removeAllRanges();
        sel.addRange(range);
      }
      setCopied("failed");
    };
    if (!navigator.clipboard) return fail();
    navigator.clipboard.writeText(to).then(() => setCopied("copied"), fail);
  };

  const errors = validateContact({ name, email, message });
  const shown = (k: keyof typeof errors) => (tried ? errors[k] : undefined);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    setTried(true);
    const first = (["name", "email", "message"] as const).find((k) => errors[k]);
    if (first) {
      document.getElementById(`${id}-${first}`)?.focus();
      return;
    }
    setSent(true);
    window.location.assign(contactMailto(to, { name, email, company, topic, message }, text));
  };

  const error = (k: keyof typeof errors) => {
    const e = shown(k);
    return e ? (
      <p id={`${id}-${k}-error`} className="mt-1.5 text-[13px] text-[#ffb4b4]">
        {text[e]}
      </p>
    ) : null;
  };
  const described = (k: keyof typeof errors) => (shown(k) ? `${id}-${k}-error` : undefined);

  return (
    <form
      noValidate
      aria-label={text.label}
      onSubmit={submit}
      className="rounded-[28px] bg-[#0B1A4A]/55 p-5 shadow-[0_30px_80px_-40px_rgba(0,0,0,0.8)] ring-1 ring-[#E8F4FF]/12 backdrop-blur-md max-sm:-mx-1 sm:p-8"
    >
      <div className="grid gap-5 sm:grid-cols-2">
        <div>
          <label htmlFor={`${id}-name`} className={label}>
            {text.name}
          </label>
          <input
            id={`${id}-name`}
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoComplete="name"
            placeholder={text.namePlaceholder}
            required
            aria-invalid={!!shown("name")}
            aria-describedby={described("name")}
            className={cn(field, "h-12")}
          />
          {error("name")}
        </div>
        <div>
          <label htmlFor={`${id}-email`} className={label}>
            {text.email}
          </label>
          <input
            id={`${id}-email`}
            type="email"
            inputMode="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            autoComplete="email"
            placeholder={text.emailPlaceholder}
            required
            aria-invalid={!!shown("email")}
            aria-describedby={described("email")}
            className={cn(field, "h-12")}
          />
          {error("email")}
        </div>
        <div className="sm:col-span-2">
          <label htmlFor={`${id}-company`} className={label}>
            {text.company}
            <span className="font-normal text-[#E8F4FF]/60">{text.optional}</span>
          </label>
          <input
            id={`${id}-company`}
            value={company}
            onChange={(e) => setCompany(e.target.value)}
            autoComplete="organization"
            className={cn(field, "h-12")}
          />
        </div>

        <fieldset className="sm:col-span-2">
          <legend className={label}>{text.topic}</legend>
          <div className="grid grid-cols-1 gap-2 min-[400px]:grid-cols-2 sm:flex sm:flex-wrap">
            {TOPICS.map((k) => {
              const on = topic === k;
              return (
                <label
                  key={k}
                  className={cn(
                    "relative inline-flex min-h-11 cursor-pointer items-center justify-center rounded-full px-4 py-2 text-center text-[14px] leading-5 ring-1 transition-[background-color,color,box-shadow,transform] duration-200 ease-out active:scale-[0.98] has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-[#44A6FA] motion-reduce:transition-none sm:min-h-10",
                    // Audit #12: the site's one selected style, a lake tint with a light hairline.
                    on
                      ? "bg-[#44A6FA]/15 text-white ring-[#8fcbff]/60"
                      : "bg-transparent text-[#E8F4FF]/80 ring-[#E8F4FF]/20 hover:text-[#E8F4FF] hover:ring-[#E8F4FF]/45",
                  )}
                >
                  <input
                    type="radio"
                    name={`${id}-topic`}
                    value={k}
                    checked={on}
                    onChange={() => setTopic(k)}
                    className="sr-only"
                  />
                  {text.topics[k]}
                </label>
              );
            })}
          </div>
        </fieldset>

        <div className="sm:col-span-2">
          <label htmlFor={`${id}-message`} className={label}>
            {text.message}
          </label>
          <textarea
            id={`${id}-message`}
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            rows={5}
            placeholder={text.messagePlaceholder}
            required
            aria-invalid={!!shown("message")}
            aria-describedby={described("message")}
            className={cn(field, "min-h-36 resize-y py-3 leading-relaxed")}
          />
          {error("message")}
        </div>
      </div>

      <div className="mt-6 flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <p aria-live="polite" className="text-[13px] leading-relaxed text-[#E8F4FF]/70 sm:max-w-xs">
          {sent ? (
            <span className="inline-flex items-center gap-1.5 text-[#E8F4FF]">
              <Check className="size-3.5 text-[#7be0b0]" aria-hidden="true" />
              {text.opening}
            </span>
          ) : (
            text.note
          )}
        </p>
        <button
          type="submit"
          className="group inline-flex h-12 shrink-0 items-center justify-center gap-2 rounded-full bg-[var(--lk-primary)] px-7 text-[15px] font-medium text-white transition-[transform,background-color,box-shadow] duration-200 ease-out hover:-translate-y-0.5 hover:bg-[var(--lk-primary-hover)] active:bg-[var(--lk-primary-press)] hover:shadow-[0_10px_40px_-8px_rgba(68,166,250,0.7)] active:translate-y-0 active:scale-[0.98] active:duration-100 motion-reduce:transition-none motion-reduce:hover:translate-y-0"
        >
          {text.send}
          <ArrowRight className="size-4 transition-transform duration-200 ease-out group-hover:translate-x-0.5 motion-reduce:transition-none" aria-hidden="true" />
        </button>
      </div>

      {sent && (
        <p className="lk-fade-in mt-4 flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-[#E8F4FF]/10 pt-4 text-[13px] text-[#E8F4FF]/75">
          {/* The address is its own node, so a failed copy can select it. */}
          {text.fallback.split("{email}")[0]}
          <span ref={addressRef} className="text-[#E8F4FF]">
            {to}
          </span>
          {text.fallback.split("{email}")[1]}
          <button
            type="button"
            onClick={copy}
            className={cn(
              "inline-flex min-h-10 items-center gap-1.5 rounded-full px-3 font-medium ring-1 transition-[background-color,color,box-shadow,transform] duration-200 ease-out active:scale-95 motion-reduce:transition-none",
              copied === "copied"
                ? "bg-[#7be0b0]/12 text-[#7be0b0] ring-[#7be0b0]/40"
                : "text-[#E8F4FF] ring-[#E8F4FF]/20 hover:bg-[#E8F4FF]/[0.08] hover:ring-[#E8F4FF]/40",
            )}
          >
            {copied === "copied" ? (
              <Check className="size-3.5" aria-hidden="true" />
            ) : (
              <Copy className="size-3.5" aria-hidden="true" />
            )}
            <span aria-live="polite">
              {copied === "copied" ? text.copied : copied === "failed" ? text.copyFailed : text.copy}
            </span>
          </button>
        </p>
      )}
    </form>
  );
}
