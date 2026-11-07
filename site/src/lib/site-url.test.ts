import { strict as assert } from "node:assert";
import { test } from "node:test";
import { parseSiteUrl, SiteUrlError } from "./site-url";

test("parseSiteUrl returns the origin of an https URL without a trailing slash", () => {
  assert.equal(parseSiteUrl("https://lakehouse.example.com"), "https://lakehouse.example.com");
  assert.equal(parseSiteUrl("https://lakehouse.example.com/"), "https://lakehouse.example.com");
  assert.equal(parseSiteUrl("  https://example.invalid  "), "https://example.invalid");
});

test("parseSiteUrl refuses a missing or empty value and names the variable", () => {
  for (const v of [undefined, "", "   "]) {
    assert.throws(() => parseSiteUrl(v), (e: unknown) => e instanceof SiteUrlError && /NEXT_PUBLIC_SITE_URL is not set/.test(e.message));
  }
});

test("parseSiteUrl refuses http, a relative value, credentials and a path", () => {
  assert.throws(() => parseSiteUrl("http://lakehouse.example.com"), /must use https/);
  assert.throws(() => parseSiteUrl("lakehouse.example.com"), /is not a URL/);
  assert.throws(() => parseSiteUrl("https://user:pw@lakehouse.example.com"), /credentials/);
  assert.throws(() => parseSiteUrl("https://example.com/lakehouse"), /origin only/);
  assert.throws(() => parseSiteUrl("https://example.com/?a=1"), /origin only/);
});
