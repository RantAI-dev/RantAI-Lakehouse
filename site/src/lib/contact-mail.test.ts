import { strict as assert } from "node:assert";
import { test } from "node:test";
import en from "../messages/en/contact.json";
import id from "../messages/id/contact.json";
import { contactMail, contactMailto, TOPICS, validateContact, type ContactFields } from "./contact-mail";

/** WEB-5 D15: the form opens the right `mailto:`, in the visitor's language. */

const fields: ContactFields = {
  name: "  Mira Okafor ",
  email: "mira@example.com",
  company: "Example Co",
  topic: "walkthrough",
  message: "Can we see the Gold tables?\nThanks.",
};

test("the subject names the product, the topic and the sender", () => {
  assert.equal(contactMail(fields, en.form).subject, "RantAI Lakehouse · Product walkthrough · Mira Okafor");
  assert.equal(contactMail(fields, id.form).subject, "RantAI Lakehouse · Demo produk · Mira Okafor");
});

test("the body is the message, then who sent it", () => {
  assert.equal(
    contactMail(fields, en.form).body,
    "Can we see the Gold tables?\nThanks.\n\n—\nName: Mira Okafor\nEmail: mira@example.com\nCompany: Example Co",
  );
  assert.equal(
    contactMail({ ...fields, company: " " }, id.form).body,
    "Can we see the Gold tables?\nThanks.\n\n—\nNama: Mira Okafor\nEmail: mira@example.com",
  );
});

test("the mailto URL goes to the given address with an encoded subject and body", () => {
  const url = contactMailto("contact@rantai.dev", fields, en.form);
  assert.ok(url.startsWith("mailto:contact@rantai.dev?subject="), url);
  const q = new URL(url).search.slice(1);
  const [s, b] = q.split("&");
  assert.equal(decodeURIComponent(s.replace(/^subject=/, "")), "RantAI Lakehouse · Product walkthrough · Mira Okafor");
  assert.equal(decodeURIComponent(b.replace(/^body=/, "")), contactMail(fields, en.form).body);
  // RFC 6068: a "+" would be read as a plus, so spaces must be %20.
  assert.ok(!url.includes("+"), url);
});

test("every topic has a label in both languages", () => {
  for (const t of TOPICS) {
    assert.ok(en.form.topics[t].trim(), `en ${t}`);
    assert.ok(id.form.topics[t].trim(), `id ${t}`);
  }
  assert.deepEqual(Object.keys(en.form.topics).sort(), [...TOPICS].sort());
});

test("validateContact asks for a name, a real-looking address and a message", () => {
  assert.deepEqual(validateContact({ name: " ", email: "", message: "" }), {
    name: "required",
    email: "required",
    message: "required",
  });
  assert.deepEqual(validateContact({ name: "M", email: "not-an-address", message: "x" }), { email: "invalidEmail" });
  assert.deepEqual(validateContact({ name: "M", email: "m@example.com", message: "x" }), {});
});
