"use client"

import { useCallback, useMemo, useState } from "react"
import {
  Boxes,
  Calendar,
  Database,
  Gauge,
  Globe,
  HardDrive,
  Mail,
  Plus,
  Search,
  Shield,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Separator } from "@/components/ui/separator"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { cn } from "@/lib/utils"

type TenantStatus = "Active" | "Trial" | "Suspended"

type Tenant = {
  id: string
  name: string
  slug: string
  users: number
  region: string
  status: TenantStatus
  plan: string
  catalogName: string
  storagePrefix: string
  createdAt: string
  contactEmail: string
  description: string
  mfaRequired: boolean
  dataResidency: string
  storageUsedGb: number
  storageQuotaGb: number
  dailyQueryLimit: number
  dailyQueryUsage: number
  ipAllowlist: string
  tags: string[]
}

const SAMPLE_TENANTS: Tenant[] = [
  {
    id: "1",
    name: "Nexus Retail",
    slug: "nexus-retail",
    users: 42,
    region: "ap-southeast-1",
    status: "Active",
    plan: "Enterprise",
    catalogName: "prod_nexus_retail",
    storagePrefix: "s3://rantai-lake/nexus-retail/",
    createdAt: "2024-06-12",
    contactEmail: "platform@nexusretail.example",
    description:
      "Primary production tenant for retail analytics, gold/silver pipelines, and semantic search.",
    mfaRequired: true,
    dataResidency: "Singapore (SG)",
    storageUsedGb: 1840,
    storageQuotaGb: 5000,
    dailyQueryLimit: 500_000,
    dailyQueryUsage: 128_400,
    ipAllowlist: "10.0.0.0/8, 172.16.0.0/12 (VPN)",
    tags: ["production", "pci-scope"],
  },
  {
    id: "2",
    name: "Fintra Bank",
    slug: "fintra",
    users: 128,
    region: "eu-west-1",
    status: "Active",
    plan: "Enterprise",
    catalogName: "prod_fintra",
    storagePrefix: "s3://rantai-lake/fintra/",
    createdAt: "2023-11-02",
    contactEmail: "data-office@fintra.example",
    description:
      "Regulated workloads with EU residency; separate vector index for internal documents.",
    mfaRequired: true,
    dataResidency: "Ireland (EU)",
    storageUsedGb: 9200,
    storageQuotaGb: 12000,
    dailyQueryLimit: 1_000_000,
    dailyQueryUsage: 412_000,
    ipAllowlist: "Office ranges + Azure VPN",
    tags: ["regulated", "eu-only"],
  },
  {
    id: "3",
    name: "Pilot Corp",
    slug: "pilot",
    users: 8,
    region: "us-east-1",
    status: "Trial",
    plan: "Trial",
    catalogName: "trial_pilot",
    storagePrefix: "s3://rantai-lake-trial/pilot/",
    createdAt: "2026-03-01",
    contactEmail: "cto@pilot.example",
    description:
      "Evaluation tenant for warehouse modernization; limited quotas and no production data.",
    mfaRequired: false,
    dataResidency: "US East (N. Virginia)",
    storageUsedGb: 42,
    storageQuotaGb: 200,
    dailyQueryLimit: 20_000,
    dailyQueryUsage: 3_100,
    ipAllowlist: "Any (trial)",
    tags: ["trial", "eval"],
  },
  {
    id: "4",
    name: "Legacy Holdings",
    slug: "legacy",
    users: 16,
    region: "ap-southeast-2",
    status: "Suspended",
    plan: "Standard",
    catalogName: "prod_legacy_holdings",
    storagePrefix: "s3://rantai-lake/legacy/",
    createdAt: "2024-01-20",
    contactEmail: "admin@legacy.example",
    description:
      "Suspended for contract renewal; read-only exports available on request.",
    mfaRequired: true,
    dataResidency: "Sydney (AU)",
    storageUsedGb: 310,
    storageQuotaGb: 2000,
    dailyQueryLimit: 100_000,
    dailyQueryUsage: 0,
    ipAllowlist: "Suspended — blocked",
    tags: ["suspended", "renewal"],
  },
]

