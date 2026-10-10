/**
 * The six `CredentialKind` suffixes a derived connector-credential name can
 * end in (ADR 0002 Addendum 3 — see
 * `rust/crates/lakehouse-store/src/connectors.rs`'s `CredentialKind` and
 * `src/services/contracts/connectors.ts`'s mirror of it). One definition,
 * read through `credential-value-input.tsx`'s `credentialKindLabel` by both
 * `connector-create-page.tsx` (a NEW connector's credential) and
 * `connector-edit-page.tsx` (replacing an EXISTING one's), rather
 * than two label lists that could drift.
 */

import type { CredentialKind } from "@/services/contracts/connectors"

export const CREDENTIAL_KIND_OPTIONS: { value: CredentialKind; label: string }[] = [
  { value: "password", label: "Password" },
  { value: "secret_key", label: "Secret key" },
  { value: "access_key", label: "Access key" },
  { value: "api_key", label: "API key" },
  { value: "token", label: "Token" },
  { value: "private_key", label: "Private key" },
]
