"use client"

import {
  AtSign,
  Bot,
  BookMarked,
  Check,
  ChevronRight,
  FileUp,
  Plus,
} from "lucide-react"
import { Menu } from "@base-ui/react/menu"
import { cn } from "@/lib/utils"
import {
  AGENT_MODEL_OPTIONS,
  KNOWLEDGE_LIBRARY_INITIAL_ENTRIES,
  SCHEMA_TABLE_MENTIONS,
  type AgentModelId,
} from "@/lib/knowledge-library-fixtures"

const menuItemClass =
  "flex cursor-pointer items-center gap-2 rounded-md px-2 py-2 text-sm outline-none select-none data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground"

const menuPopupClass =
  "z-50 min-w-[260px] max-w-[min(100vw-24px,320px)] origin-[var(--transform-origin)] rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md outline-none"

export type PromptActionsMenuProps = {
  disabled?: boolean
  onUploadDocument: () => void
  agentModel: AgentModelId
  onAgentModelChange: (id: AgentModelId) => void
  useIntelligenceKnowledge: boolean
  onUseIntelligenceKnowledgeChange: (v: boolean) => void
  selectedKnowledgeIds: string[]
  onToggleKnowledgeId: (id: string, selected: boolean) => void
  onMentionToken: (token: string) => void
}

/**
 * "+" menu attached to the natural-language prompt in Query Studio.
 *
 * Lets the user:
 * 1. Upload a document (delegates to `onUploadDocument`).
 * 2. Pick the agent model (radio submenu, controlled via `agentModel` /
 *    `onAgentModelChange`).
 * 3. Toggle "Use Intelligence Knowledge" and, when enabled, select which
 *    library entries to attach (`selectedKnowledgeIds` / `onToggleKnowledgeId`).
 * 4. Mention a table or schema by token (calls `onMentionToken` to insert
 *    `@token` into the prompt).
 *
 * The component is purely presentational: all state lives in the parent.
 */
