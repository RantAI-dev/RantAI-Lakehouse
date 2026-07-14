"use client"

import { Plus } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { cn } from "@/lib/utils"

const TAB_CLASSIFICATION = "classification"
const TAB_MASKING = "masking"
const TAB_RETENTION = "retention"
const TAB_ACCESS = "access"
const TAB_QUALITY = "quality"
const TAB_RLS = "rls"

const CLASSIFICATION_ROWS = [
  {
    id: "1",
    label: "Customer PII",
    level: "Restricted",
    tagged: "38 tables",
    owner: "Security",
    updated: "2 days ago",
  },
  {
    id: "2",
    label: "Financial records",
    level: "Confidential",
    tagged: "22 tables",
    owner: "Finance Ops",
    updated: "1 week ago",
  },
  {
    id: "3",
    label: "Operational metrics",
    level: "Internal",
    tagged: "64 tables",
    owner: "Data Platform",
    updated: "3 days ago",
  },
  {
    id: "4",
    label: "Public reference",
    level: "Public",
    tagged: "12 tables",
    owner: "Marketing",
    updated: "Apr 1, 2026",
  },
] as const

const MASKING_ROWS = [
  {
    id: "1",
    name: "Email partial mask",
    target: "bronze.customers.email",
    method: "Partial redact",
    status: "Active" as const,
  },
  {
    id: "2",
    name: "SSN tokenization",
    target: "raw.ingest.*.ssn",
    method: "Tokenize",
    status: "Active" as const,
  },
  {
    id: "3",
    name: "Phone hash",
    target: "silver.support_tickets.phone",
    method: "One-way hash",
    status: "Draft" as const,
  },
] as const

const RETENTION_ROWS = [
  {
    id: "1",
    policy: "Raw ingest logs",
    period: "90 days",
    appliesTo: "s3://lake-raw/logs/*",
    enforcement: "Active" as const,
  },
  {
    id: "2",
    policy: "Bronze snapshots",
    period: "18 months",
    appliesTo: "Zone: Bronze / finance_*",
    enforcement: "Active" as const,
  },
  {
    id: "3",
    policy: "Sandbox tables",
    period: "30 days",
    appliesTo: "Prefix: dev_sandbox.*",
    enforcement: "Warning" as const,
  },
] as const

const ACCESS_ROWS = [
  {
    id: "1",
    name: "Analysts read Gold",
    principals: "Group: analysts",
    resources: "Gold zone (read)",
    effect: "Allow" as const,
  },
  {
    id: "2",
    name: "Pipelines service account",
    principals: "SA: pipeline-runner",
    resources: "Raw → Bronze (write)",
    effect: "Allow" as const,
  },
  {
    id: "3",
    name: "External sharing block",
    principals: "All external IDs",
    resources: "Restricted classifications",
    effect: "Deny" as const,
  },
] as const

const RLS_ROWS = [
  {
    id: "1",
    name: "Orders: region scope",
    table: "gold.orders_fact",
    predicate: "region = current_user_region()",
    roles: "Analyst, Editor",
    status: "Active" as const,
  },
  {
    id: "2",
    name: "Support: own tickets",
    table: "silver.support_tickets",
    predicate: "assignee_id = current_user_id() OR team_id = current_team_id()",
    roles: "Support",
    status: "Active" as const,
  },
  {
    id: "3",
    name: "Finance: entity lock",
    table: "gold.gl_entries",
    predicate: "entity_code IN (SELECT entity FROM user_allowed_entities())",
    roles: "Finance",
    status: "Draft" as const,
  },
] as const

const QUALITY_ROWS = [
  {
    id: "1",
    rule: "Orders: amount non-null",
    dataset: "gold.orders_fact",
    lastRun: "15 min ago",
    result: "Pass" as const,
    score: "99.2%",
  },
  {
    id: "2",
    rule: "Customer key uniqueness",
    dataset: "silver.dim_customer",
    lastRun: "1 hour ago",
    result: "Pass" as const,
    score: "100%",
  },
  {
    id: "3",
    rule: "Inventory qty ≥ 0",
    dataset: "bronze.inventory_daily",
    lastRun: "6 hours ago",
    result: "Fail" as const,
    score: "94.8%",
  },
] as const

