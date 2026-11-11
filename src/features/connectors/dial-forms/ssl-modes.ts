/**
 * The TLS modes a connection form offers (`WEB-8` D8, spec `web-8.md`):
 * a closed list per adapter instead of a free-text field, because the mode
 * string is the adapter's own vocabulary and a typo in it is silently
 * ignored by the driver.
 *
 * They were private to `cdc-dial-form.tsx`; the batch SQL and Oracle forms
 * offer the same lists now, so one module owns them (rule 4). `disable` is
 * always labelled as not encrypted: it is a deliberate choice, never
 * something a person can pick by accident (`SEC-14`).
 */
export type SslModeOption = { value: string; label: string }

/** The TLS modes a change-capture dial may name (the Debezium worker's four). */
export const CDC_SSL_MODES: SslModeOption[] = [
  { value: "require", label: "Require (encrypted, certificate not checked)" },
  { value: "verify-ca", label: "Verify CA" },
  { value: "verify-full", label: "Verify CA and host name" },
  { value: "disable", label: "Disable (not encrypted)" },
]

/** Oracle takes two modes (`SRC-4` slice 6, S6-c): its thin driver does TLS only over TCPS with a
 * Java trust store (`SRC-37`), so `require` is Oracle's native network encryption (AES256, SHA256
 * checksum: encrypted, the server not authenticated) and the verify modes are not offered. */
export const ORACLE_SSL_MODES: SslModeOption[] = [
  { value: "require", label: "Require (native network encryption, server not authenticated)" },
  { value: "disable", label: "Disable (not encrypted)" },
]

/**
 * The sentinel behind a form's "Default" entry, which sends no `sslMode` at
 * all (`null`): that is what the field's blank has always meant, and the API
 * stores the field as optional (`SqlDial::ssl_mode: Option<String>`,
 * `lakehouse-store/src/ingest_spec.rs`), so nothing is invented by leaving it
 * out. A real mode is never this string.
 */
export const SSL_MODE_DEFAULT = "__default__"

/**
 * The choices one form offers: the adapter's own modes, the "Default" entry
 * first, and — last — any value the dial already holds that is not among
 * them, so an old free-text value is still shown and is not dropped when the
 * form is saved untouched.
 */
export function sslModeChoices(modes: readonly SslModeOption[], current: string | null): SslModeOption[] {
  const known = current !== null && current !== "" ? modes.some((m) => m.value === current) : true
  return [
    { value: SSL_MODE_DEFAULT, label: "Default (no mode sent)" },
    ...modes,
    ...(known || current === null ? [] : [{ value: current, label: `${current} (not one of the listed modes)` }]),
  ]
}

/** What a "Default" choice sends: no mode, which is the field's blank. */
export function sslModeFromChoice(choice: string): string | null {
  return choice === SSL_MODE_DEFAULT ? null : choice
}
