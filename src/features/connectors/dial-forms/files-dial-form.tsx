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
        <Label htmlFor="files-dial-protocol">Protocol</Label>
        <select
          id="files-dial-protocol"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={dial.protocol}
          onChange={(e) => set("protocol", e.target.value as FilesDial["protocol"])}
        >
          <option value="s3">S3-compatible</option>
          <option value="sftp">SFTP</option>
        </select>
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
        <Input id="files-dial-bucket" value={dial.bucket} onChange={(e) => set("bucket", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="files-dial-prefix">Prefix (optional)</Label>
        <Input
          id="files-dial-prefix"
          value={dial.prefix ?? ""}
          onChange={(e) => set("prefix", e.target.value === "" ? null : e.target.value)}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="files-dial-endpoint">Endpoint override (optional)</Label>
        <Input
          id="files-dial-endpoint"
          value={dial.endpoint ?? ""}
          onChange={(e) => set("endpoint", e.target.value === "" ? null : e.target.value)}
          placeholder="https://rustfs.internal:9000"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="files-dial-region">Region (optional)</Label>
        <Input
          id="files-dial-region"
          value={dial.region ?? ""}
          onChange={(e) => set("region", e.target.value === "" ? null : e.target.value)}
        />
      </div>
    </div>
  )
}
