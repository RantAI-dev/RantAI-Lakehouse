"use client"

import { Gauge, ListTree, RefreshCw } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"
import { QueryTransparencyPanel } from "./query-transparency-panel"
import { SchemaBrowser } from "./schema-browser"
import type { PanelTab, StudioAction } from "./studio-actions"
import type { useQueryStudio } from "./use-query-studio"

/**
 * The studio's one side panel (`UX-9`, layout A): Schema | Plan. It replaced
 * the schema browser that opened beside the editor and the "Execution
 * transparency" card stacked under it; one place on the right now holds
 * both, a tab each (`DESIGN.md` §3, one side panel at a time).
 *
 * On a laptop it sits beside the editor and stays in view while the page
 * scrolls; on a narrow screen it follows the results.
 */
export function StudioSidePanel({
  studio,
  tab,
  onTabChange,
  estimateAgain,
  className,
}: {
  readonly studio: ReturnType<typeof useQueryStudio>
  readonly tab: PanelTab
  readonly onTabChange: (tab: PanelTab) => void
  /** The command menu's "Estimate again", offered at the foot of the Plan tab too. */
  readonly estimateAgain: StudioAction
  readonly className?: string
}) {
  return (
    <aside
      aria-label="Schema and plan"
      className={cn("flex min-h-0 min-w-0 flex-col bg-background", className)}
    >
      <Tabs
        value={tab}
        onValueChange={(value) => onTabChange(value === "plan" ? "plan" : "schema")}
        className="min-h-0 flex-1 gap-0"
      >
        <div className="flex h-11 shrink-0 items-center border-b border-border px-2">
          <TabsList variant="line" className="h-8 gap-0.5">
            <TabsTrigger
              value="schema"
              className="h-7 flex-none rounded-md px-2.5 text-[13px] data-active:bg-muted after:hidden dark:data-active:bg-input/40"
            >
              <ListTree className="size-3.5" aria-hidden />
              Schema
            </TabsTrigger>
            <TabsTrigger
              value="plan"
              className="h-7 flex-none rounded-md px-2.5 text-[13px] data-active:bg-muted after:hidden dark:data-active:bg-input/40"
            >
              <Gauge className="size-3.5" aria-hidden />
              Plan
            </TabsTrigger>
          </TabsList>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          <TabsContent value="schema">
            <SchemaBrowser studio={studio} />
          </TabsContent>
          <TabsContent value="plan">
            <QueryTransparencyPanel
              state={studio.estimateAct}
              estimatable={studio.estimatable}
              estimateSupported={studio.estimateSupported}
              waitingFor={studio.estimateWaitsFor}
              onRetry={studio.reEstimate}
            />
            {studio.estimateSupported && studio.estimateAct.status === "success" ? (
              <div className="px-3 pb-3">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={estimateAgain.run}
                  disabled={estimateAgain.disabled}
                  title={estimateAgain.disabled ? estimateAgain.reason : undefined}
                >
                  <RefreshCw aria-hidden />
                  Estimate again
                </Button>
              </div>
            ) : null}
          </TabsContent>
        </div>
      </Tabs>
    </aside>
  )
}