export function PromptActionsMenu({
  disabled,
  onUploadDocument,
  agentModel,
  onAgentModelChange,
  useIntelligenceKnowledge,
  onUseIntelligenceKnowledgeChange,
  selectedKnowledgeIds,
  onToggleKnowledgeId,
  onMentionToken,
}: PromptActionsMenuProps) {
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger
        type="button"
        disabled={disabled}
        className={cn(
          "flex size-8 shrink-0 items-center justify-center rounded-full text-primary hover:bg-muted disabled:opacity-40",
          "data-[popup-open]:bg-muted"
        )}
        aria-label="Prompt actions: upload, model, knowledge, mentions"
      >
        <Plus className="size-5" strokeWidth={2} />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Positioner side="top" align="start" sideOffset={8}>
          <Menu.Popup className={menuPopupClass}>
            <Menu.Item
              className={menuItemClass}
              onClick={onUploadDocument}
              closeOnClick
            >
              <FileUp className="size-4 shrink-0 text-primary" aria-hidden />
              Upload document
            </Menu.Item>

            <Menu.Separator className="my-1 h-px bg-border" />

            <Menu.SubmenuRoot>
              <Menu.SubmenuTrigger className={cn(menuItemClass, "justify-between")}>
                <span className="flex items-center gap-2">
                  <Bot className="size-4 shrink-0 text-primary" aria-hidden />
                  Agent model
                </span>
                <ChevronRight className="size-4 shrink-0 opacity-60" aria-hidden />
              </Menu.SubmenuTrigger>
              <Menu.Portal>
                <Menu.Positioner side="right" align="start" sideOffset={4}>
                  <Menu.Popup className={menuPopupClass}>
                    <Menu.RadioGroup
                      value={agentModel}
                      onValueChange={(v) => onAgentModelChange(v as AgentModelId)}
                    >
                      {AGENT_MODEL_OPTIONS.map((opt) => (
                        <Menu.RadioItem
                          key={opt.id}
                          value={opt.id}
                          className={menuItemClass}
                          closeOnClick
                        >
                          {opt.label}
                        </Menu.RadioItem>
                      ))}
                    </Menu.RadioGroup>
                  </Menu.Popup>
                </Menu.Positioner>
              </Menu.Portal>
            </Menu.SubmenuRoot>

            <Menu.Separator className="my-1 h-px bg-border" />

            <Menu.CheckboxItem
              className={cn(menuItemClass, "gap-2")}
              checked={useIntelligenceKnowledge}
              onCheckedChange={(checked) =>
                onUseIntelligenceKnowledgeChange(Boolean(checked))
              }
              closeOnClick={false}
            >
              <Menu.CheckboxItemIndicator className="flex size-4 items-center justify-center rounded-[3px] border border-border data-[checked]:border-primary data-[checked]:bg-primary/15">
                <Check className="size-3 text-primary" strokeWidth={3} />
              </Menu.CheckboxItemIndicator>
              <BookMarked className="size-4 shrink-0 text-primary" aria-hidden />
              Use Intelligence Knowledge
            </Menu.CheckboxItem>

            {useIntelligenceKnowledge && (
              <Menu.Group>
                <Menu.GroupLabel className="px-2 py-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                  Knowledge library
                </Menu.GroupLabel>
                <div className="max-h-[200px] overflow-y-auto">
                  {KNOWLEDGE_LIBRARY_INITIAL_ENTRIES.map((k) => {
                    const checked = selectedKnowledgeIds.includes(k.id)
                    return (
                      <Menu.CheckboxItem
                        key={k.id}
                        className={cn(
                          menuItemClass,
                          "gap-2 pl-2",
                          !useIntelligenceKnowledge && "opacity-50"
                        )}
                        checked={checked}
                        disabled={!useIntelligenceKnowledge}
                        onCheckedChange={(v) => onToggleKnowledgeId(k.id, Boolean(v))}
                        closeOnClick={false}
                      >
                        <Menu.CheckboxItemIndicator className="flex size-4 shrink-0 items-center justify-center rounded-[3px] border border-border data-[checked]:border-primary data-[checked]:bg-primary/15">
                          <Check className="size-3 text-primary" strokeWidth={3} />
                        </Menu.CheckboxItemIndicator>
                        <span className="min-w-0 flex-1 truncate text-left">
                          <span className="block truncate font-medium">
                            {k.title}
                          </span>
                          <span className="block truncate text-[11px] text-muted-foreground">
                            {k.zone}
                          </span>
                        </span>
                      </Menu.CheckboxItem>
                    )
                  })}
                </div>
              </Menu.Group>
            )}

            <Menu.Separator className="my-1 h-px bg-border" />

            <Menu.SubmenuRoot>
              <Menu.SubmenuTrigger className={cn(menuItemClass, "justify-between")}>
                <span className="flex items-center gap-2">
                  <AtSign className="size-4 shrink-0 text-primary" aria-hidden />
                  Mention table or schema
                </span>
                <ChevronRight className="size-4 shrink-0 opacity-60" aria-hidden />
              </Menu.SubmenuTrigger>
              <Menu.Portal>
                <Menu.Positioner side="right" align="start" sideOffset={4}>
                  <Menu.Popup className={menuPopupClass}>
                    <div className="max-h-[220px] overflow-y-auto">
                      {SCHEMA_TABLE_MENTIONS.map((m) => (
                        <Menu.Item
                          key={m.id}
                          className={menuItemClass}
                          onClick={() => onMentionToken(m.token)}
                          closeOnClick
                        >
                          <span className="min-w-0 truncate text-left">
                            {m.label}
                          </span>
                        </Menu.Item>
                      ))}
                    </div>
                  </Menu.Popup>
                </Menu.Positioner>
              </Menu.Portal>
            </Menu.SubmenuRoot>
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  )
}
