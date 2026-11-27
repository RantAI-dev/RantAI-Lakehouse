import { strict as assert } from "node:assert";
import { test } from "node:test";
import en from "../messages/en/contact.json";
import { faqJsonLd } from "./faq-json-ld";

test("faqJsonLd lists every question with its answer, in page order", () => {
  const data = faqJsonLd(en.faq.items);
  assert.equal(data["@type"], "FAQPage");
  const entities = data.mainEntity as { "@type": string; name: string; acceptedAnswer: { text: string } }[];
  assert.equal(entities.length, en.faq.items.length);
  entities.forEach((e, i) => {
    assert.equal(e["@type"], "Question");
    assert.equal(e.name, en.faq.items[i].q);
    assert.equal(e.acceptedAnswer.text, en.faq.items[i].a);
  });
  // It survives the JSON round trip the page's JsonLd component makes.
  assert.deepEqual(JSON.parse(JSON.stringify(data)), data);
});
