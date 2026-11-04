import { strict as assert } from "node:assert";
import { test } from "node:test";
import { homeStructuredData } from "./structured-data";

const data = homeStructuredData({ locale: "id", siteUrl: "https://example.invalid", description: "D" });
const graph = data["@graph"] as Record<string, unknown>[];

test("homeStructuredData describes the product and RantAI", () => {
  const app = graph.find((n) => n["@type"] === "SoftwareApplication")!;
  assert.equal(app.url, "https://example.invalid/id/");
  assert.equal(app.applicationCategory, "BusinessApplication");
  assert.equal(app.operatingSystem, "Linux");
  assert.equal(app.license, "https://www.gnu.org/licenses/agpl-3.0.html");
  assert.equal(app.isAccessibleForFree, true);
  assert.ok(graph.some((n) => n["@type"] === "Organization" && n.name === "RantAI"));
});

test("homeStructuredData invents no rating, review or offer (principle 2)", () => {
  const text = JSON.stringify(data);
  for (const k of ["aggregateRating", "review", "offers", "ratingValue"]) assert.ok(!text.includes(k), k);
});
