import { strict as assert } from "node:assert";
import { test } from "node:test";
import { languageAlternates, localizedPath, unprefixedPath } from "./paths";

test("localizedPath keeps English at the root and puts Indonesian under /id/", () => {
  assert.equal(localizedPath("en", "/"), "/");
  assert.equal(localizedPath("id", "/"), "/id/");
  assert.equal(localizedPath("en", "/docs"), "/docs/");
  assert.equal(localizedPath("id", "/docs/"), "/id/docs/");
});

test("localizedPath never writes an /en/ prefix", () => {
  for (const p of ["/", "/a", "/a/b/"]) assert.ok(!localizedPath("en", p).startsWith("/en"), p);
});

test("languageAlternates lists every language and English as x-default", () => {
  assert.deepEqual(languageAlternates("/"), { en: "/", id: "/id/", "x-default": "/" });
});

test("unprefixedPath strips the Indonesian prefix and leaves English paths alone", () => {
  assert.equal(unprefixedPath("/id/"), "/");
  assert.equal(unprefixedPath("/id"), "/");
  assert.equal(unprefixedPath("/id/docs/"), "/docs/");
  assert.equal(unprefixedPath("/"), "/");
  assert.equal(unprefixedPath("/idea/"), "/idea/");
});
