"use client";

/**
 * The Details tab of the side panel (`BI-61` T6): what the ⋯ menu's dialogs
 * hold about this dashboard, at a glance, as calm lists: who can see it and
 * how it is shared, how fresh its data is, its subscriptions and its last
 * versions. Each section has one way into the dialog that changes it (one
 * action, one button), and nothing here writes on its own. Everything is read
 * through the services the dialogs already use; nothing new is asked of the
 * API.
 */
import * as React from "react";
import { ChevronRight, Clock, Code2, Globe, History, Mail, PencilLine, RefreshCw, UserPlus, Users, type LucideIcon } from "lucide-react";

import { useService } from "@/hooks/use-service";
import { formatDateTime, formatRelativeTime } from "@/lib/format";
import { ruleLabel, type CacheRule } from "@/lib/result-cache";
import { cn } from "@/lib/utils";
import { dashboardService, reportService, type Board } from "@/services";
import { EASE, FOCUS } from "./dashboard-header";
import { describeSchedule } from "./report-words";

/** One fact: an icon, its name and its value; a button when it opens the dialog that changes it. */
function Row({ icon: Icon, label, value, onClick, title }: {
  readonly icon: LucideIcon;
  readonly label: string;
  readonly value: React.ReactNode;
  readonly onClick?: () => void;
  /** The value in full, as its tooltip (a date behind "5h ago"). */
  readonly title?: string;
}) {
  const inner = (
    <>
      <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
      <span className="min-w-0 flex-1 truncate text-sm">{label}</span>
      <span title={title} className="min-w-0 max-w-[55%] truncate text-right text-sm text-muted-foreground tabular-nums">{value}</span>
      {onClick ? <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" aria-hidden /> : null}
    </>
  );
  return onClick ? (
    <button type="button" onClick={onClick} className={cn("-mx-2 flex items-center gap-2.5 rounded-lg px-2 py-1.5 text-left hover:bg-muted/60", EASE, FOCUS)}>{inner}</button>
  ) : <div className="flex items-center gap-2.5 py-1.5">{inner}</div>;
}

function Section({ title, action, children }: { readonly title: string; readonly action?: React.ReactNode; readonly children: React.ReactNode }) {
  return (
    <section className="grid gap-1">
      <div className="flex h-7 items-center justify-between gap-2">
        <h3 className="text-xs font-medium text-muted-foreground">{title}</h3>
        {action}
      </div>
      {children}
    </section>
  );
}

function SectionLink({ children, onClick }: { readonly children: React.ReactNode; readonly onClick: () => void }) {
  return (
    <button type="button" onClick={onClick}
      className={cn("rounded-md px-1.5 py-0.5 text-xs font-medium text-primary hover:bg-primary/10 dark:text-sky-300", EASE, FOCUS)}>
      {children}
    </button>
  );
}

const Quiet = ({ children }: { readonly children: React.ReactNode }) => <p className="py-1.5 text-sm text-muted-foreground">{children}</p>;

export function DetailsPanel({
  board, listed, canManage, canShare, canWrite, canDownload, dataFrom, autoRefresh, cacheRule,
  onShare, onFreshness, onSubscriptions, onHistory,
}: {
  readonly board: string;
  /** The dashboard as the list has it (its public link and embed); absent while the list loads. */
  readonly listed?: Board;
  readonly canManage: boolean;
  /** Share is offered (it is not on a dashboard in a personal space, R11). */
  readonly canShare: boolean;
  readonly canWrite: boolean;
  /** Subscriptions follow the download switch, as the ⋯ menu's item does. */
  readonly canDownload: boolean;
  readonly dataFrom?: string | null;
  /** "Off" or "Every 5m". */
  readonly autoRefresh: string;
  /** The freshness rule in force (`BI-4` part B). */
  readonly cacheRule?: CacheRule | null;
  readonly onShare: () => void;
  readonly onFreshness: () => void;
  readonly onSubscriptions: () => void;
  readonly onHistory: () => void;
}) {
  // Who has what needs manage on the dashboard; anyone else is not asked for it.
  const access = useService(
    (signal) => (canManage ? dashboardService.getAccess("board", board, signal) : Promise.resolve(null)),
    [board, canManage],
  );
  const subs = useService(
    (signal) => (canDownload ? reportService.list(board, signal) : Promise.resolve(null)),
    [board, canDownload],
  );
  const versions = useService((signal) => dashboardService.listVersions(board, signal), [board]);

  const accessValue = (pick: (a: NonNullable<typeof access.data>) => string) =>
    access.status === "loading" ? "…" : access.status === "error" || !access.data ? "Not available" : pick(access.data);

  return (
    <div className="grid min-h-0 grid-cols-1 content-start gap-6 overflow-y-auto p-4" data-slot="details-panel">
      <Section title="Sharing" action={canShare ? <SectionLink onClick={onShare}>Share…</SectionLink> : undefined}>
        {canManage ? (
          <>
            <Row icon={Users} label="Who can open it" value={accessValue((a) => (a.restricted ? "Only the people listed" : "Follows its folder"))} />
            <Row icon={UserPlus} label="Added here" value={accessValue((a) => `${a.rules.length.toLocaleString()} ${a.rules.length === 1 ? "person or role" : "people and roles"}`)} />
          </>
        ) : null}
        <Row icon={Globe} label="Public link" value={listed ? (listed.publicToken ? "On" : "Off") : "…"} />
        <Row icon={Code2} label="Embed" value={listed ? (listed.embedEnabled ? "On" : "Off") : "…"} />
      </Section>

      <Section title="Data">
        <Row icon={Clock} label="Data from" value={dataFrom ? dataFrom.replace(/^Data from /, "") : "—"} />
        <Row icon={RefreshCw} label="Auto-refresh" value={autoRefresh} />
        <Row icon={History} label="Freshness" value={ruleLabel(cacheRule)} onClick={canWrite ? onFreshness : undefined} />
      </Section>

      {canDownload ? (
        <Section title="Subscriptions" action={<SectionLink onClick={onSubscriptions}>Manage…</SectionLink>}>
          {subs.status === "loading" ? <Quiet>Loading…</Quiet> : null}
          {subs.status === "error" ? <Quiet>{subs.error.message}</Quiet> : null}
          {subs.status === "success" && subs.data && subs.data.items.length === 0 ? <Quiet>No subscriptions yet.</Quiet> : null}
          {subs.status === "success" && subs.data
            ? subs.data.items.slice(0, 4).map((s) => {
                const who = s.recipients.map((r) => r.name ?? r.email ?? "").filter(Boolean);
                return (
                  <Row key={s.id} icon={Mail} label={who.length ? (who.length > 1 ? `${who[0]} and ${who.length - 1} more` : who[0]) : "No recipients"}
                    value={s.paused ? "Paused" : describeSchedule(s.schedule, s.timeZone).replace(/ \([^)]*\)$/, "")} />
                );
              })
            : null}
          {subs.status === "success" && subs.data && subs.data.items.length > 4 ? <Quiet>{subs.data.items.length - 4} more</Quiet> : null}
        </Section>
      ) : null}

      <Section title="Version history" action={<SectionLink onClick={onHistory}>All versions…</SectionLink>}>
        {/* BI-61 review round 1, SHOULD-FIX 11 (owner): the last edit lives here, not beside "Data from"
            in the header; who made it is the newest version's author, when the history has one. */}
        {listed?.updatedAt ? (
          <Row icon={PencilLine} label="Edited" title={formatDateTime(listed.updatedAt)}
            value={`${formatRelativeTime(listed.updatedAt)}${versions.status === "success" && versions.data.versions[0]?.author ? ` by ${versions.data.versions[0].author}` : ""}`} />
        ) : null}
        {versions.status === "loading" ? <Quiet>Loading…</Quiet> : null}
        {versions.status === "error" ? <Quiet>{versions.error.message}</Quiet> : null}
        {versions.status === "success" && versions.data.versions.length === 0 ? <Quiet>No versions yet. The next change is the first.</Quiet> : null}
        {versions.status === "success" ? (
          <ol className="grid gap-0.5">
            {versions.data.versions.slice(0, 5).map((v, i) => (
              <li key={v.version} className="flex items-start gap-2.5 py-1.5">
                <span aria-hidden className={cn("mt-1.5 size-1.5 shrink-0 rounded-full", i === 0 ? "bg-primary" : "bg-muted-foreground/40")} />
                <div className="grid min-w-0 flex-1 gap-0.5">
                  <p className="truncate text-sm">{v.summary || `Version ${v.version}`}</p>
                  <p className="truncate text-xs text-muted-foreground">{v.author} · {v.at}</p>
                </div>
                {i === 0 ? <span className="shrink-0 text-xs text-muted-foreground">Current</span> : null}
              </li>
            ))}
          </ol>
        ) : null}
      </Section>
    </div>
  );
}
