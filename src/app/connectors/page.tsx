"use client"

import { useState } from "react"
import { Check, Database, Plug, Server } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardFooter, CardHeader } from "@/components/ui/card"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"

type Conn = {
  id: string
  name: string
  kind: string
  description: string
  icon: typeof Plug
  status: "Connected" | "Available" | "Error"
}

const CONNECTORS: Conn[] = [
  {
    id: "pg",
    name: "PostgreSQL",
    kind: "OLTP",
    description: "CDC from core banking — WAL capture.",
    icon: Database,
    status: "Connected",
  },
  {
    id: "s3",
    name: "Amazon S3",
    kind: "Object",
    description: "Landing zone for raw files and exports.",
    icon: Server,
    status: "Connected",
  },
]

export default function ConnectorsPage() {
  const [enabled, setEnabled] = useState<Record<string, boolean>>({
    pg: true,
    s3: true,
  })

  return (
    <div className="flex flex-col gap-4">
      <header className="border-b border-border pb-2">
        <div className="flex flex-wrap items-center gap-2">
          <Plug className="size-7 text-primary" aria-hidden />
          <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
            Connectors
          </h1>
        </div>
        <p className="mt-1 text-sm text-muted-foreground">
          Enable sources for ingestion pipelines. Disabled connectors skip
          scheduled runs.
        </p>
      </header>

      <section className="grid gap-3 sm:grid-cols-2">
        {CONNECTORS.map((c) => {
          const Icon = c.icon
          const on = enabled[c.id] ?? false
          return (
            <Card
              key={c.id}
              className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]"
            >
              <CardHeader className="flex flex-row items-start justify-between gap-3 p-4">
                <div className="flex min-w-0 items-start gap-3">
                  <div className="flex size-10 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
                    <Icon className="size-5" aria-hidden />
                  </div>
                  <div className="min-w-0">
                    <p className="font-medium text-primary">{c.name}</p>
                    <p className="text-sm text-muted-foreground">{c.description}</p>
                  </div>
                </div>
                <Switch
                  checked={on}
                  onCheckedChange={(v) =>
                    setEnabled((prev) => ({ ...prev, [c.id]: v }))
                  }
                  aria-label={`Enable ${c.name}`}
                />
              </CardHeader>
              <CardContent className="px-4 pb-2">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="outline" className="text-[11px] text-primary">
                    {c.kind}
                  </Badge>
                  <Badge
                    className={cn(
                      "border-0 text-[11px] font-semibold",
                      c.status === "Connected" &&
                        "bg-[#ecfdf2] text-[#008a2e] hover:bg-[#ecfdf2]",
                      c.status === "Available" &&
                        "bg-primary/10 text-primary hover:bg-primary/10",
                      c.status === "Error" &&
                        "bg-destructive/10 text-destructive"
                    )}
                  >
                    {c.status}
                  </Badge>
                </div>
              </CardContent>
              <CardFooter className="border-t border-border px-4 py-3">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="gap-2 border-border text-primary"
                  disabled={!on}
                >
                  <Check className="size-4" aria-hidden />
                  Configure
                </Button>
              </CardFooter>
            </Card>
          )
        })}
      </section>
    </div>
  )
}
