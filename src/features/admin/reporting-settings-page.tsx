"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import {
  COMPANY_PALETTE_MAX,
  COMPANY_PALETTE_MIN,
  MAX_CONTENT_SITES,
  NUMBER_LOCALES,
  SHIPPED_FONTS,
  isHexColor,
} from "@/lib/formatting-settings"
import { listTimeZones } from "@/lib/time-zones"
import { withNotify } from "@/lib/notify"
import { settingsService, type ReportingSettingsInput } from "@/services"

const WEEK_STARTS = { monday: "Monday", sunday: "Sunday" } as const

/**
 * Admin > Settings (`BI-9`, `BI-17`): the report time zone and the first day
 * of the week, the number locale, the company palette, the allowed fonts and the content sites,
 * for the whole deployment. Anyone signed in sees the values; only a
 * holder of `settings:write` can change them (the server enforces it; the
 * controls are disabled here so the page does not offer what would be
 * refused).
 */
export function ReportingSettingsPage() {
  const { hasPermission } = useAuth()
  return <ReportingSettingsForm canWrite={hasPermission("settings:write")} />
}

export function ReportingSettingsForm({
  canWrite,
  zoneList = listTimeZones,
}: {
  canWrite: boolean
  /**
   * BI-9 review fix R6: where the zone names come from. Tests pass a short
   * list, because several hundred `<option>`s are slow to render under load;
   * the page always uses the browser's full list.
   */
  zoneList?: (saved?: string) => string[]
}) {
  const state = useService((signal) => settingsService.getReporting(signal), [])
  const [zone, setZone] = React.useState("")
  const [week, setWeek] = React.useState("monday")
  const [locale, setLocale] = React.useState("en-US")
  const [palette, setPalette] = React.useState<string[]>([])
  const [fonts, setFonts] = React.useState<string[]>([])
  const [hex, setHex] = React.useState("")
  const [sites, setSites] = React.useState<string[]>([])
  const [site, setSite] = React.useState("")
  const save = useServiceAction(
    withNotify(
      { success: "Settings saved", error: "Settings were not saved" },
      (signal, input: ReportingSettingsInput) =>
        settingsService.saveReporting(input, signal)
    )
  )

  const loaded = state.status === "success" ? state.data : null
  React.useEffect(() => {
    if (loaded) {
      setZone(loaded.timeZone)
      setWeek(loaded.weekStart)
      setLocale(loaded.numberLocale)
      setPalette(loaded.companyPalette)
      setFonts(loaded.fonts)
      setSites(loaded.contentSites ?? [])
    }
  }, [loaded])

  const zones = React.useMemo(() => zoneList(loaded?.timeZone), [zoneList, loaded?.timeZone])
  const paletteInvalid = palette.length > 0 && palette.length < COMPANY_PALETTE_MIN
  const changed =
    loaded !== null &&
    (zone !== loaded.timeZone ||
      week !== loaded.weekStart ||
      locale !== loaded.numberLocale ||
      palette.join() !== loaded.companyPalette.join() ||
      fonts.join() !== loaded.fonts.join() ||
      sites.join() !== (loaded.contentSites ?? []).join())

  function addColour() {
    const colour = hex.trim().toLowerCase()
    if (!isHexColor(colour) || palette.includes(colour) || palette.length >= COMPANY_PALETTE_MAX)
      return
    setPalette([...palette, colour])
    setHex("")
  }

  // The server normalises and has the last word; this only keeps the obvious
  // mistakes (no scheme, a path, a wildcard) from reaching it.
  const siteText = site.trim().toLowerCase()
  const siteValid = /^https:\/\/[a-z0-9.-]+(:[0-9]{1,5})?$/.test(siteText)

  function addSite() {
    if (!siteValid || sites.includes(siteText) || sites.length >= MAX_CONTENT_SITES) return
    setSites([...sites, siteText])
    setSite("")
  }

  function toggleFont(id: string, on: boolean) {
    // Kept in the shipped order, so the comparison with the saved list is stable.
    setFonts(Object.keys(SHIPPED_FONTS).filter((f) => (f === id ? on : fonts.includes(f))))
  }

  async function handleSave() {
    const result = await save.run({
      timeZone: zone,
      weekStart: week,
      numberLocale: locale,
      companyPalette: palette,
      fonts,
      contentSites: sites,
    })
    if (result) state.reload()
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Settings"
        description="Used by every dashboard, for grouping dates and for “today”."
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {loaded ? (
        <div className="flex max-w-sm flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="report-time-zone">Report time zone</Label>
            {/* A native datalist: typing filters the 400-odd names, with no
                popup to manage. The server still checks the name. */}
            <Input
              id="report-time-zone"
              aria-label="Report time zone"
              list="report-time-zones"
              value={zone}
              onChange={(e) => setZone(e.target.value)}
              placeholder="Search a time zone"
              disabled={!canWrite}
              autoComplete="off"
            />
            <datalist id="report-time-zones">
              {zones.map((name) => (
                <option key={name} value={name} />
              ))}
            </datalist>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>First day of the week</Label>
            <Select
              value={week}
              items={WEEK_STARTS}
              onValueChange={(v) => setWeek(v ?? week)}
              disabled={!canWrite}
            >
              <SelectTrigger className="w-full" aria-label="First day of the week">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {Object.entries(WEEK_STARTS).map(([value, label]) => (
                  <SelectItem key={value} value={value}>
                    {label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <section className="flex flex-col gap-1.5" aria-labelledby="settings-numbers">
            <h2 id="settings-numbers" className="text-sm font-medium">
              Numbers
            </h2>
            <Label htmlFor="number-locale">Number locale</Label>
            <Select
              value={locale}
              items={NUMBER_LOCALES}
              onValueChange={(v) => setLocale(v ?? locale)}
              disabled={!canWrite}
            >
              <SelectTrigger id="number-locale" className="w-full" aria-label="Number locale">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {Object.entries(NUMBER_LOCALES).map(([value, label]) => (
                  <SelectItem key={value} value={value}>
                    {label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              One setting for the whole deployment. Charts keep their separators until you
              choose.
            </p>
          </section>
          <section className="flex flex-col gap-1.5" aria-labelledby="settings-palette">
            <h2 id="settings-palette" className="text-sm font-medium">
              Company palette
            </h2>
            <p className="text-xs text-muted-foreground">
              {COMPANY_PALETTE_MIN} to {COMPANY_PALETTE_MAX} colours. With a palette set, new
              charts use it.
            </p>
            <ul className="flex flex-wrap gap-2" aria-label="Company palette colours">
              {palette.map((colour) => (
                <li key={colour} className="flex items-center gap-1">
                  <span
                    className="size-6 rounded border"
                    style={{ backgroundColor: colour }}
                    role="img"
                    aria-label={colour}
                  />
                  {canWrite ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Remove ${colour}`}
                      onClick={() => setPalette(palette.filter((c) => c !== colour))}
                    >
                      Remove
                    </Button>
                  ) : null}
                </li>
              ))}
            </ul>
            {canWrite ? (
              <div className="flex gap-2">
                <Input
                  aria-label="Palette colour"
                  value={hex}
                  onChange={(e) => setHex(e.target.value)}
                  placeholder="#1a2b3c"
                  autoComplete="off"
                />
                <Button
                  variant="outline"
                  onClick={addColour}
                  disabled={!isHexColor(hex.trim()) || palette.length >= COMPANY_PALETTE_MAX}
                >
                  Add colour
                </Button>
              </div>
            ) : null}
            {paletteInvalid ? (
              <p role="alert" className="text-xs text-destructive">
                A palette needs at least {COMPANY_PALETTE_MIN} colours, or none.
              </p>
            ) : null}
          </section>
          <section className="flex flex-col gap-1.5" aria-labelledby="settings-fonts">
            <h2 id="settings-fonts" className="text-sm font-medium">
              Fonts
            </h2>
            <p className="text-xs text-muted-foreground">
              The fonts a dashboard may use. All of them ship with the console.
            </p>
            {Object.entries(SHIPPED_FONTS).map(([id, label]) => (
              <div key={id} className="flex items-center gap-2">
                <Checkbox
                  id={`font-${id}`}
                  checked={fonts.includes(id)}
                  onCheckedChange={(on) => toggleFont(id, on === true)}
                  disabled={!canWrite}
                />
                <Label htmlFor={`font-${id}`}>{label}</Label>
              </div>
            ))}
          </section>
          <section className="flex flex-col gap-1.5" aria-labelledby="settings-content-sites">
            <h2 id="settings-content-sites" className="text-sm font-medium">
              Allowed content sites
            </h2>
            <p className="text-xs text-muted-foreground">
              Image and web-page cards may only show addresses on these sites, written
              https://host or https://host:port, no wildcards. At most {MAX_CONTENT_SITES}.
              With none listed, no such card can be saved.
            </p>
            <ul className="flex flex-col gap-1" aria-label="Allowed content sites">
              {sites.map((s) => (
                <li key={s} className="flex items-center gap-2 text-sm">
                  <span className="font-mono">{s}</span>
                  {canWrite ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Remove ${s}`}
                      onClick={() => setSites(sites.filter((x) => x !== s))}
                    >
                      Remove
                    </Button>
                  ) : null}
                </li>
              ))}
            </ul>
            {canWrite ? (
              <div className="flex gap-2">
                <Input
                  aria-label="Content site"
                  value={site}
                  onChange={(e) => setSite(e.target.value)}
                  placeholder="https://images.example.com"
                  autoComplete="off"
                />
                <Button
                  variant="outline"
                  onClick={addSite}
                  disabled={!siteValid || sites.length >= MAX_CONTENT_SITES}
                >
                  Add site
                </Button>
              </div>
            ) : null}
          </section>
          {canWrite ? (
            <Button
              className="w-fit"
              onClick={handleSave}
              disabled={!changed || paletteInvalid || save.status === "pending"}
            >
              Save
            </Button>
          ) : (
            <p className="text-sm text-muted-foreground">
              Only an administrator can change these.
            </p>
          )}
        </div>
      ) : null}
    </div>
  )
}
