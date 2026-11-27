import { strict as assert } from "node:assert";
import { test } from "node:test";
import { contentSecurityPolicy, inlineScripts, scriptHash } from "./csp";

const CONTENT_SECURITY_POLICY = contentSecurityPolicy(["sha256-AAA="]);

const directives = Object.fromEntries(
  CONTENT_SECURITY_POLICY.split(";").map((d) => {
    const [name, ...values] = d.trim().split(/\s+/);
    return [name, values];
  }),
);

test("the CSP allows only this origin, with no third-party host (D18)", () => {
  for (const values of Object.values(directives)) {
    for (const v of values) assert.ok(/^'[a-z0-9-]+'$|^'sha256-[A-Za-z0-9+/=]+'$|^(data|blob):$/.test(v), `unexpected source ${v}`);
  }
  assert.deepEqual(directives["script-src"], ["'self'", "'sha256-AAA='"]);
  assert.ok(!directives["script-src"].includes("'unsafe-inline'"), "scripts are allowed by hash, not unsafe-inline");
  assert.deepEqual(directives["connect-src"], ["'self'"]);
  assert.deepEqual(directives["img-src"], ["'self'", "data:", "blob:"]);
  assert.deepEqual(directives["object-src"], ["'none'"]);
  assert.deepEqual(directives["form-action"], ["'none'"]);
});

test("the CSP has no directive a meta tag ignores", () => {
  for (const d of ["frame-ancestors", "report-uri", "report-to", "sandbox"]) assert.equal(directives[d], undefined, d);
  assert.ok(!CONTENT_SECURITY_POLICY.includes("unsafe-eval"));
});

test("scriptHash is the base64 SHA-256 of the exact text", () => {
  // echo -n 'abc' | openssl dgst -sha256 -binary | base64
  assert.equal(scriptHash("abc"), "sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=");
});

test("inlineScripts finds executable inline scripts only", () => {
  const html =
    '<script src="/a.js"></script><script>self.x=1</script><script type="application/ld+json">{"a":1}</script>' +
    '<script id="b" async="">y()</script>';
  assert.deepEqual(inlineScripts(html), ["self.x=1", "y()"]);
});
