"use client";

/**
 * The app shell's one right side panel, shared by the AI Copilot and a page's
 * own panels (`BI-61` T6, owner rule "one side panel at a time", DESIGN.md).
 *
 * A page registers tabs (the dashboard page: Comments and Details) while it
 * is mounted, and opens one by id. The shell draws them as tabs of the panel
 * the Copilot docks in (`CopilotSidebar`), so opening one replaces what the
 * panel shows and two panels never show side by side. The page keeps
 * rendering the tab's content itself, into the panel's body through a portal
 * (`host`), so the content stays in the page's React tree with its own state
 * and providers.
 *
 * A page that registers nothing sees the shell exactly as before.
 */
import * as React from "react";

import { useCopilot } from "@/features/copilot/use-copilot";

export type SidePanelTab = { readonly id: string; readonly label: string };

type SidePanelValue = {
  /** The tabs the page on screen registered; empty on every other page. */
  readonly tabs: readonly SidePanelTab[];
  /** The page tab the panel shows; null when it shows the Copilot or is closed. */
  readonly active: string | null;
  /** Where the active tab's content goes (`createPortal`); null until the panel has drawn its body. */
  readonly host: HTMLElement | null;
  readonly open: (id: string) => void;
  /** Shows `id`, or closes the panel when `id` is what it already shows: the page's own Comments and Details buttons. */
  readonly toggle: (id: string) => void;
  /** Takes the page tab off the panel; the Copilot shows again if it is docked there and open. */
  readonly close: () => void;
  /**
   * Closes the whole panel: the page tab, and the Copilot too when it is
   * docked there (owner QA 2026-11-15: the panel has no close button, so its
   * page buttons, Escape and a click on its edge do this).
   */
  readonly dismiss: () => void;
  /** For the panel: the element its body is. */
  readonly setHost: (el: HTMLElement | null) => void;
  /** For `useSidePanelTabs`. */
  readonly setTabs: (tabs: readonly SidePanelTab[]) => void;
};

const NO_TABS: readonly SidePanelTab[] = [];

const NOOP: SidePanelValue = {
  tabs: NO_TABS,
  active: null,
  host: null,
  open: () => {},
  toggle: () => {},
  close: () => {},
  dismiss: () => {},
  setHost: () => {},
  setTabs: () => {},
};

const SidePanelContext = React.createContext<SidePanelValue | null>(null);

export function SidePanelProvider({ children }: { readonly children: React.ReactNode }) {
  const copilot = useCopilot();
  const [tabs, setTabsState] = React.useState<readonly SidePanelTab[]>(NO_TABS);
  const [active, setActive] = React.useState<string | null>(null);
  const [host, setHost] = React.useState<HTMLElement | null>(null);
  const { dockPosition, expanded, setExpanded } = copilot;
  const open = React.useCallback((id: string) => {
    setActive(id);
    // The bottom dock's chat rises over the page as a panel of its own; it
    // folds away so only the side panel shows.
    if (dockPosition === "bottom" && expanded) setExpanded(false);
  }, [dockPosition, expanded, setExpanded]);
  const close = React.useCallback(() => setActive(null), []);
  const dismiss = React.useCallback(() => {
    setActive(null);
    if (dockPosition === "right" && expanded) setExpanded(false);
  }, [dockPosition, expanded, setExpanded]);
  const toggle = React.useCallback((id: string) => {
    if (active === id) dismiss();
    else open(id);
  }, [active, dismiss, open]);
  const setTabs = React.useCallback((next: readonly SidePanelTab[]) => {
    setTabsState(next.length ? next : NO_TABS);
    // A page that leaves takes its tabs, and the panel showing one of them, with it.
    setActive((cur) => (cur && next.some((t) => t.id === cur) ? cur : null));
  }, []);
  // The Copilot opening (its button, the dock, a page asking it to) replaces a page tab.
  const wasExpanded = React.useRef(expanded);
  React.useEffect(() => {
    if (expanded && !wasExpanded.current) setActive(null);
    wasExpanded.current = expanded;
  }, [expanded]);
  const value = React.useMemo(
    () => ({ tabs, active, host, open, toggle, close, dismiss, setHost, setTabs }),
    [tabs, active, host, open, toggle, close, dismiss, setTabs],
  );
  return <SidePanelContext.Provider value={value}>{children}</SidePanelContext.Provider>;
}

/** The shared panel; outside the shell (a test, a public page) a no-op that never opens. */
export function useSidePanel(): SidePanelValue {
  return React.useContext(SidePanelContext) ?? NOOP;
}

/** Registers `tabs` while the calling page is mounted. Pass a stable list (or an equal one: it is compared by value). */
export function useSidePanelTabs(tabs: readonly SidePanelTab[]): void {
  const { setTabs } = useSidePanel();
  const key = JSON.stringify(tabs);
  React.useEffect(() => {
    setTabs(JSON.parse(key) as SidePanelTab[]);
    return () => setTabs(NO_TABS);
  }, [key, setTabs]);
}
