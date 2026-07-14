"use client"

import Link from "next/link"
import { useMemo, useState } from "react"
import { FolderPlus, Users } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Badge } from "@/components/ui/badge"
import { Textarea } from "@/components/ui/textarea"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet"
import {
  MOCK_COLLABORATION_PROJECTS,
  type CollaborationProject,
} from "@/lib/query-collaboration-fixtures"

export default function QueryCollaborationProjectsPage() {
  const [projectName, setProjectName] = useState("")
  const [collaboratorsRaw, setCollaboratorsRaw] = useState("")
  const [projects, setProjects] = useState<CollaborationProject[]>(
    () => MOCK_COLLABORATION_PROJECTS
  )
  const [error, setError] = useState<string | null>(null)
  const [createOpen, setCreateOpen] = useState(false)

  const collaboratorCandidates = useMemo(
    () =>
      collaboratorsRaw
        .split(",")
        .map((name) => name.trim())
        .filter(Boolean),
    [collaboratorsRaw]
  )

  const createProject = () => {
    const trimmedName = projectName.trim()
    if (!trimmedName) {
      setError("Project name is required.")
      return
    }
    if (collaboratorCandidates.length === 0) {
      setError("Add at least one collaborator.")
      return
    }

    const now = Date.now()
    const id = `${trimmedName.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${now
      .toString()
      .slice(-4)}`
    const newProject: CollaborationProject = {
      id,
      name: trimmedName,
      memberCount: collaboratorCandidates.length + 1,
      lastActivity: "Just now",
      description:
        "New collaboration project. Start by running a Natural Language or SQL query together.",
      members: [
        { id: `m-${now}`, name: "You", role: "Owner", status: "online" },
        ...collaboratorCandidates.map((name, idx) => ({
          id: `m-${now}-${idx}`,
          name,
          role: "Analyst" as const,
          status: "offline" as const,
        })),
      ],
      activities: [
        {
          id: `a-${now}`,
          actor: "You",
          action: `created project "${trimmedName}"`,
          when: "Just now",
        },
      ],
      queryHistory: [],
    }
    setProjects((prev) => [newProject, ...prev])
    setProjectName("")
    setCollaboratorsRaw("")
    setError(null)
    setCreateOpen(false)
  }

  return (
    <div className="flex min-h-[min(720px,calc(100dvh-7rem))] flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border pb-2">
        <div>
          <Breadcrumb>
            <BreadcrumbList>
              <BreadcrumbItem>
                <BreadcrumbLink render={<Link href="/query-studio" />}>
                  Query Studio
                </BreadcrumbLink>
              </BreadcrumbItem>
              <BreadcrumbSeparator />
              <BreadcrumbItem>
                <BreadcrumbPage>Collaboration</BreadcrumbPage>
              </BreadcrumbItem>
            </BreadcrumbList>
          </Breadcrumb>
          <h1 className="text-2xl font-semibold tracking-tight text-primary">
            Collaboration Projects
          </h1>
          <p className="text-sm text-muted-foreground">
            Manage project collaboration before entering shared Query Studio.
          </p>
        </div>
        <Sheet open={createOpen} onOpenChange={setCreateOpen}>
          <SheetTrigger
            render={
              <Button type="button" className="h-10 gap-2 rounded-md px-4">
                <FolderPlus className="size-4" />
                Create Project
              </Button>
            }
          />
          <SheetContent side="right" className="w-full sm:max-w-lg">
            <SheetHeader>
              <SheetTitle>Create project</SheetTitle>
              <SheetDescription>
                Define project context and invite collaborators.
              </SheetDescription>
            </SheetHeader>
            <div className="grid gap-4 px-4 pb-4">
              <div className="space-y-2">
                <Label htmlFor="project-name">Project name</Label>
                <Input
                  id="project-name"
                  value={projectName}
                  onChange={(e) => setProjectName(e.target.value)}
                  placeholder="e.g. Fraud Monitoring Q3"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="project-collaborators">
                  Collaborators (comma separated)
                </Label>
                <Textarea
                  id="project-collaborators"
                  value={collaboratorsRaw}
                  onChange={(e) => setCollaboratorsRaw(e.target.value)}
                  className="min-h-[96px]"
                  placeholder="e.g. Sinta Ayu, Bima Putra, Nadia Putri"
                />
              </div>
              <div className="space-y-2">
                <p className="text-xs text-muted-foreground">Collaborator preview</p>
                <div className="flex flex-wrap items-center gap-2">
                  {collaboratorCandidates.length === 0 ? (
                    <p className="text-xs text-muted-foreground">
                      Add one or more collaborator names.
                    </p>
                  ) : (
                    collaboratorCandidates.map((name) => (
                      <Badge key={name} variant="outline">
                        <Users className="size-3" />
                        {name}
                      </Badge>
                    ))
                  )}
                </div>
              </div>
              {error && <p className="text-sm text-destructive">{error}</p>}
              <div className="flex justify-end">
                <Button onClick={createProject}>Create Project</Button>
              </div>
            </div>
          </SheetContent>
        </Sheet>
      </div>

      <Card className="rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
        <CardHeader className="border-b border-border">
          <h2 className="text-base font-medium text-primary">Project list</h2>
          <p className="text-sm text-muted-foreground">
            Projects you own or joined as collaborator.
          </p>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 p-4">
          {projects.map((project) => (
            <div
              key={project.id}
              className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border p-3"
            >
              <div className="space-y-1">
                <p className="font-medium text-primary">{project.name}</p>
                <p className="text-xs text-muted-foreground">{project.description}</p>
                <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                  <span>{project.memberCount} members</span>
                  <span>•</span>
                  <span>Last activity {project.lastActivity}</span>
                </div>
              </div>
              <Button
                size="sm"
                variant="outline"
                render={<Link href={`/query-studio/collaboration/${project.id}`} />}
              >
                Open Collaborative Studio
              </Button>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  )
}
