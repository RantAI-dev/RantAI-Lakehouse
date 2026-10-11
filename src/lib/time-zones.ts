/**
 * The zone names the Settings page offers: the browser's own IANA list, plus
 * `UTC` (which `Intl.supportedValuesOf` leaves out) and the zone already
 * saved, so a value the server holds is never missing from its own picker.
 * The server has the last word: it saves a name only if the database engine
 * lists it.
 */
export function listTimeZones(saved?: string): string[] {
  let names: string[] = []
  try {
    names = Intl.supportedValuesOf("timeZone")
  } catch {
    // An old runtime: the saved zone and UTC are still offered.
  }
  const all = new Set<string>(["UTC", ...names])
  if (saved) all.add(saved)
  return [...all].sort((a, b) => a.localeCompare(b))
}
