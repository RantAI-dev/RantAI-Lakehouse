"use client"

import { useCallback, useMemo, useState } from "react"
import {
  Ban,
  Download,
  KeyRound,
  Layers,
  Mail,
  MailPlus,
  Search,
  Shield,
  UserCheck,
  Users,
} from "lucide-react"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
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

type Role = "Admin" | "Editor" | "Viewer"
type UserStatus = "Active" | "Invited" | "Suspended"

type UserRow = {
  id: string
  name: string
  email: string
  role: Role
  status: UserStatus
  lastActive: string
  department: string
  mfaEnabled: boolean
  authProvider: "SSO" | "Local"
  groups: string[]
  joinedAt: string
  invitedAt?: string
  recentActivity: string[]
}

const SAMPLE_USERS: UserRow[] = [
  {
    id: "1",
    name: "Aisha Rahman",
    email: "aisha.rahman@company.com",
    role: "Admin",
    status: "Active",
    lastActive: "2 min ago",
    department: "Data Platform",
    mfaEnabled: true,
    authProvider: "SSO",
    groups: ["platform-admins", "governance"],
    joinedAt: "2024-03-18",
    recentActivity: [
      "Updated RLS policy on gold.orders_fact",
      "Exported audit sample (CSV)",
    ],
  },
  {
    id: "2",
    name: "Marcus Chen",
    email: "marcus.chen@company.com",
    role: "Editor",
    status: "Active",
    lastActive: "1 hour ago",
    department: "Analytics",
    mfaEnabled: true,
    authProvider: "SSO",
    groups: ["analysts-east", "pipelines-write"],
    joinedAt: "2025-01-08",
    recentActivity: [
      "Ran pipeline dry-run (silver.dim_customer)",
      "Saved Query Studio session",
    ],
  },
  {
    id: "3",
    name: "Elena Vasquez",
    email: "elena.vasquez@company.com",
    role: "Viewer",
    status: "Invited",
    lastActive: "—",
    department: "Finance",
    mfaEnabled: false,
    authProvider: "Local",
    groups: ["finance-read"],
    joinedAt: "—",
    invitedAt: "2026-04-10",
    recentActivity: ["Invite pending — no sessions yet"],
  },
  {
    id: "4",
    name: "James Okafor",
    email: "james.okafor@company.com",
    role: "Editor",
    status: "Active",
    lastActive: "Yesterday",
    department: "Analytics",
    mfaEnabled: true,
    authProvider: "SSO",
    groups: ["analysts-east"],
    joinedAt: "2024-11-22",
    recentActivity: ["Semantic search: 12 queries (24h)"],
  },
  {
    id: "5",
    name: "Sofia Lindström",
    email: "sofia.lindstrom@company.com",
    role: "Viewer",
    status: "Suspended",
    lastActive: "14 days ago",
    department: "Marketing",
    mfaEnabled: true,
    authProvider: "SSO",
    groups: ["marketing-read"],
    joinedAt: "2023-09-01",
    recentActivity: ["Account suspended — login blocked"],
  },
  {
    id: "6",
    name: "David Park",
    email: "david.park@company.com",
    role: "Admin",
    status: "Active",
    lastActive: "5 min ago",
    department: "Security",
    mfaEnabled: true,
    authProvider: "SSO",
    groups: ["security", "platform-admins"],
    joinedAt: "2024-07-02",
    recentActivity: [
      "Reviewed connector enablement (S3)",
      "Exported user list",
    ],
  },
]

const ROLE_FILTER_ALL = "all"
const STATUS_FILTER_ALL = "all"

function initials(name: string) {
  const parts = name.trim().split(/\s+/)
  const a = parts[0]?.[0] ?? ""
  const b = parts.length > 1 ? parts[parts.length - 1]![0] : parts[0]?.[1] ?? ""
  return (a + b).toUpperCase()
}

function StatusBadge({ status }: { status: UserStatus }) {
  const variant =
    status === "Active"
      ? "bg-[#ecfdf2] text-[#008a2e] border-0 hover:bg-[#ecfdf2]"
      : status === "Invited"
        ? "bg-primary/10 text-primary border-0"
        : "bg-destructive/10 text-destructive border-0"
  return (
    <Badge className={cn("text-xs font-semibold tracking-[-0.072px]", variant)}>
      {status}
    </Badge>
  )
}

