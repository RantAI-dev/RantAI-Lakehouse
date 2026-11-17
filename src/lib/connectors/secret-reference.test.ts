import { describe, expect, test } from "bun:test"
import fixture from "../../../rust/crates/lakehouse-api/tests/fixtures/secret_manager_refs.json"
import { composeReference, referenceProblem, type SecretManager } from "./secret-reference"

type Case = { ref: string; manager: SecretManager }

describe("the shared reference list", () => {
  test("every valid reference is accepted by the form's check", () => {
    for (const c of fixture.valid as Case[]) {
      expect(referenceProblem(c.manager, c.ref)).toBeNull()
    }
  })

  test("every refused reference is refused by the form's check too", () => {
    for (const c of fixture.refused as Case[]) {
      expect(referenceProblem(c.manager, c.ref)).not.toBeNull()
    }
  })
})

describe("composeReference", () => {
  test("builds each manager's reference from the typed parts", () => {
    expect(composeReference("vault", { path: "secret/connectors/orders", key: "password" })).toBe(
      "vault:secret/connectors/orders#password"
    )
    expect(composeReference("aws", { secretId: "prod/orders-db", key: "password" })).toBe(
      "aws-sm:prod/orders-db#password"
    )
    expect(composeReference("aws", { secretId: "prod/orders-db", key: "" })).toBe("aws-sm:prod/orders-db")
    expect(composeReference("gcp", { project: "my-project-1", secret: "orders-db", version: "" })).toBe(
      "gcp-sm:projects/my-project-1/secrets/orders-db/versions/latest"
    )
    expect(composeReference("azure", { vault: "corp-vault", secret: "orders-db", version: "" })).toBe(
      "azure-kv:corp-vault/orders-db"
    )
  })

  test("the built references of valid parts pass the check", () => {
    const built: [SecretManager, Record<string, string>][] = [
      ["vault", { path: "kv/a", key: "k" }],
      ["aws", { secretId: "prod/orders-db" }],
      ["gcp", { project: "my-project-1", secret: "orders_db", version: "7" }],
      ["azure", { vault: "corp-vault", secret: "orders-db", version: "0123456789abcdef0123456789abcdef" }],
    ]
    for (const [manager, parts] of built) {
      expect(referenceProblem(manager, composeReference(manager, parts))).toBeNull()
    }
  })

  test("trims what was typed", () => {
    expect(composeReference("vault", { path: " kv/a ", key: " k " })).toBe("vault:kv/a#k")
  })
})
