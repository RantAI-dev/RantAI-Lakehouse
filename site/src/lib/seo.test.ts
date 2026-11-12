import { strict as assert } from "node:assert";
import { test } from "node:test";
import { buildPageMetadata, ogImagePath, SITE_NAME } from "./seo";

const text = { title: "T", description: "D" };

test("buildPageMetadata gives the English home its root canonical and three hreflang links", () => {
  const m = buildPageMetadata({ locale: "en", path: "/", text });
  assert.equal(m.alternates?.canonical, "/");
  assert.deepEqual(m.alternates?.languages, { en: "/", id: "/id/", "x-default": "/" });
  assert.deepEqual(m.title, { absolute: "T" });
  assert.equal(m.description, "D");
});

test("buildPageMetadata gives the Indonesian page its /id/ canonical and Open Graph locale", () => {
  const m = buildPageMetadata({ locale: "id", path: "/", text });
  assert.equal(m.alternates?.canonical, "/id/");
  const og = m.openGraph as Record<string, unknown>;
  assert.equal(og.url, "/id/");
  assert.equal(og.locale, "id_ID");
  assert.deepEqual(og.alternateLocale, ["en_US"]);
  assert.equal(og.siteName, SITE_NAME);
  assert.equal(og.type, "website");
  assert.deepEqual(og.images, [{ url: "/id/og.png", width: 1200, height: 630, type: "image/png", alt: SITE_NAME }]);
});

test("ogImagePath points each language at its own png", () => {
  assert.equal(ogImagePath("en"), "/og.png");
  assert.equal(ogImagePath("id"), "/id/og.png");
});

test("buildPageMetadata asks for the large preview card and never sets robots", () => {
  const m = buildPageMetadata({ locale: "en", path: "/", text });
  assert.equal((m.twitter as Record<string, unknown>).card, "summary_large_image");
  assert.equal(m.robots, undefined);
});
