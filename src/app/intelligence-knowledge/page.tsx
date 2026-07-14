"use client"

import { Fragment, useCallback, useRef, useState } from "react"
import {
  ChevronDown,
  ChevronRight,
  FileText,
  Library,
  Loader2,
  PenLine,
  Upload,
  UploadCloud,
  Users,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import {
  KNOWLEDGE_LIBRARY_INITIAL_ENTRIES,
  USER_KNOWLEDGE_DOCUMENTS_INITIAL,
  type KnowledgeEntry,
  type UserKnowledgeDocument,
} from "@/lib/knowledge-library-fixtures"

/** Formats an ISO date as e.g. "Apr 17, 2026, 2:22 PM"; falls back to raw string on error. */
function formatShortDate(iso: string) {
  try {
    return new Intl.DateTimeFormat("en-US", {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(iso))
  } catch {
    return iso
  }
}

/** Formats a byte count with a single unit (B / KB / MB) for the documents list. */
function formatFileSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  const kb = bytes / 1024
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`
  const mb = kb / 1024
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`
}

/**
 * Intelligence Knowledge library page at `/intelligence-knowledge`.
 *
 * Two tabs:
 * - **Query knowledge** — entries absorbed from query results (mock-ingested via
 *   the "Ingest from last query" button).
 * - **Documents & context** — user-uploaded files and free-form web context
 *   (a `Sheet` collects title + body before saving).
 *
 * Both tables are expandable rows: clicking a row toggles a detail panel
 * (source query for queries; file metadata or body for documents).
 *
 * All state is in-memory (initialized from the `*_INITIAL` fixtures).
 */
export default function IntelligenceKnowledgePage() {
  const [entries, setEntries] = useState<KnowledgeEntry[]>(
    KNOWLEDGE_LIBRARY_INITIAL_ENTRIES
  )
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [ingesting, setIngesting] = useState(false)

  const [userDocuments, setUserDocuments] = useState<UserKnowledgeDocument[]>(
    USER_KNOWLEDGE_DOCUMENTS_INITIAL
  )
  const [docExpandedId, setDocExpandedId] = useState<string | null>(null)
  const [contextOpen, setContextOpen] = useState(false)
  const [contextTitle, setContextTitle] = useState("")
  const [contextBody, setContextBody] = useState("")
  const fileInputRef = useRef<HTMLInputElement>(null)

  /**
   * Mock-ingests one new knowledge entry from the "last query result".
   *
   * Adds a 900ms delay to mimic a network call, then prepends a synthetic
   * `KnowledgeEntry` to the list. No real backend is involved.
   */
  const absorbMockQuery = useCallback(async () => {
    if (ingesting) return
    setIngesting(true)
    await new Promise((r) => setTimeout(r, 900))
    const next: KnowledgeEntry = {
      id: `k-${Date.now()}`,
      title: "Latest query snapshot (mock)",
      sourceQuery:
        "SELECT * FROM gold.orders_fact WHERE order_date = CURRENT_DATE LIMIT 500",
      rowCount: 500,
      distinctUsers: 3,
      zone: "Gold",
      excerpt:
        "Daily snapshot: amount distribution follows the weekly trend; minor outliers on a few order IDs.",
      ingestedAt: new Date().toISOString(),
    }
    setEntries((prev) => [next, ...prev])
    setIngesting(false)
  }, [ingesting])

  /**
   * Handles the file picker input. Adds one `UserKnowledgeDocument` (kind `"upload"`)
   * per selected file, capturing name, size, and MIME type. File contents are NOT
   * read — this is a UI-only fixture flow.
   */
  const handleFilesSelected = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const list = e.target.files
      if (!list?.length) return
      const now = new Date().toISOString()
      const added: UserKnowledgeDocument[] = []
      for (let i = 0; i < list.length; i++) {
        const f = list[i]
        added.push({
          id: `ud-${Date.now()}-${i}`,
          kind: "upload",
          title: f.name,
          fileName: f.name,
          fileSizeBytes: f.size,
          mimeType: f.type || "application/octet-stream",
          createdAt: now,
        })
      }
      setUserDocuments((prev) => [...added, ...prev])
      e.target.value = ""
    },
    []
  )

  /**
   * Saves the "New context" sheet form as a `web_context` document and resets
   * the form. No-ops when title or body is blank (the Save button is also disabled).
   */
  const saveWebContext = useCallback(() => {
    const title = contextTitle.trim()
    const body = contextBody.trim()
    if (!title || !body) return
    const next: UserKnowledgeDocument = {
      id: `ud-ctx-${Date.now()}`,
      kind: "web_context",
      title,
      body,
      createdAt: new Date().toISOString(),
    }
    setUserDocuments((prev) => [next, ...prev])
    setContextTitle("")
    setContextBody("")
    setContextOpen(false)
  }, [contextTitle, contextBody])

  return (
    <div className="flex flex-col gap-4">
      <header className="border-b border-border pb-2">
        <div className="flex flex-wrap items-center gap-2">
          <Library className="size-7 shrink-0 text-primary" aria-hidden />
          <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
            Intellegance Knowladge
          </h1>
        </div>
        <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
          Query-derived knowledge, uploaded documents, and web-authored context
          in one library. Use the tabs to switch between query ingest and your
          files.
        </p>
      </header>

      <Tabs defaultValue="queries" className="flex flex-col gap-4">
        <TabsList className="h-auto w-fit flex-wrap gap-1 rounded-md bg-secondary p-1">
          <TabsTrigger
            value="queries"
            className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
          >
            Query knowledge
          </TabsTrigger>
          <TabsTrigger
            value="documents"
            className="rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
          >
            Documents &amp; context
          </TabsTrigger>
        </TabsList>

        <TabsContent value="queries" className="mt-0 flex flex-col gap-4">
          <div className="flex flex-wrap items-center justify-end gap-2">
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    type="button"
                    variant="outline"
                    disabled={ingesting}
                    className="h-10 gap-2 rounded-md border-border px-4 text-sm font-medium text-primary hover:bg-muted hover:text-primary"
                    onClick={absorbMockQuery}
                  >
                    {ingesting ? (
                      <Loader2 className="size-4 animate-spin" aria-hidden />
                    ) : (
                      <UploadCloud className="size-4" aria-hidden />
                    )}
                    Ingest from last query
                  </Button>
                }
              />
              <TooltipContent className="max-w-xs text-left">
                Mock: append one knowledge entry from the latest query result,
                as if writing to your vector or document store.
              </TooltipContent>
            </Tooltip>
          </div>

      <section className="flex flex-col gap-3">
        <div className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="min-w-[200px] pl-4">Knowledge entry</TableHead>
                <TableHead>Zone</TableHead>
                <TableHead className="whitespace-nowrap">Ingested</TableHead>
                <TableHead className="text-right">
                  <span className="inline-flex items-center justify-end gap-1">
                    <Users className="size-3.5 text-muted-foreground" aria-hidden />
                    Users
                  </span>
                </TableHead>
                <TableHead className="text-right">Rows</TableHead>
                <TableHead className="w-12 pr-4" aria-label="Expand" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {entries.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={6}
                    className="py-10 text-center text-muted-foreground"
                  >
                    No knowledge entries yet. Use ingest to add from query
                    results.
                  </TableCell>
                </TableRow>
              ) : (
                entries.map((c) => {
                  const open = expandedId === c.id
                  return (
                    <Fragment key={c.id}>
                      <TableRow
                        className={cn(
                          "cursor-pointer",
                          open && "bg-muted/40"
                        )}
                        onClick={() =>
                          setExpandedId((prev) =>
                            prev === c.id ? null : c.id
                          )
                        }
                      >
                        <TableCell className="max-w-[360px] pl-4 align-top">
                          <span className="flex items-start gap-2">
                            {open ? (
                              <ChevronDown className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                            ) : (
                              <ChevronRight className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                            )}
                            <span className="font-medium leading-snug text-primary">
                              {c.title}
                            </span>
                          </span>
                        </TableCell>
                        <TableCell className="align-top">
                          <Badge
                            variant="outline"
                            className="text-[11px] text-primary"
                          >
                            {c.zone}
                          </Badge>
                        </TableCell>
                        <TableCell className="align-top font-mono text-xs text-muted-foreground whitespace-nowrap">
                          {formatShortDate(c.ingestedAt)}
                        </TableCell>
                        <TableCell className="align-top text-right font-mono text-sm tabular-nums text-primary">
                          {c.distinctUsers.toLocaleString("en-US")}
                        </TableCell>
                        <TableCell className="align-top text-right font-mono text-sm text-muted-foreground tabular-nums">
                          {c.rowCount.toLocaleString("en-US")}
                        </TableCell>
                        <TableCell className="pr-4 align-top" />
                      </TableRow>
                      {open && (
                        <TableRow className="hover:bg-transparent">
                          <TableCell
                            colSpan={6}
                            className="border-t border-border bg-muted/25 px-4 pb-4 pt-0"
                          >
                            <div className="space-y-3 pl-6 pt-3">
                              <p className="text-sm text-muted-foreground">
                                <span className="font-medium text-primary">
                                  {c.distinctUsers.toLocaleString("en-US")}{" "}
                                </span>
                                distinct users ran this query (within the tracked
                                window).
                              </p>
                              <div>
                                <p className="text-xs font-medium text-primary">
                                  Source query
                                </p>
                                <pre className="mt-1 max-h-36 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-background p-3 font-mono text-[11px] leading-relaxed text-muted-foreground shadow-[inset_0_1px_2px_0_rgba(0,0,0,0.04)]">
                                  {c.sourceQuery}
                                </pre>
                              </div>
                              <div>
                                <p className="text-xs font-medium text-primary">
                                  Absorbed summary
                                </p>
                                <p className="mt-1 text-sm leading-relaxed text-muted-foreground">
                                  {c.excerpt}
                                </p>
                              </div>
                            </div>
                          </TableCell>
                        </TableRow>
                      )}
                    </Fragment>
                  )
                })
              )}
            </TableBody>
          </Table>
        </div>
      </section>
        </TabsContent>

        <TabsContent value="documents" className="mt-0 flex flex-col gap-4">
          <input
            ref={fileInputRef}
            type="file"
            className="sr-only"
            multiple
            onChange={handleFilesSelected}
            aria-hidden
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="outline"
              className="h-10 gap-2 rounded-md border-border px-4 text-sm font-medium"
              onClick={() => fileInputRef.current?.click()}
            >
              <Upload className="size-4" aria-hidden />
              Upload document
            </Button>
            <Button
              type="button"
              className="h-10 gap-2 rounded-md px-4 text-sm font-medium"
              onClick={() => setContextOpen(true)}
            >
              <PenLine className="size-4" aria-hidden />
              New context
            </Button>
          </div>
          <p className="text-sm text-muted-foreground">
            Upload files to index alongside your lake knowledge, or write
            context directly in the browser—both appear in this list.
          </p>

          <section className="flex flex-col gap-3">
            <div className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
              <Table>
                <TableHeader>
                  <TableRow className="hover:bg-transparent">
                    <TableHead className="min-w-[200px] pl-4">Title</TableHead>
                    <TableHead>Type</TableHead>
                    <TableHead className="whitespace-nowrap">Added</TableHead>
                    <TableHead className="w-12 pr-4" aria-label="Expand" />
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {userDocuments.length === 0 ? (
                    <TableRow>
                      <TableCell
                        colSpan={4}
                        className="py-10 text-center text-muted-foreground"
                      >
                        No documents yet. Upload a file or create context to get
                        started.
                      </TableCell>
                    </TableRow>
                  ) : (
                    userDocuments.map((d) => {
                      const open = docExpandedId === d.id
                      const typeLabel =
                        d.kind === "upload" ? "File upload" : "Web context"
                      return (
                        <Fragment key={d.id}>
                          <TableRow
                            className={cn(
                              "cursor-pointer",
                              open && "bg-muted/40"
                            )}
                            onClick={() =>
                              setDocExpandedId((prev) =>
                                prev === d.id ? null : d.id
                              )
                            }
                          >
                            <TableCell className="max-w-[360px] pl-4 align-top">
                              <span className="flex items-start gap-2">
                                {open ? (
                                  <ChevronDown className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                                ) : (
                                  <ChevronRight className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                                )}
                                <span className="flex items-start gap-2">
                                  {d.kind === "upload" ? (
                                    <FileText className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                                  ) : (
                                    <PenLine className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                                  )}
                                  <span className="font-medium leading-snug text-primary">
                                    {d.title}
                                  </span>
                                </span>
                              </span>
                            </TableCell>
                            <TableCell className="align-top">
                              <Badge
                                variant="outline"
                                className="text-[11px] text-primary"
                              >
                                {typeLabel}
                              </Badge>
                            </TableCell>
                            <TableCell className="align-top font-mono text-xs text-muted-foreground whitespace-nowrap">
                              {formatShortDate(d.createdAt)}
                            </TableCell>
                            <TableCell className="pr-4 align-top" />
                          </TableRow>
                          {open && (
                            <TableRow className="hover:bg-transparent">
                              <TableCell
                                colSpan={4}
                                className="border-t border-border bg-muted/25 px-4 pb-4 pt-0"
                              >
                                <div className="space-y-3 pl-6 pt-3">
                                  {d.kind === "upload" ? (
                                    <>
                                      <div className="grid gap-1 text-sm">
                                        <span className="text-xs font-medium text-primary">
                                          File
                                        </span>
                                        <span className="font-mono text-xs text-muted-foreground">
                                          {d.fileName}
                                        </span>
                                      </div>
                                      <div className="flex flex-wrap gap-4 text-sm text-muted-foreground">
                                        <span>
                                          <span className="font-medium text-primary">
                                            Size
                                          </span>{" "}
                                          {formatFileSize(d.fileSizeBytes)}
                                        </span>
                                        <span>
                                          <span className="font-medium text-primary">
                                            MIME
                                          </span>{" "}
                                          {d.mimeType}
                                        </span>
                                      </div>
                                    </>
                                  ) : (
                                    <div>
                                      <p className="text-xs font-medium text-primary">
                                        Content
                                      </p>
                                      <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-background p-3 font-sans text-sm leading-relaxed text-muted-foreground shadow-[inset_0_1px_2px_0_rgba(0,0,0,0.04)]">
                                        {d.body}
                                      </pre>
                                    </div>
                                  )}
                                </div>
                              </TableCell>
                            </TableRow>
                          )}
                        </Fragment>
                      )
                    })
                  )}
                </TableBody>
              </Table>
            </div>
          </section>
        </TabsContent>
      </Tabs>

      <Sheet open={contextOpen} onOpenChange={setContextOpen}>
        <SheetContent
          side="right"
          className="flex h-full max-h-[100dvh] w-full flex-col gap-0 border-l border-border p-0 sm:max-w-lg"
        >
          <SheetHeader className="shrink-0 space-y-2 border-b border-border px-5 pb-4 pt-5 text-left">
            <SheetTitle>New web context</SheetTitle>
            <SheetDescription>
              Save free-form notes or instructions. They are stored like
              uploaded documents and listed under Documents &amp; context.
            </SheetDescription>
          </SheetHeader>
          <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4">
            <div className="space-y-2">
              <Label htmlFor="ctx-title">Title</Label>
              <Input
                id="ctx-title"
                value={contextTitle}
                onChange={(e) => setContextTitle(e.target.value)}
                placeholder="e.g. Q2 rollout — data quality checks"
                className="h-10"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="ctx-body">Context</Label>
              <Textarea
                id="ctx-body"
                value={contextBody}
                onChange={(e) => setContextBody(e.target.value)}
                placeholder="Write the context you want indexed with your knowledge library…"
                className="min-h-40"
              />
            </div>
          </div>
          <SheetFooter className="shrink-0 border-t border-border bg-muted/20 sm:flex-row sm:justify-end">
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                setContextOpen(false)
                setContextTitle("")
                setContextBody("")
              }}
            >
              Cancel
            </Button>
            <Button
              type="button"
              disabled={!contextTitle.trim() || !contextBody.trim()}
              onClick={saveWebContext}
            >
              Save context
            </Button>
          </SheetFooter>
        </SheetContent>
      </Sheet>
    </div>
  )
}