function StatusBadge({ status }: { status: TenantStatus }) {
  const cls =
    status === "Active"
      ? "bg-[#ecfdf2] text-[#008a2e] border-0 hover:bg-[#ecfdf2]"
      : status === "Trial"
        ? "bg-primary/10 text-primary border-0"
        : "bg-destructive/10 text-destructive border-0"
  return (
    <Badge className={cn("text-xs font-semibold tracking-[-0.072px]", cls)}>
      {status}
    </Badge>
  )
}

function storagePct(used: number, quota: number) {
  if (quota <= 0) return 0
  return Math.min(100, Math.round((used / quota) * 100))
}

function queryPct(used: number, limit: number) {
  if (limit <= 0) return 0
  return Math.min(100, Math.round((used / limit) * 100))
}

export default function TenantManagementPage() {
  const [query, setQuery] = useState("")
  const [detailOpen, setDetailOpen] = useState(false)
  const [activeTenant, setActiveTenant] = useState<Tenant | null>(null)

  const openDetail = useCallback((t: Tenant) => {
    setActiveTenant(t)
    setDetailOpen(true)
  }, [])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return SAMPLE_TENANTS
    return SAMPLE_TENANTS.filter(
      (t) =>
        t.name.toLowerCase().includes(q) ||
        t.slug.toLowerCase().includes(q) ||
        t.region.toLowerCase().includes(q) ||
        t.catalogName.toLowerCase().includes(q)
    )
  }, [query])

  const summary = useMemo(() => {
    const tenants = SAMPLE_TENANTS
    return {
      total: tenants.length,
      active: tenants.filter((t) => t.status === "Active").length,
      trial: tenants.filter((t) => t.status === "Trial").length,
      suspended: tenants.filter((t) => t.status === "Suspended").length,
    }
  }, [])

  return (
    <div className="flex flex-col gap-4">
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-border pb-2">
        <div className="flex flex-wrap items-center gap-2">
          <Boxes className="size-7 text-primary" aria-hidden />
          <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
            Tenant management
          </h1>
        </div>
        <Button
          type="button"
          className="h-10 gap-2 bg-primary text-primary-foreground"
        >
          <Plus className="size-4" aria-hidden />
          Add tenant
        </Button>
      </header>
      <p className="-mt-2 text-sm text-muted-foreground">
        Isolate catalogs, storage prefixes, and quotas per organization. Select
        a tenant to view plan, residency, and usage.
      </p>

      <section className="inline-flex w-fit max-w-full flex-wrap gap-2 self-start rounded-md bg-accent p-2">
        <Card className="w-[152px] shrink-0 gap-0 overflow-hidden rounded-lg border border-border bg-card shadow-sm">
          <CardHeader className="space-y-0 px-3 pt-3 pb-1.5">
            <p className="text-sm font-medium leading-5 text-primary">Tenants</p>
          </CardHeader>
          <CardContent className="px-3 pb-4 pt-1.5">
            <p className="text-[24px] font-medium leading-none text-muted-foreground">
              {summary.total}
            </p>
          </CardContent>
        </Card>
        <Card className="w-[152px] shrink-0 gap-0 overflow-hidden rounded-lg border border-border bg-card shadow-sm">
          <CardHeader className="space-y-0 px-3 pt-3 pb-1.5">
            <p className="text-sm font-medium leading-5 text-primary">Active</p>
          </CardHeader>
          <CardContent className="px-3 pb-4 pt-1.5">
            <p className="text-[24px] font-medium leading-none text-muted-foreground">
              {summary.active}
            </p>
          </CardContent>
        </Card>
        <Card className="w-[152px] shrink-0 gap-0 overflow-hidden rounded-lg border border-border bg-card shadow-sm">
          <CardHeader className="space-y-0 px-3 pt-3 pb-1.5">
            <p className="text-sm font-medium leading-5 text-primary">Trial</p>
          </CardHeader>
          <CardContent className="px-3 pb-4 pt-1.5">
            <p className="text-[24px] font-medium leading-none text-muted-foreground">
              {summary.trial}
            </p>
          </CardContent>
        </Card>
        <Card className="w-[152px] shrink-0 gap-0 overflow-hidden rounded-lg border border-border bg-card shadow-sm">
          <CardHeader className="space-y-0 px-3 pt-3 pb-1.5">
            <p className="text-sm font-medium leading-5 text-primary">
              Suspended
            </p>
          </CardHeader>
          <CardContent className="px-3 pb-4 pt-1.5">
            <p className="text-[24px] font-medium leading-none text-muted-foreground">
              {summary.suspended}
            </p>
          </CardContent>
        </Card>
      </section>

      <div className="relative max-w-md">
        <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-primary" />
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search by name, slug, region, or catalog…"
          className="h-10 rounded-md border-border pl-9"
        />
      </div>

      <div className="overflow-hidden rounded-lg border border-border bg-card shadow-sm">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-4">Tenant</TableHead>
              <TableHead>Slug</TableHead>
              <TableHead className="hidden md:table-cell">Catalog</TableHead>
              <TableHead>Users</TableHead>
              <TableHead className="hidden sm:table-cell">Region</TableHead>
              <TableHead>Plan</TableHead>
              <TableHead className="pr-4">Status</TableHead>
              <TableHead className="w-[100px] pr-4 text-right">Detail</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {filtered.map((t) => (
              <TableRow key={t.id} className="group">
                <TableCell className="pl-4 font-medium text-primary">
                  <button
                    type="button"
                    onClick={() => openDetail(t)}
                    className="text-left underline-offset-2 hover:underline"
                  >
                    {t.name}
                  </button>
                </TableCell>
                <TableCell className="font-mono text-xs text-muted-foreground">
                  {t.slug}
                </TableCell>
                <TableCell className="hidden max-w-[180px] truncate font-mono text-xs text-muted-foreground md:table-cell">
                  {t.catalogName}
                </TableCell>
                <TableCell className="text-muted-foreground">{t.users}</TableCell>
                <TableCell className="hidden font-mono text-xs text-muted-foreground sm:table-cell">
                  {t.region}
                </TableCell>
                <TableCell>
                  <Badge variant="outline" className="text-[11px] font-normal">
                    {t.plan}
                  </Badge>
                </TableCell>
                <TableCell className="pr-4">
                  <StatusBadge status={t.status} />
                </TableCell>
                <TableCell className="pr-4 text-right">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="text-primary"
                    onClick={() => openDetail(t)}
                  >
                    View
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>

      <Sheet open={detailOpen} onOpenChange={setDetailOpen}>
        <SheetContent
          side="right"
          className="flex h-full max-h-[100dvh] w-full flex-col gap-0 overflow-hidden p-0 sm:max-w-lg"
        >
          {activeTenant && (
            <>
              <SheetHeader className="shrink-0 space-y-3 border-b border-border bg-muted/25 px-5 pb-4 pt-5 text-left">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0 space-y-1">
                    <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                      Tenant
                    </p>
                    <SheetTitle className="text-xl font-semibold tracking-tight">
                      {activeTenant.name}
                    </SheetTitle>
                    <SheetDescription className="font-mono text-xs text-muted-foreground">
                      {activeTenant.slug}
                    </SheetDescription>
                  </div>
                  <StatusBadge status={activeTenant.status} />
                </div>
                <p className="text-sm leading-relaxed text-foreground">
                  {activeTenant.description}
                </p>
                <div className="flex flex-wrap gap-1.5">
                  {activeTenant.tags.map((tag) => (
                    <Badge
                      key={tag}
                      variant="outline"
                      className="text-[11px] font-normal text-muted-foreground"
                    >
                      {tag}
                    </Badge>
                  ))}
                </div>
              </SheetHeader>

              <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
                <section className="space-y-2">
                  <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Identity &amp; routing
                  </h3>
                  <div className="grid gap-px overflow-hidden rounded-lg border border-border bg-border sm:grid-cols-2">
                    {[
                      {
                        icon: Database,
                        label: "Lake catalog",
                        value: activeTenant.catalogName,
                        mono: true,
                      },
                      {
                        icon: Globe,
                        label: "Primary region",
                        value: activeTenant.region,
                        mono: true,
                      },
                      {
                        icon: Shield,
                        label: "Data residency",
                        value: activeTenant.dataResidency,
                        mono: false,
                      },
                      {
                        icon: Calendar,
                        label: "Created",
                        value: activeTenant.createdAt,
                        mono: false,
                      },
                    ].map((row) => (
                      <div
                        key={row.label}
                        className="flex gap-2 bg-card px-3 py-2.5 sm:col-span-1"
                      >
                        <row.icon className="mt-0.5 size-4 shrink-0 text-primary" />
                        <div className="min-w-0">
                          <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                            {row.label}
                          </p>
                          <p
                            className={cn(
                              "mt-0.5 text-sm font-medium leading-snug text-foreground",
                              row.mono && "font-mono text-xs"
                            )}
                          >
                            {row.value}
                          </p>
                        </div>
                      </div>
                    ))}
                  </div>
                </section>

                <Separator className="my-5" />

                <section className="space-y-2">
                  <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Storage &amp; access
                  </h3>
                  <div className="rounded-lg border border-border bg-muted/15 p-3">
                    <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                      Object prefix
                    </p>
                    <p className="mt-1 break-all font-mono text-xs leading-relaxed text-foreground">
                      {activeTenant.storagePrefix}
                    </p>
                  </div>
                  <div className="mt-3 space-y-2">
                    <div className="flex items-center justify-between gap-2 text-xs">
                      <span className="flex items-center gap-1.5 text-muted-foreground">
                        <HardDrive className="size-3.5" />
                        Storage ({activeTenant.storageUsedGb.toLocaleString()} /{" "}
                        {activeTenant.storageQuotaGb.toLocaleString()} GB)
                      </span>
                      <span className="tabular-nums font-medium text-foreground">
                        {storagePct(
                          activeTenant.storageUsedGb,
                          activeTenant.storageQuotaGb
                        )}
                        %
                      </span>
                    </div>
                    <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
                      <div
                        className="h-full rounded-full bg-primary/80"
                        style={{
                          width: `${storagePct(
                            activeTenant.storageUsedGb,
                            activeTenant.storageQuotaGb
                          )}%`,
                        }}
                      />
                    </div>
                  </div>
                  <div className="mt-4 space-y-2">
                    <div className="flex items-center justify-between gap-2 text-xs">
                      <span className="flex items-center gap-1.5 text-muted-foreground">
                        <Gauge className="size-3.5" />
                        Query volume (24h)
                      </span>
                      <span className="tabular-nums font-medium text-foreground">
                        {activeTenant.dailyQueryUsage.toLocaleString()} /{" "}
                        {activeTenant.dailyQueryLimit.toLocaleString()}
                      </span>
                    </div>
                    <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
                      <div
                        className="h-full rounded-full bg-primary/60"
                        style={{
                          width: `${queryPct(
                            activeTenant.dailyQueryUsage,
                            activeTenant.dailyQueryLimit
                          )}%`,
                        }}
                      />
                    </div>
                  </div>
                  <div className="mt-3 rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                    <span className="font-medium text-foreground">
                      Network:{" "}
                    </span>
                    {activeTenant.ipAllowlist}
                  </div>
                </section>

                <Separator className="my-5" />

                <section className="space-y-3">
                  <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Plan &amp; security
                  </h3>
                  <dl className="grid gap-3 text-sm sm:grid-cols-2">
                    <div>
                      <dt className="text-xs text-muted-foreground">Plan</dt>
                      <dd className="mt-0.5 font-medium">{activeTenant.plan}</dd>
                    </div>
                    <div>
                      <dt className="text-xs text-muted-foreground">
                        MFA required
                      </dt>
                      <dd className="mt-0.5 font-medium">
                        {activeTenant.mfaRequired ? "Yes" : "No"}
                      </dd>
                    </div>
                    <div className="sm:col-span-2">
                      <dt className="flex items-center gap-1.5 text-xs text-muted-foreground">
                        <Mail className="size-3.5" />
                        Billing / platform contact
                      </dt>
                      <dd className="mt-0.5 font-mono text-xs text-foreground">
                        {activeTenant.contactEmail}
                      </dd>
                    </div>
                  </dl>
                </section>
              </div>

              <SheetFooter className="shrink-0 gap-2 border-t border-border bg-card/90 px-5 py-4 backdrop-blur-sm sm:flex-row sm:justify-end">
                <Button type="button" variant="outline" size="sm">
                  Edit quotas
                </Button>
                <Button type="button" size="sm" className="bg-primary">
                  Open audit for tenant
                </Button>
              </SheetFooter>
            </>
          )}
        </SheetContent>
      </Sheet>
    </div>
  )
}
