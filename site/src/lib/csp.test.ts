import { strict as assert } from "node:assert";
import { test } from "node:test";
import { CONTENT_SECURITY_POLICY } from "./csp";

const directives = Object.fromEntries(
  CONTENT_SECURITY_POLICY.split(";").map((d) => {
    const [name, ...values] = d.trim().split(/\s+/);
    return [name, values];
  }),
);

test("the CSP allows only this origin, with no third-party host (D18)", () => {
  for (const values of Object.values(directives)) {
    for (const v of values) assert.ok(/^'[a-z-]+'$|^(data|blob):$/.test(v), `unexpected source ${v}`);
  }
  assert.deepEqual(directives["script-src"], ["'self'", "'unsafe-inline'"]);
  assert.deepEqual(directives["connect-src"], ["'self'"]);
  assert.deepEqual(directives["img-src"], ["'self'", "data:", "blob:"]);
  assert.deepEqual(directives["object-src"], ["'none'"]);
  assert.deepEqual(directives["form-action"], ["'none'"]);
});

test("the CSP has no directive a meta tag ignores", () => {
  for (const d of ["frame-ancestors", "report-uri", "report-to", "sandbox"]) assert.equal(directives[d], undefined, d);
  assert.ok(!CONTENT_SECURITY_POLICY.includes("unsafe-eval"));
});