function RoleBadge({ role }: { role: Role }) {
  return (
    <Badge
      variant="outline"
      className="border-border font-medium text-primary"
    >
      {role}
    </Badge>
  )
}

export default function UserManagementPage() {
  const [query, setQuery] = useState("")
  const [roleFilter, setRoleFilter] = useState<string>(ROLE_FILTER_ALL)
  const [statusFilter, setStatusFilter] = useState<string>(STATUS_FILTER_ALL)
  const [detailOpen, setDetailOpen] = useState(false)
  const [activeUser, setActiveUser] = useState<UserRow | null>(null)

  const openDetail = useCallback((u: UserRow) => {
    setActiveUser(u)
    setDetailOpen(true)
  }, [])

  const stats = useMemo(() => {
    const total = SAMPLE_USERS.length
    const active = SAMPLE_USERS.filter((u) => u.status === "Active").length
    const invited = SAMPLE_USERS.filter((u) => u.status === "Invited").length
    const suspended = SAMPLE_USERS.filter((u) => u.status === "Suspended").length
    const admins = SAMPLE_USERS.filter((u) => u.role === "Admin").length
    return [
      { label: "Total users", value: total, icon: Users },
      { label: "Active", value: active, icon: UserCheck },
      { label: "Pending invites", value: invited, icon: MailPlus },
      { label: "Suspended", value: suspended, icon: Ban },
      { label: "Admins", value: admins, icon: Shield },
    ] as const
  }, [])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    return SAMPLE_USERS.filter((u) => {
      const roleOk =
        roleFilter === ROLE_FILTER_ALL || u.role === roleFilter
      const statusOk =
        statusFilter === STATUS_FILTER_ALL || u.status === statusFilter
      if (!roleOk || !statusOk) return false
      if (!q) return true
      return (
        u.name.toLowerCase().includes(q) ||
        u.email.toLowerCase().includes(q) ||
        u.department.toLowerCase().includes(q)
      )
    })
  }, [query, roleFilter, statusFilter])

  return (
    <div className="flex flex-col gap-4">
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-border pb-2">
        <h1 className="text-[24px] font-semibold leading-8 tracking-[-0.144px] text-primary">
          User Management
        </h1>
        <div className="flex flex-wrap items-center gap-3">
          <Button
            variant="outline"
            size="default"
            className="h-10 gap-2 rounded-md border-border px-4 text-sm font-medium text-primary hover:bg-muted hover:text-primary"
            type="button"
          >
            <Download className="size-4" />
            Export
          </Button>
          <Button
            size="default"
            className="h-10 gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground"
            type="button"
          >
            <MailPlus className="size-4" />
            Invite user
          </Button>
        </div>
      </header>
      <p className="-mt-2 text-sm text-muted-foreground">
        Roles, groups, and MFA for this organization. Open a user to see
        auth, groups, and recent activity.
      </p>

      <section className="inline-flex w-fit max-w-full flex-wrap gap-2 self-start rounded-md bg-accent p-2">
        {stats.map((item) => {
          const Icon = item.icon
          return (
            <Card
              key={item.label}
              className="w-[152px] shrink-0 gap-0 overflow-hidden rounded-lg border border-border bg-card shadow-sm"
            >
              <CardHeader className="space-y-0 px-3 pt-3 pb-1.5">
                <div className="flex items-center gap-1.5">
                  <Icon className="size-3.5 shrink-0 text-muted-foreground" />
                  <p className="text-sm font-medium leading-5 tracking-[-0.084px] text-primary">
                    {item.label}
                  </p>
                </div>
              </CardHeader>
              <CardContent className="px-3 pb-4 pt-1.5">
                <p className="text-[24px] font-medium leading-none tracking-[-0.144px] text-muted-foreground">
                  {item.value}
                </p>
              </CardContent>
            </Card>
          )
        })}
      </section>

      <section className="flex flex-col gap-3">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          <div className="relative max-w-md flex-1">
            <Search
              className="pointer-events-none absolute left-3 top-1/2 size-4 shrink-0 -translate-y-1/2 text-primary"
              aria-hidden
            />
            <Input
              type="search"
              placeholder="Search by name, email, or department…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="h-10 w-full rounded-md border-border bg-background pl-9 pr-3 text-sm"
              aria-label="Search users"
            />
          </div>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <Select
              value={roleFilter}
              onValueChange={(v) => setRoleFilter(v ?? ROLE_FILTER_ALL)}
            >
              <SelectTrigger className="h-10 w-full rounded-md border-border sm:w-[160px] pl-3 text-sm text-primary data-placeholder:text-muted-foreground [&_svg]:text-primary">
                <SelectValue placeholder="Role" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ROLE_FILTER_ALL}>All roles</SelectItem>
                <SelectItem value="Admin">Admin</SelectItem>
                <SelectItem value="Editor">Editor</SelectItem>
                <SelectItem value="Viewer">Viewer</SelectItem>
              </SelectContent>
            </Select>
            <Select
              value={statusFilter}
              onValueChange={(v) => setStatusFilter(v ?? STATUS_FILTER_ALL)}
            >
              <SelectTrigger className="h-10 w-full rounded-md border-border sm:w-[160px] pl-3 text-sm text-primary data-placeholder:text-muted-foreground [&_svg]:text-primary">
                <SelectValue placeholder="Status" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={STATUS_FILTER_ALL}>All statuses</SelectItem>
                <SelectItem value="Active">Active</SelectItem>
                <SelectItem value="Invited">Invited</SelectItem>
                <SelectItem value="Suspended">Suspended</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        <div className="overflow-hidden rounded-lg border border-border bg-card shadow-sm">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-4">User</TableHead>
                <TableHead className="hidden lg:table-cell">Department</TableHead>
                <TableHead>Role</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="hidden md:table-cell">Auth</TableHead>
                <TableHead>Last active</TableHead>
                <TableHead className="w-[100px] pr-4 text-right">Detail</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={7}
                    className="py-10 text-center text-muted-foreground"
                  >
                    No users match your filters.
                  </TableCell>
                </TableRow>
              ) : (
                filtered.map((user) => (
                  <TableRow key={user.id}>
                    <TableCell className="pl-4">
                      <div className="flex max-w-[280px] items-center gap-3">
                        <div
                          className="flex size-9 shrink-0 items-center justify-center rounded-full bg-primary/15 text-xs font-semibold text-primary"
                          aria-hidden
                        >
                          {initials(user.name)}
                        </div>
                        <div className="min-w-0">
                          <button
                            type="button"
                            onClick={() => openDetail(user)}
                            className="truncate text-left font-medium text-primary underline-offset-2 hover:underline"
                          >
                            {user.name}
                          </button>
                          <p className="truncate text-sm text-muted-foreground">
                            {user.email}
                          </p>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="hidden text-muted-foreground lg:table-cell">
                      {user.department}
                    </TableCell>
                    <TableCell>
                      <RoleBadge role={user.role} />
                    </TableCell>
                    <TableCell>
                      <StatusBadge status={user.status} />
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <span className="text-xs text-muted-foreground">
                        {user.authProvider}
                        {user.mfaEnabled ? " · MFA" : ""}
                      </span>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {user.lastActive}
                    </TableCell>
                    <TableCell className="pr-4 text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        className="text-primary hover:text-primary"
                        type="button"
                        onClick={() => openDetail(user)}
                      >
                        View
                      </Button>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </section>

      <Sheet open={detailOpen} onOpenChange={setDetailOpen}>
        <SheetContent
          side="right"
          className="flex h-full max-h-[100dvh] w-full flex-col gap-0 overflow-hidden p-0 sm:max-w-lg"
        >
          {activeUser && (
            <>
              <SheetHeader className="shrink-0 space-y-4 border-b border-border bg-muted/25 px-5 pb-4 pt-5 text-left">
                <div className="flex items-start gap-4">
                  <div
                    className="flex size-14 shrink-0 items-center justify-center rounded-full bg-primary/15 text-lg font-semibold text-primary"
                    aria-hidden
                  >
                    {initials(activeUser.name)}
                  </div>
                  <div className="min-w-0 flex-1 space-y-1">
                    <SheetTitle className="text-xl font-semibold tracking-tight">
                      {activeUser.name}
                    </SheetTitle>
                    <SheetDescription className="font-mono text-xs">
                      {activeUser.email}
                    </SheetDescription>
                    <div className="flex flex-wrap items-center gap-2 pt-1">
                      <RoleBadge role={activeUser.role} />
                      <StatusBadge status={activeUser.status} />
                    </div>
                  </div>
                </div>
                <p className="text-sm text-muted-foreground">
                  <span className="font-medium text-foreground">
                    {activeUser.department}
                  </span>
                  {activeUser.invitedAt && activeUser.status === "Invited" ? (
                    <>
                      {" "}
                      · Invited {activeUser.invitedAt}
                    </>
                  ) : null}
                </p>
              </SheetHeader>

              <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
                <section className="space-y-2">
                  <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Authentication
                  </h3>
                  <div className="grid gap-px overflow-hidden rounded-lg border border-border bg-border sm:grid-cols-2">
                    <div className="flex gap-2 bg-card px-3 py-2.5">
                      <KeyRound className="mt-0.5 size-4 shrink-0 text-primary" />
                      <div>
                        <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                          Provider
                        </p>
                        <p className="mt-0.5 text-sm font-medium">
                          {activeUser.authProvider}
                        </p>
                      </div>
                    </div>
                    <div className="flex gap-2 bg-card px-3 py-2.5">
                      <Shield className="mt-0.5 size-4 shrink-0 text-primary" />
                      <div>
                        <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                          MFA
                        </p>
                        <p className="mt-0.5 text-sm font-medium">
                          {activeUser.mfaEnabled ? "Enabled" : "Not set"}
                        </p>
                      </div>
                    </div>
                    <div className="flex gap-2 bg-card px-3 py-2.5 sm:col-span-2">
                      <Mail className="mt-0.5 size-4 shrink-0 text-primary" />
                      <div>
                        <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                          Member since
                        </p>
                        <p className="mt-0.5 text-sm font-medium">
                          {activeUser.joinedAt}
                        </p>
                      </div>
                    </div>
                  </div>
                </section>

                <Separator className="my-5" />

                <section className="space-y-2">
                  <h3 className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                    <Layers className="size-3.5" />
                    Groups
                  </h3>
                  <div className="flex flex-wrap gap-1.5">
                    {activeUser.groups.map((g) => (
                      <Badge
                        key={g}
                        variant="outline"
                        className="font-mono text-[11px] font-normal text-foreground"
                      >
                        {g}
                      </Badge>
                    ))}
                  </div>
                </section>

                <Separator className="my-5" />

                <section className="space-y-2">
                  <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Recent activity
                  </h3>
                  <p className="text-xs text-muted-foreground">
                    Last seen:{" "}
                    <span className="font-medium text-foreground">
                      {activeUser.lastActive}
                    </span>
                  </p>
                  <ul className="space-y-2 rounded-lg border border-border bg-muted/30 p-3">
                    {activeUser.recentActivity.map((line) => (
                      <li
                        key={line}
                        className="flex gap-2 text-sm leading-snug text-foreground"
                      >
                        <span className="mt-1.5 size-1 shrink-0 rounded-full bg-primary/60" />
                        <span>{line}</span>
                      </li>
                    ))}
                  </ul>
                </section>
              </div>

              <SheetFooter className="shrink-0 gap-2 border-t border-border bg-card/90 px-5 py-4 backdrop-blur-sm sm:flex-row sm:justify-end">
                <Button type="button" variant="outline" size="sm">
                  Reset MFA
                </Button>
                <Button type="button" variant="outline" size="sm">
                  Edit role
                </Button>
                <Button type="button" size="sm" className="bg-primary">
                  Suspend user
                </Button>
              </SheetFooter>
            </>
          )}
        </SheetContent>
      </Sheet>
    </div>
  )
}
