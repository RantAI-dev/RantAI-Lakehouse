/**
 * The contact form's e-mail (WEB-5 D15). The site has no server and sends
 * nothing (D1): the form lays the message out and opens the visitor's own
 * e-mail app on a `mailto:` link, as the RantAI site's contact form does.
 * Pure, so the subject and body are checked in `contact-mail.test.ts`
 * without a browser.
 */

/** The form's topics, in order; their labels are `contact.form.topics.<key>`. */
export const TOPICS = ["install", "walkthrough", "connectors", "other"] as const;
export type Topic = (typeof TOPICS)[number];

export type ContactFields = {
  name: string;
  email: string;
  company: string;
  topic: Topic;
  message: string;
};

/** The words the e-mail is built from, read from `contact.form` in the visitor's language. */
export type MailText = {
  /** "RantAI Lakehouse · {topic} · {name}" */
  subject: string;
  topics: Record<Topic, string>;
  bodyName: string;
  bodyEmail: string;
  bodyCompany: string;
};

export type FieldErrors = Partial<Record<"name" | "email" | "message", "required" | "invalidEmail">>;

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** Which required fields are empty, and whether the address looks like one. */
export function validateContact(f: Pick<ContactFields, "name" | "email" | "message">): FieldErrors {
  const errors: FieldErrors = {};
  if (!f.name.trim()) errors.name = "required";
  if (!f.email.trim()) errors.email = "required";
  else if (!EMAIL.test(f.email.trim())) errors.email = "invalidEmail";
  if (!f.message.trim()) errors.message = "required";
  return errors;
}

/** The subject and body of the e-mail, as the visitor will see them in their e-mail app. */
export function contactMail(f: ContactFields, text: MailText): { subject: string; body: string } {
  const subject = text.subject.replace("{topic}", text.topics[f.topic]).replace("{name}", f.name.trim());
  const body = [
    f.message.trim(),
    "",
    "—",
    `${text.bodyName}: ${f.name.trim()}`,
    `${text.bodyEmail}: ${f.email.trim()}`,
    ...(f.company.trim() ? [`${text.bodyCompany}: ${f.company.trim()}`] : []),
  ].join("\n");
  return { subject, body };
}

/**
 * The `mailto:` URL. `encodeURIComponent` (not `URLSearchParams`, which
 * writes spaces as `+`): RFC 6068 reads `+` literally, so a subject built
 * with it would arrive as "RantAI+Lakehouse".
 */
export function contactMailto(to: string, f: ContactFields, text: MailText): string {
  const { subject, body } = contactMail(f, text);
  return `mailto:${to}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
}
