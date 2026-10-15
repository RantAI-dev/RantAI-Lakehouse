/**
 * The reference a credential can be, when the installation has a secret
 * manager (`SRC-12` part B, decision 2): the form asks for the parts of the
 * manager's reference and builds the string the API stores.
 *
 * The Rust grammar (`rust/crates/lakehouse-api/src/secret_manager/refs.rs`)
 * is the AUTHORITY: it decides at save time, and checks the secret exists and
 * the connection works before anything is stored. This file only builds the
 * string and says early what is plainly malformed, so a difference between
 * the two can make the form ask for too little (the server then refuses with
 * a reason) or too much, never let a bad reference through. The fixture
 * `rust/crates/lakehouse-api/tests/fixtures/secret_manager_refs.json` lists
 * references both sides must agree on; its valid ones are checked in
 * `secret-reference.test.ts`.
 *
 * A reference names a place (a path, a name, a version id), never a value.
 */

export type SecretManager = "vault" | "aws" | "gcp" | "azure"

/** The fields of one manager's reference, in the order the form shows them. */
export type ReferencePart = { key: string; label: string; placeholder: string; optional?: boolean }

export const REFERENCE_PARTS: Record<SecretManager, ReferencePart[]> = {
  vault: [
    { key: "path", label: "Path in Vault", placeholder: "secret/connectors/orders" },
    { key: "key", label: "Key in the secret", placeholder: "password" },
  ],
  aws: [
    { key: "secretId", label: "Secret name or ARN", placeholder: "prod/orders-db" },
    { key: "key", label: "Key in the secret", placeholder: "password", optional: true },
  ],
  gcp: [
    { key: "project", label: "Project", placeholder: "my-project-1" },
    { key: "secret", label: "Secret", placeholder: "orders-db" },
    { key: "version", label: "Version", placeholder: "latest", optional: true },
  ],
  azure: [
    { key: "vault", label: "Vault name", placeholder: "corp-vault" },
    { key: "secret", label: "Secret name", placeholder: "orders-db" },
    { key: "version", label: "Version", placeholder: "current", optional: true },
  ],
}

/** The name shown for a manager ("Secret manager: HashiCorp Vault"). */
export const SECRET_MANAGER_LABEL: Record<SecretManager, string> = {
  vault: "HashiCorp Vault",
  aws: "AWS Secrets Manager",
  gcp: "Google Secret Manager",
  azure: "Azure Key Vault",
}

export type ReferenceParts = Record<string, string>

const NAME = /^[A-Za-z0-9_.-]+$/
const segment = (s: string) => s !== "." && s !== ".." && NAME.test(s)

/** The reference string for the parts typed so far (not validated). */
export function composeReference(manager: SecretManager, parts: ReferenceParts): string {
  const part = (key: string) => (parts[key] ?? "").trim()
  switch (manager) {
    case "vault":
      return `vault:${part("path")}#${part("key")}`
    case "aws":
      return `aws-sm:${part("secretId")}${part("key") ? `#${part("key")}` : ""}`
    case "gcp":
      return `gcp-sm:projects/${part("project")}/secrets/${part("secret")}/versions/${part("version") || "latest"}`
    case "azure":
      return `azure-kv:${part("vault")}/${part("secret")}${part("version") ? `/${part("version")}` : ""}`
  }
}

/**
 * Why `reference` is plainly malformed for `manager`, or `null`. The same
 * shapes as the server's grammar; the server stays the authority.
 */
export function referenceProblem(manager: SecretManager, reference: string): string | null {
  const bad = "Not a valid reference for this secret manager."
  if (reference.length > 1024) return bad
  switch (manager) {
    case "vault": {
      const m = /^vault:([^#]*)#(.*)$/.exec(reference)
      if (!m) return bad
      const [, location, key] = m
      const [mount, ...path] = location.split("/")
      return path.length > 0 && segment(mount) && path.every(segment) && segment(key) && key.length <= 128
        ? null
        : "Give the path (mount, then the secret's path) and the key, using letters, digits, '.', '_' and '-'."
    }
    case "aws": {
      const m = /^aws-sm:([^#]*)(?:#(.*))?$/.exec(reference)
      if (!m) return bad
      const [, id, key] = m
      const idOk = /^[A-Za-z0-9/_+=.@:-]{1,2048}$/.test(id)
      const keyOk = key === undefined || (segment(key) && key.length <= 128)
      return idOk && keyOk ? null : "Give the secret's name or ARN, and optionally a key inside it."
    }
    case "gcp":
      return /^gcp-sm:projects\/[A-Za-z0-9_.:-]{1,64}\/secrets\/[A-Za-z0-9_-]{1,255}\/versions\/(latest|[0-9]{1,20})$/.test(
        reference
      )
        ? null
        : "Give the project, the secret's name, and a version number or 'latest'."
    case "azure":
      return /^azure-kv:[A-Za-z0-9-]{3,24}\/[A-Za-z0-9-]{1,127}(\/[A-Za-z0-9]{1,64})?$/.test(reference)
        ? null
        : "Give the vault's name (3 to 24 letters, digits or '-'), the secret's name, and optionally a version."
  }
}
