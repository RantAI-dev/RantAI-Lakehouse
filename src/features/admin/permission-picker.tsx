"use client"

import * as React from "react"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { Badge } from "@/components/ui/badge"
import { identityService, type Permission } from "@/services"
import { XIcon } from "lucide-react"

export type PermissionPickerProps = {
  id?: string
  value: string
  onChange: (value: string) => void
  disabled?: boolean
  permissions?: Permission[]
}

function parseTokens(raw: string): string[] {
  if (!raw) return []
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean)
}

function serializeTokens(tokens: string[]): string {
  return tokens.join(", ")
}

export function PermissionPicker({
  id = "permission-picker",
  value,
  onChange,
  disabled = false,
  permissions: permissionsProp,
}: PermissionPickerProps) {
  const [catalog, setCatalog] = React.useState<Permission[]>(permissionsProp ?? [])
  const [loading, setLoading] = React.useState(!permissionsProp)

  React.useEffect(() => {
    if (permissionsProp) {
      setCatalog(permissionsProp)
      setLoading(false)
      return
    }
    let active = true
    identityService
      .listPermissions()
      .then((perms) => {
        if (active && Array.isArray(perms)) {
          setCatalog(perms)
        }
        if (active) setLoading(false)
      })
      .catch(() => {
        if (active) setLoading(false)
      })
    return () => {
      active = false
    }
  }, [permissionsProp])

  const tokens = React.useMemo(() => parseTokens(value), [value])

  const knownNames = React.useMemo(
    () => new Set(catalog.map((p) => p.name)),
    [catalog]
  )

  const unknownTokens = React.useMemo(() => {
    return tokens.filter((t) => t !== "*:*" && !knownNames.has(t))
  }, [tokens, knownNames])

  const areas = React.useMemo(() => {
    const map = new Map<string, Permission[]>()
    for (const perm of catalog) {
      const list = map.get(perm.area) ?? []
      list.push(perm)
      map.set(perm.area, list)
    }
    return Array.from(map.entries()).map(([area, permissions]) => ({
      area,
      permissions,
    }))
  }, [catalog])

  const hasFullAccess = tokens.includes("*:*")

  function handleFullAccessToggle(checked: boolean) {
    if (checked) {
      onChange("*:*")
    } else {
      onChange(serializeTokens(tokens.filter((t) => t !== "*:*")))
    }
  }

  function handleTokenToggle(name: string, checked: boolean) {
    let next: string[]
    if (checked) {
      next = [...tokens, name]
    } else {
      next = tokens.filter((t) => t !== name)
    }
    onChange(serializeTokens(next))
  }

  function handleAreaAllToggle(areaPermNames: string[], checked: boolean) {
    let next: string[]
    if (checked) {
      const toAdd = areaPermNames.filter((name) => !tokens.includes(name))
      next = [...tokens, ...toAdd]
    } else {
      next = tokens.filter((t) => !areaPermNames.includes(t))
    }
    onChange(serializeTokens(next))
  }

  function handleRemoveUnknown(tokenToRemove: string) {
    const next = tokens.filter((t) => t !== tokenToRemove)
    onChange(serializeTokens(next))
  }

  const renderUnknownTokens = () => (
    <div className="space-y-1.5" data-testid="unknown-tokens">
      <Label className="text-xs font-medium text-muted-foreground">Not recognised</Label>
      <div className="flex flex-wrap gap-1.5">
        {unknownTokens.map((token) => (
          <Badge
            key={token}
            variant="outline"
            className="gap-1.5 border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-400 py-1 px-2 font-mono text-xs"
          >
            <span>{token}</span>
            <span className="text-[10px] text-muted-foreground font-sans">(Not recognised)</span>
            <button
              type="button"
              className="hover:text-foreground inline-flex items-center justify-center cursor-pointer"
              onClick={() => handleRemoveUnknown(token)}
              aria-label={`Remove ${token}`}
              title={`Remove ${token}`}
            >
              <XIcon className="size-3" />
            </button>
          </Badge>
        ))}
      </div>
    </div>
  )

  if (loading && catalog.length === 0) {
    return (
      <div className="text-xs text-muted-foreground py-2" data-testid="permissions-loading">
        Loading permissions...
      </div>
    )
  }

  if (hasFullAccess) {
    return (
      <div className="space-y-4" id={id} data-testid="permission-picker">
        <div className="flex items-center gap-2 p-3 rounded-md border bg-muted/40" data-testid="full-access-box">
          <Checkbox
            id={`${id}-full-access`}
            aria-label="Full access (*:*)"
            checked={true}
            disabled={disabled}
            onCheckedChange={(checked) => handleFullAccessToggle(Boolean(checked))}
          />
          <div className="grid gap-0.5 leading-none">
            <Label htmlFor={`${id}-full-access`} className="text-sm font-semibold cursor-pointer">
              Full access (<code>*:*</code>)
            </Label>
            <p className="text-xs text-muted-foreground">
              Unrestricted platform administrator grant across all areas.
            </p>
          </div>
        </div>
        {unknownTokens.length > 0 && renderUnknownTokens()}
      </div>
    )
  }

  return (
    <div className="space-y-4" id={id} data-testid="permission-picker">
      {unknownTokens.length > 0 && renderUnknownTokens()}

      <div className="space-y-3">
        {areas.map(({ area, permissions: areaPerms }) => {
          const areaNames = areaPerms.map((p) => p.name)
          const allChecked =
            areaNames.length > 0 && areaNames.every((name) => tokens.includes(name))

          return (
            <div
              key={area}
              className="space-y-2.5 rounded-md border p-3 bg-card"
              data-testid={`area-group-${area.toLowerCase()}`}
            >
              <div className="flex items-center justify-between pb-1.5 border-b">
                <span className="text-sm font-semibold text-foreground">{area}</span>
                <div className="flex items-center gap-1.5">
                  <Checkbox
                    id={`${id}-${area}-all`}
                    aria-label={`All in this area (${area})`}
                    checked={allChecked}
                    disabled={disabled}
                    onCheckedChange={(checked) =>
                      handleAreaAllToggle(areaNames, Boolean(checked))
                    }
                  />
                  <Label
                    htmlFor={`${id}-${area}-all`}
                    className="text-xs text-muted-foreground cursor-pointer"
                  >
                    All in this area
                  </Label>
                </div>
              </div>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5 pt-0.5">
                {areaPerms.map((perm) => {
                  const isChecked = tokens.includes(perm.name)
                  const permId = `${id}-${perm.name.replace(/[:*]/g, "-")}`
                  return (
                    <div key={perm.name} className="flex items-start gap-2">
                      <Checkbox
                        id={permId}
                        aria-label={perm.name}
                        checked={isChecked}
                        disabled={disabled}
                        onCheckedChange={(checked) =>
                          handleTokenToggle(perm.name, Boolean(checked))
                        }
                      />
                      <div className="grid gap-0.5 leading-none">
                        <Label
                          htmlFor={permId}
                          className="text-xs font-mono font-medium cursor-pointer"
                        >
                          {perm.name}
                        </Label>
                        <p className="text-[11px] text-muted-foreground leading-snug">
                          {perm.description}
                        </p>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
