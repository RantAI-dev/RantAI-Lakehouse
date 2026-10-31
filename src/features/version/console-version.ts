import { version as packageVersion } from "../../../package.json"

/**
 * The console's OWN version — the `package.json` version, read at build
 * time (REL-2 S1-T5: the console build gets its version from
 * `package.json`).
 *
 * Importing it (rather than a build argument that a deployment has to keep
 * in step) means the console can never claim a version its own package
 * does not carry: one source, and the release pipeline publishes that same
 * number.
 */
export const CONSOLE_VERSION: string = packageVersion

/**
 * The mismatch the banner reports, or `null` when there is none.
 *
 * A missing server version is NOT a mismatch: the console does not know
 * what the server runs (the health route did not answer), and saying "the
 * server runs unknown" would be noise, not information.
 */
export function versionMismatch(
  serverVersion: string | null | undefined
): { server: string; console: string } | null {
  if (!serverVersion) return null
  if (serverVersion === CONSOLE_VERSION) return null
  return { server: serverVersion, console: CONSOLE_VERSION }
}