function GovernanceTableShell({
  title,
  description,
  actionLabel,
  children,
}: {
  title: string
  description: string
  actionLabel: string
  children: React.ReactNode
}) {
  return (
    <Card className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
      <CardHeader className="flex flex-col gap-3 border-b border-border p-4 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
        <div className="min-w-0 space-y-1">
          <h2 className="text-base font-medium leading-none tracking-[-0.096px] text-primary">
            {title}
          </h2>
          <p className="text-sm text-muted-foreground">{description}</p>
        </div>
        <Button
          type="button"
          size="default"
          className="h-10 shrink-0 gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground"
        >
          <Plus className="size-4" />
          {actionLabel}
        </Button>
      </CardHeader>
      <CardContent className="p-0">{children}</CardContent>
    </Card>
  )
}

function LevelBadge({ level }: { level: string }) {
  const isHigh =
    level === "Restricted" || level === "Confidential"
  return (
    <Badge
      className={cn(
        "text-xs font-semibold tracking-[-0.072px] border-0",
        isHigh
          ? "bg-destructive/10 text-destructive hover:bg-destructive/10"
          : level === "Public"
            ? "bg-[#ecfdf2] text-[#008a2e] hover:bg-[#ecfdf2]"
            : "bg-primary/10 text-primary hover:bg-primary/10"
      )}
    >
      {level}
    </Badge>
  )
}

function StatusBadge({
  status,
  tone,
}: {
  status: string
  tone: "success" | "neutral" | "warn" | "danger"
}) {
  const cls =
    tone === "success"
      ? "bg-[#ecfdf2] text-[#008a2e] border-0 hover:bg-[#ecfdf2]"
      : tone === "danger"
        ? "bg-destructive/10 text-destructive border-0"
        : tone === "warn"
          ? "bg-amber-500/15 text-amber-800 dark:text-amber-200 border-0"
          : "bg-muted text-muted-foreground border-0"
  return (
    <Badge className={cn("text-xs font-semibold tracking-[-0.072px]", cls)}>
      {status}
    </Badge>
  )
}

function EffectBadge({ effect }: { effect: "Allow" | "Deny" }) {
  return (
    <Badge
      className={cn(
        "text-xs font-semibold tracking-[-0.072px] border-0",
        effect === "Allow"
          ? "bg-[#ecfdf2] text-[#008a2e] hover:bg-[#ecfdf2]"
          : "bg-destructive/10 text-destructive"
      )}
    >
      {effect}
    </Badge>
  )
}

