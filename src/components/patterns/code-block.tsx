import { cn } from "@/lib/utils"

/** Monospace block for SQL, identifiers, and configuration snippets. */
export function CodeBlock({
  children,
  className,
}: {
  children: string
  className?: string
}) {
  return (
    <pre
      className={cn(
        "overflow-x-auto rounded-lg border border-border bg-muted/30 px-4 py-3 font-mono text-xs leading-5 text-foreground",
        className
      )}
    >
      {children}
    </pre>
  )
}

/** Pretty-printed JSON viewer for tool schemas and event payloads. */
export function JsonViewer({
  value,
  className,
}: {
  value: unknown
  className?: string
}) {
  return <CodeBlock className={className}>{JSON.stringify(value, null, 2)}</CodeBlock>
}
