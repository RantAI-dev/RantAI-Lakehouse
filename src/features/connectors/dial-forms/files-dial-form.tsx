"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { FilesDial } from "@/services/contracts/connectors"

/** `dial` for the `files` (object-storage) adapter. */
export function FilesDialForm({
  value,
  onChange,
}: {
  value: FilesDial | null
  onChange: (next: FilesDial) => void
}) {
  const dial: FilesDial = value ?? {
    protocol: "s3",
    endpoint: null,
    bucket: "",
    prefix: null,
    format: "csv",
    region: null,
  }
  function set<K extends keyof FilesDial>(key: K, v: FilesDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <div className="space-y-1.5">
        {/* SRC-6 F8: this adapter reads S3 only (adapters/files.py); SFTP is its
            own connector type. A stored dial that still says "sftp" is shown
            as it is, never rewritten, so the connector opens. */}
        {/* SRC-6 slice A review, SHOULD-FIX 1: the value is plain text, so the
            label names it through aria-labelledby on a group role (a bare
            <p> does not expose a name). */}
        <Label id="files-dial-protocol-label">Protocol</Label>
        <p
          id="files-dial-protocol"
          role="group"
          aria-labelledby="files-dial-protocol-label"
          className="flex h-8 items-center text-sm"
        >
          {dial.protocol === "s3" ? "S3-compatible" : dial.protocol}
        </p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="files-dial-format">Format</Label>
        <select
          id="files-dial-format"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={dial.format}
          onChange={(e) => set("format", e.target.value as FilesDial["format"])}
        >
          <option value="csv">CSV</option>
          <option value="json">Newline-delimited JSON</option>
          <option value="parquet">Parquet</option>
        </select>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="files-dial-bucket">Bucket / root</Label>
        <Input
          id="files-dial-bucket"
          value={dial.bucket}
          onChange={(e) => set("bucket", e.target.value)}
          autoComplete="off"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="files-dial-prefix">Prefix (optional)</Label>
        <Input
          id="files-dial-prefix"
          value={dial.prefix ?? ""}
          onChange={(e) => set("prefix", e.target.value === "" ? null : e.target.value)}
          autoComplete="off"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="files-dial-endpoint">Endpoint</Label>
        <Input
          id="files-dial-endpoint"
          value={dial.endpoint ?? ""}
          onChange={(e) => set("endpoint", e.target.value === "" ? null : e.target.value)}
          placeholder="https://rustfs.internal:9000"
          autoComplete="off"
          aria-describedby="files-dial-endpoint-hint"
        />
        {/* SRC-6 D3: the console's test dials this endpoint; with none it has
            nothing to dial. */}
        <p id="files-dial-endpoint-hint" className="text-xs text-muted-foreground">
          Needed to test the connection. Leave empty for AWS S3.
        </p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="files-dial-region">Region (optional)</Label>
        <Input
          id="files-dial-region"
          value={dial.region ?? ""}
          onChange={(e) => set("region", e.target.value === "" ? null : e.target.value)}
          autoComplete="off"
        />
      </div>
    </div>
  )
}