export default function DataGovernancePage() {
  return (
    <div className="flex flex-col gap-4">
      <header className="border-b border-border pb-2">
        <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
          Data Governance
        </h1>
        <p className="mt-1 text-sm leading-5 tracking-[-0.084px] text-muted-foreground">
          Classification, protection, retention, access, and quality controls for
          your lakehouse.
        </p>
      </header>

      <Tabs defaultValue={TAB_CLASSIFICATION} className="flex flex-col gap-4">
        <div className="-mx-1 overflow-x-auto px-1 pb-1">
          <TabsList className="h-auto min-h-9 w-max max-w-full flex-wrap gap-1 rounded-md bg-secondary p-1 sm:inline-flex sm:w-auto sm:flex-nowrap">
            <TabsTrigger
              value={TAB_CLASSIFICATION}
              className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
            >
              Data classification
            </TabsTrigger>
            <TabsTrigger
              value={TAB_MASKING}
              className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
            >
              Data masking
            </TabsTrigger>
            <TabsTrigger
              value={TAB_RETENTION}
              className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
            >
              Retention policies
            </TabsTrigger>
            <TabsTrigger
              value={TAB_ACCESS}
              className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
            >
              Access policies
            </TabsTrigger>
            <TabsTrigger
              value={TAB_QUALITY}
              className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
            >
              Data quality
            </TabsTrigger>
            <TabsTrigger
              value={TAB_RLS}
              className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
            >
              Row-level security
            </TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value={TAB_CLASSIFICATION} className="mt-0">
          <GovernanceTableShell
            title="Sensitivity labels"
            description="Tag tables and columns with classification levels for discovery and policy enforcement."
            actionLabel="Add classification"
          >
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-4">Label</TableHead>
                  <TableHead>Level</TableHead>
                  <TableHead>Tagged</TableHead>
                  <TableHead>Owner</TableHead>
                  <TableHead className="pr-4">Updated</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {CLASSIFICATION_ROWS.map((row) => (
                  <TableRow key={row.id}>
                    <TableCell className="pl-4 font-medium text-primary">
                      {row.label}
                    </TableCell>
                    <TableCell>
                      <LevelBadge level={row.level} />
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.tagged}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.owner}
                    </TableCell>
                    <TableCell className="pr-4 text-muted-foreground">
                      {row.updated}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </GovernanceTableShell>
        </TabsContent>

        <TabsContent value={TAB_MASKING} className="mt-0">
          <GovernanceTableShell
            title="Masking rules"
            description="Define how sensitive values are obscured in queries and exports."
            actionLabel="New masking rule"
          >
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-4">Rule</TableHead>
                  <TableHead>Target</TableHead>
                  <TableHead>Method</TableHead>
                  <TableHead className="pr-4">Status</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {MASKING_ROWS.map((row) => (
                  <TableRow key={row.id}>
                    <TableCell className="pl-4 font-medium text-primary">
                      {row.name}
                    </TableCell>
                    <TableCell className="max-w-[200px] truncate font-mono text-xs text-muted-foreground sm:max-w-none">
                      {row.target}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.method}
                    </TableCell>
                    <TableCell className="pr-4">
                      <StatusBadge
                        status={row.status}
                        tone={row.status === "Active" ? "success" : "neutral"}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </GovernanceTableShell>
        </TabsContent>

        <TabsContent value={TAB_RETENTION} className="mt-0">
          <GovernanceTableShell
            title="Retention schedules"
            description="Control how long data is kept per zone, bucket, or table pattern."
            actionLabel="Add policy"
          >
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-4">Policy</TableHead>
                  <TableHead>Period</TableHead>
                  <TableHead>Applies to</TableHead>
                  <TableHead className="pr-4">Enforcement</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {RETENTION_ROWS.map((row) => (
                  <TableRow key={row.id}>
                    <TableCell className="pl-4 font-medium text-primary">
                      {row.policy}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.period}
                    </TableCell>
                    <TableCell className="max-w-[220px] text-muted-foreground sm:max-w-none">
                      {row.appliesTo}
                    </TableCell>
                    <TableCell className="pr-4">
                      <StatusBadge
                        status={row.enforcement}
                        tone={
                          row.enforcement === "Active"
                            ? "success"
                            : "warn"
                        }
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </GovernanceTableShell>
        </TabsContent>

        <TabsContent value={TAB_ACCESS} className="mt-0">
          <GovernanceTableShell
            title="Access policies"
            description="Who can read or write which zones, tables, or classifications."
            actionLabel="New policy"
          >
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-4">Name</TableHead>
                  <TableHead>Principals</TableHead>
                  <TableHead>Resources</TableHead>
                  <TableHead className="pr-4">Effect</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {ACCESS_ROWS.map((row) => (
                  <TableRow key={row.id}>
                    <TableCell className="pl-4 font-medium text-primary">
                      {row.name}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.principals}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.resources}
                    </TableCell>
                    <TableCell className="pr-4">
                      <EffectBadge effect={row.effect} />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </GovernanceTableShell>
        </TabsContent>

        <TabsContent value={TAB_RLS} className="mt-0">
          <GovernanceTableShell
            title="Row-level security (RLS)"
            description="Predicates merged into queries so users only see rows allowed for their role."
            actionLabel="New RLS policy"
          >
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-4">Policy</TableHead>
                  <TableHead>Table</TableHead>
                  <TableHead className="min-w-[200px]">Predicate</TableHead>
                  <TableHead>Roles</TableHead>
                  <TableHead className="pr-4">Status</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {RLS_ROWS.map((row) => (
                  <TableRow key={row.id}>
                    <TableCell className="pl-4 font-medium text-primary">
                      {row.name}
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {row.table}
                    </TableCell>
                    <TableCell className="max-w-[280px] font-mono text-[11px] leading-snug text-muted-foreground sm:max-w-none">
                      {row.predicate}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.roles}
                    </TableCell>
                    <TableCell className="pr-4">
                      <StatusBadge
                        status={row.status}
                        tone={row.status === "Active" ? "success" : "neutral"}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </GovernanceTableShell>
        </TabsContent>

        <TabsContent value={TAB_QUALITY} className="mt-0">
          <GovernanceTableShell
            title="Quality checks"
            description="Scheduled validations and freshness expectations on curated datasets."
            actionLabel="Add check"
          >
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-4">Rule</TableHead>
                  <TableHead>Dataset</TableHead>
                  <TableHead>Last run</TableHead>
                  <TableHead>Score</TableHead>
                  <TableHead className="pr-4">Result</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {QUALITY_ROWS.map((row) => (
                  <TableRow key={row.id}>
                    <TableCell className="pl-4 font-medium text-primary">
                      {row.rule}
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {row.dataset}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.lastRun}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.score}
                    </TableCell>
                    <TableCell className="pr-4">
                      <StatusBadge
                        status={row.result}
                        tone={row.result === "Pass" ? "success" : "danger"}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </GovernanceTableShell>
        </TabsContent>
      </Tabs>
    </div>
  )
}
