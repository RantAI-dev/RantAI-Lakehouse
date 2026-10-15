"use client";

import * as React from "react";
import { DEFAULT_NUMBER_LOCALE, SHIPPED_FONT_IDS } from "@/lib/formatting-settings";
import type { ReportingContext as Reporting } from "@/lib/time-grain";

/**
 * The deployment's report time zone and first day of the week as the server
 * used them for the data on screen (`reporting` in the dashboard, embed and
 * public payloads, BI-9). Defaults match the server's own, for a view that
 * is drawn before its payload arrives.
 */
const DEFAULT_REPORTING: Reporting = {
  timeZone: "Asia/Jakarta", weekStart: "monday",
  numberLocale: DEFAULT_NUMBER_LOCALE, companyPalette: [], fonts: [...SHIPPED_FONT_IDS],
};

export const ReportingContext = React.createContext<Reporting>(DEFAULT_REPORTING);

export function useReporting(): Reporting {
  return React.useContext(ReportingContext);
}

/** Provide a payload's `reporting`, falling back to the defaults when an older server sent none. */
export function ReportingProvider({
  reporting, children,
}: {
  readonly reporting: Reporting | undefined;
  readonly children: React.ReactNode;
}) {
  const timeZone = reporting?.timeZone;
  const weekStart = reporting?.weekStart;
  const numberLocale = reporting?.numberLocale ?? DEFAULT_NUMBER_LOCALE;
  // The palette and fonts are arrays; their joined text is the stable key.
  const paletteKey = (reporting?.companyPalette ?? []).join(",");
  const fontsKey = (reporting?.fonts ?? SHIPPED_FONT_IDS).join(",");
  // Rebuilt from the fields, not the object: the payload hands a new
  // object on every fetch, which would re-render every tile for nothing.
  const value = React.useMemo(
    () =>
      timeZone && weekStart
        ? {
            timeZone, weekStart, numberLocale,
            companyPalette: paletteKey ? paletteKey.split(",") : [],
            fonts: fontsKey ? fontsKey.split(",") : [],
          }
        : DEFAULT_REPORTING,
    [timeZone, weekStart, numberLocale, paletteKey, fontsKey],
  );
  return <ReportingContext.Provider value={value}>{children}</ReportingContext.Provider>;
}
