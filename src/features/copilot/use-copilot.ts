"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import type { ToolStep } from "./tool-step";
import { ALL_CAP_KEYS, toolsFromCaps } from "./capabilities";
import { derivePageContext, type PageContext } from "./page-context";

export type Mode = "ask" | "build";
export type Msg = {
  role: "user" | "assistant";
  content: string;
  tools?: ToolStep[];
  buildRunId?: string;
  chartCreated?: boolean;
};
export type SessionMeta = { id: string; title: string; mode: string; updatedAt?: string };

/**
 * Otak AI Copilot yang DIPAKAI BERSAMA (lewat context) oleh chat dock global,
 * halaman /copilot, DAN daftar riwayat di sidebar. Mengurus percakapan (mode
 * Ask/Build, kirim, tool loop) DAN riwayat (simpan/muat sesi dari
 * console.chat_session). Satu instance → semua tampilan konsisten.
 */
function useCopilotState() {
  const [mode, setMode] = React.useState<Mode>("ask");
  const [messages, setMessages] = React.useState<Msg[]>([]);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [sessionId, setSessionId] = React.useState<string | null>(null);
  const [sessions, setSessions] = React.useState<SessionMeta[]>([]);
  const [enabledCaps, setEnabledCaps] = React.useState<Set<string>>(() => new Set(ALL_CAP_KEYS));
  const pathname = usePathname();
  const [pageOverride, setPageOverride] = React.useState<PageContext | null>(null);
  const pageContext = pageOverride ?? derivePageContext(pathname);
  const pageContextRef = React.useRef(pageContext);
  pageContextRef.current = pageContext;
  const setPageContext = React.useCallback((ctx: PageContext | null) => setPageOverride(ctx), []);

  const toggleCap = React.useCallback((key: string) => {
    setEnabledCaps((prev) => {
      const n = new Set(prev);
      if (n.has(key)) n.delete(key); else n.add(key);
      return n;
    });
  }, []);

  const refreshSessions = React.useCallback(async () => {
    try {
      const res = await fetch("/api/ai/sessions", { cache: "no-store" });
      const json = await res.json();
      if (Array.isArray(json.sessions)) setSessions(json.sessions);
    } catch { /* abaikan */ }
  }, []);

  React.useEffect(() => { void refreshSessions(); }, [refreshSessions]);

  const persist = React.useCallback(async (msgs: Msg[], m: Mode, id: string | null) => {
    try {
      const res = await fetch("/api/ai/sessions", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id: id ?? undefined, mode: m, messages: msgs }),
      });
      const json = await res.json();
      if (json.id) setSessionId(json.id);
      void refreshSessions();
    } catch { /* abaikan */ }
  }, [refreshSessions]);

  const send = React.useCallback(async (text: string) => {
    const q = text.trim();
    if (!q || busy) return;
    setError(null);
    const next: Msg[] = [...messages, { role: "user", content: q }];
    setMessages(next);
    setBusy(true);
    try {
      const res = await fetch("/api/ai/chat", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          mode,
          tools: toolsFromCaps(enabledCaps, mode),
          context: pageContextRef.current?.system,
          messages: next.map((m) => ({ role: m.role, content: m.content })),
        }),
      });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.hint ?? json?.detail ?? json?.error ?? "Copilot gagal");
      const full: Msg[] = [
        ...next,
        {
          role: "assistant", content: json.answer || "(no answer)",
          tools: json.toolTrace ?? [], buildRunId: json.buildRunId, chartCreated: json.chartCreated,
        },
      ];
      setMessages(full);
      void persist(full, mode, sessionId);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [busy, messages, mode, sessionId, persist, enabledCaps]);

  const newChat = React.useCallback(() => {
    setMessages([]); setSessionId(null); setError(null);
  }, []);

  const loadSession = React.useCallback(async (id: string) => {
    setError(null);
    try {
      const res = await fetch(`/api/ai/sessions?id=${encodeURIComponent(id)}`, { cache: "no-store" });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.error ?? "Failed to load session");
      setMessages((json.session.messages ?? []) as Msg[]);
      if (json.session.mode === "build" || json.session.mode === "ask") setMode(json.session.mode);
      setSessionId(json.session.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const removeSession = React.useCallback(async (id: string) => {
    await fetch(`/api/ai/sessions?id=${encodeURIComponent(id)}`, { method: "DELETE" });
    if (id === sessionId) newChat();
    void refreshSessions();
  }, [sessionId, newChat, refreshSessions]);

  return {
    mode, setMode, messages, busy, error, sessionId, sessions,
    enabledCaps, toggleCap, pageContext, setPageContext,
    send, newChat, loadSession, removeSession, refreshSessions,
  };
}

type CopilotValue = ReturnType<typeof useCopilotState>;
const CopilotContext = React.createContext<CopilotValue | null>(null);

/** Provider tunggal — bungkus app agar dock/halaman/sidebar berbagi 1 percakapan. */
export function CopilotProvider({ children }: { children: React.ReactNode }) {
  const value = useCopilotState();
  return React.createElement(CopilotContext.Provider, { value }, children);
}

/** Akses otak Copilot bersama. */
export function useCopilot(): CopilotValue {
  const ctx = React.useContext(CopilotContext);
  if (!ctx) throw new Error("useCopilot must be used within a CopilotProvider");
  return ctx;
}
