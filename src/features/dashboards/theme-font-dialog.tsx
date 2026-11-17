"use client";

import * as React from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SHIPPED_FONTS } from "@/lib/formatting-settings";
import { isTheme, type DashboardTheme } from "@/lib/dashboard-look";

const THEME_LABELS: Record<DashboardTheme, string> = { auto: "Follow the console", light: "Light", dark: "Dark" };
/** The console's own font is the empty choice. */
const OWN = "__console__";

/**
 * "Theme and font…" of the dashboard's ⋯ menu (`BI-17` part A, D6): the
 * dashboard's tiles take this theme and font wherever it is shown, including a
 * public link and a signed embed (decision 13). Only the fonts an
 * administrator allows are offered (decision 12); the server refuses another.
 */
export function ThemeFontDialog({
  open, onOpenChange, theme, font, allowedFonts, saving, error, onSave,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** What the dashboard has now: absent is `auto` and the console's font. */
  readonly theme?: string;
  readonly font?: string;
  /** The font ids an administrator allows. */
  readonly allowedFonts: readonly string[];
  readonly saving?: boolean;
  /** The server's sentence when the last save was refused. */
  readonly error?: string | null;
  readonly onSave: (choice: { theme: DashboardTheme; font: string }) => void;
}) {
  const [t, setT] = React.useState<DashboardTheme>(isTheme(theme) ? theme : "auto");
  const [f, setF] = React.useState(font ?? "");
  const fontItems: Record<string, string> = {
    [OWN]: "The console's font",
    ...Object.fromEntries(Object.entries(SHIPPED_FONTS).filter(([id]) => allowedFonts.includes(id))),
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader><DialogTitle>Theme and font</DialogTitle></DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label>Theme</Label>
            <Select value={t} items={THEME_LABELS} onValueChange={(v) => { if (isTheme(v)) setT(v); }}>
              <SelectTrigger className="w-full" aria-label="Theme"><SelectValue /></SelectTrigger>
              <SelectContent>
                {Object.entries(THEME_LABELS).map(([v, label]) => <SelectItem key={v} value={v}>{label}</SelectItem>)}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-1.5">
            <Label>Font</Label>
            <Select value={f || OWN} items={fontItems} onValueChange={(v) => setF(v && v !== OWN ? v : "")}>
              <SelectTrigger className="w-full" aria-label="Font"><SelectValue /></SelectTrigger>
              <SelectContent>
                {Object.entries(fontItems).map(([v, label]) => <SelectItem key={v} value={v}>{label}</SelectItem>)}
              </SelectContent>
            </Select>
          </div>
          <p className="text-xs text-muted-foreground">Applies to this dashboard&apos;s tiles everywhere it is shown, including public links and embeds.</p>
          {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" disabled={saving} onClick={() => onSave({ theme: t, font: f })}>{saving ? "Saving…" : "Save"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
