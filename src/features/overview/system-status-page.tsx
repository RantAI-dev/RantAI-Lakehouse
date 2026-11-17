"use client"

import * as React from "react"
import Link from "next/link"
import { RefreshCw } from "lucide-react"

import { PageHeader } from "@/components/patterns/page-header"
import { Button } from "@/components/ui/button"
import { ReplicationSlotHealth } from "@/features/governance/replication-slot-health"
import { useService } from "@/hooks/use-service"
import { opsService } from "@/services"
import { FreshnessStrip } from "./freshness-strip"
import {
  MaintenanceSection,
  QueriesSection,
  ServicesSection,
  StatusSummary,
  StorageSection,
} from "./system-status-sections"

/**
 * The sections a reader can jump to. Their ids are the anchors the old
 * pages redirect to (`/services` → `/health#services`, …), so renaming one
 * breaks a bookmark: change the redirect with it.
 */
const SECTIONS = [
  { id: "services", label: "Services" },
  { id: "storage", label: "Storage" },
  { id: "change-capture", label: "Change capture" },
  { id: "maintenance", label: "Maintenance" },
  { id: "freshness", label: "Freshness and queries" },
] as const

function Section({ id, children }: { id: string; children: React.ReactNode }) {
  // `scroll-mt` keeps an anchored section clear of the sticky top bar.
  return (
    <section id={id} className="scroll-mt-20">
      {children}
    </section>
  )
}

/**
 * System status (`WEB-3` S2-D2): "is the system healthy?" on one page.
 *
 * It replaces Health, Services, Capacity, Ingestion (CDC) and Table
 * Maintenance, five read-only pages that each answered part of the
 * question (Health repeated parts of Services and Capacity as cards). Each
 * section reads the same API its old page read; there is no new endpoint
 * (S2-D6). Each loads on its own, so one that fails shows its own error
 * with Retry and the rest of the page still renders (S2-D4).
 *
 * The services list is loaded here, once, because two sections use it:
 * the one-line state at the top and the Services table.
 */
export function SystemStatusPage() {
  const [refreshKey, setRefreshKey] = React.useState(0)
  const services = useService((s) => opsService.listServices(s), [refreshKey])
  const refresh = () => setRefreshKey((k) => k + 1)

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="System status"
        description="Whether the platform is up, what it stores, and what its background jobs last did."
        actions={
          <Button
            variant="outline"
            size="sm"
            onClick={refresh}
            disabled={services.status === "loading"}
          >
            <RefreshCw
              className={services.status === "loading" ? "size-4 animate-spin" : "size-4"}
              aria-hidden
            />
            Refresh
          </Button>
        }
      />

      <StatusSummary services={services} refreshKey={refreshKey} />

      <nav
        aria-label="Sections"
        className="-my-1 flex items-center gap-1 overflow-x-auto py-1"
      >
        {SECTIONS.map((section) => (
          <Button
            key={section.id}
            size="sm"
            variant="ghost"
            className="shrink-0 text-muted-foreground"
            render={<Link href={`#${section.id}`} />}
          >
            {section.label}
          </Button>
        ))}
      </nav>

      <Section id="services">
        <ServicesSection services={services} />
      </Section>

      <Section id="storage">
        <StorageSection refreshKey={refreshKey} />
      </Section>

      <Section id="change-capture">
        <ReplicationSlotHealth
          title="Live change capture"
          description="Each CDC connector's Postgres replication slot as last checked (every 15 minutes). A disconnected slot is critical whatever its size: it keeps WAL on the source database."
          emptyLine="Live change capture: no CDC connector has reported a replication slot."
          refreshKey={refreshKey}
        />
      </Section>

      <Section id="maintenance">
        <MaintenanceSection refreshKey={refreshKey} />
      </Section>

      <Section id="freshness">
        <div className="flex flex-col gap-4">
          {/* Keyed so the page's Refresh reloads it: the strip owns its
              own request and takes no reload prop. */}
          <FreshnessStrip key={refreshKey} />
          <QueriesSection refreshKey={refreshKey} />
        </div>
      </Section>
    </div>
  )
}
