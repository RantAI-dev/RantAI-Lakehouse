/**
 * `GET`/`PUT /api/settings/reporting` (`BI-9`): the report time zone and the
 * first day of the week, one pair for the whole deployment. Grouping a
 * timestamp by day or month, and the relative date filters ("this month"),
 * both read it, so a chart and a filter agree about where a month begins.
 *
 * `BI-17` part A adds the number locale, the company palette and the allowed
 * fonts, `BI-16` part B the allowed content sites, and `BI-1` the roles that
 * may not download, saved on the same screen with the same permission.
 *
 * Produced by `routes::settings` in `lakehouse-api`. Reading needs only a
 * sign-in; saving needs `settings:write`.
 */
export type ReportingSettings = {
  /** An IANA zone name the engine knows, such as `Asia/Jakarta`. */
  timeZone: string
  /** `"monday"` or `"sunday"`; widened in case a later server adds one. */
  weekStart: "monday" | "sunday" | string
  /** False while these are the defaults and nothing was ever saved. */
  saved: boolean
  /** `"en-US"` or `"id-ID"`: the separators every number uses. */
  numberLocale: "en-US" | "id-ID" | string
  /** Empty, or 2 to 12 `#rrggbb` colours that become the default palette. */
  companyPalette: string[]
  /** The font ids dashboards may use. */
  fonts: string[]
  /**
   * Sites image and web cards may load from, `https://host` or
   * `https://host:port`, at most 50 (`BI-16` part B). Empty: no such card can
   * be saved.
   */
  contentSites: string[]
  /**
   * Names of the roles that may not download (`BI-1`). A person whose every
   * role is on the list gets no download items; a Platform Admin always may.
   * Empty: everyone may.
   */
  downloadDeniedRoles: string[]
  /**
   * Outside email domains a scheduled report may go to (`RPT-1`), lower case,
   * at most 50, no wildcards. Empty: no report leaves the company.
   */
  reportAllowedDomains: string[]
  /**
   * Whether public links work (`BI-19` part A). On at first; off stops every
   * link at once, and switching it on again brings back the links that have
   * not expired.
   */
  publicLinksEnabled: boolean
}

/**
 * What a save sends. The formatting fields are optional: one left out keeps
 * the value stored, so a caller that only changes the zone is unaffected.
 */
export type ReportingSettingsInput = Pick<ReportingSettings, "timeZone" | "weekStart"> &
  Partial<Pick<ReportingSettings, "numberLocale" | "companyPalette" | "fonts" | "contentSites" | "downloadDeniedRoles" | "reportAllowedDomains" | "publicLinksEnabled">>

export interface SettingsService {
  getReporting(signal?: AbortSignal): Promise<ReportingSettings>
  /** The server refuses an unknown zone or a third first day with a 400. */
  saveReporting(
    input: ReportingSettingsInput,
    signal?: AbortSignal
  ): Promise<ReportingSettings>
}
