import { strict as assert } from "node:assert"
import { test } from "node:test"
import { tableKeyAssetHref } from "./table-key-href"

test("tableKeyAssetHref links a Silver or serving key to its asset page", () => {
  assert.equal(tableKeyAssetHref("silver.orders"), "/data/assets/silver.orders")
  assert.equal(tableKeyAssetHref("serving.daily_sales", "quality"), "/data/assets/serving.daily_sales?tab=quality")
})

test("tableKeyAssetHref gives a Bronze or unknown key no link", () => {
  assert.equal(tableKeyAssetHref("bronze.orders"), null)
  assert.equal(tableKeyAssetHref("orders"), null)
})
