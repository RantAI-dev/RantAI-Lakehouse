import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test";
import * as React from "react";
import type { Msg } from "./use-copilot";

/**
 * `usePathname` lives in `next/navigation`; the hook reads it for the page
 * context. The happy-dom environment does not provide it, so the module is
 * stubbed before the hook is imported.
 */
mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/main-board",
}));

const { CopilotProvider, useCopilot, continuationText } = await import("./use-copilot");

/** A draft the model made and the person still has to confirm. */
type Draft = { tool: string; args: Record<string, unknown>; summary: string };

/** A step the model ran before its draft, when a case needs one. */
type ReadStep = { tool: string; args: Record<string, unknown>; result: Record<string, unknown> };

/** The draft the answer holds when a case names no other. */
const defaultDraft: Draft = {
  tool: "create_board",
  args: { name: "Overview" },
  summary: "Creates the board Overview.",
};

/** The requests the hook sent to the chat, so a test can read what it asked. */
const chatRequests: { body: string }[] = [];
/** What the tool route answers: a run that succeeded, or one that failed. */
let toolResult: Record<string, unknown> = { outcome: "ok", result: { created: true, id: "b1", name: "Overview" } };
/** What the tool route answers for one tool, when a case needs another payload. */
let toolResults: Record<string, Record<string, unknown>> = {};
/** A second draft in the same answer, when a case needs two steps to confirm. */
let extraDraft: Draft | null = null;
/** The drafts in the same answer, when a case needs other drafts than the default one. */
let drafts: Draft[] = [defaultDraft];
/** The steps the model ran before its draft, when a case needs them. */
let readSteps: ReadStep[] = [];
/** The steps the model says the request still has. */
let nextStep: string | null = "add two charts";

/** A response the hook can read: the body as one chunk, and a JSON body. */
function fakeResponse(body: string, json: unknown): Response {
  const encoder = new TextEncoder();
  let sent = false;
  return {
    ok: true,
    status: 200,
    json: async () => json,
    body: {
      getReader: () => ({
        read: async () => {
          if (sent) return { done: true, value: undefined };
          sent = true;
          return { done: false, value: encoder.encode(body) };
        },
      }),
    },
  } as unknown as Response;
}

/**
 * The chat's answer: the steps the model ran before its draft, a draft that
 * waits for a Confirm, and the steps the request still has. `extraDraft`
 * adds a second draft to the same answer.
 */
function chatBody(): string {
  const list = [...drafts];
  if (extraDraft) list.push(extraDraft);
  const body: Record<string, unknown> = {
    answer: "The board draft is ready.",
    toolTrace: [
      ...readSteps.map((s) => ({ tool: s.tool, args: s.args, ok: true, result: s.result })),
      ...list.map((d) => ({
        tool: d.tool,
        args: d.args,
        ok: true,
        result: {
          needs_confirmation: true,
          tool: d.tool,
          args: d.args,
          summary: d.summary,
        },
      })),
    ],
  };
  if (nextStep) body.nextStep = nextStep;
  return `${JSON.stringify({ type: "done", body })}\n`;
}

const originalFetch = globalThis.fetch;

beforeEach(() => {
  chatRequests.length = 0;
  toolResult = { outcome: "ok", result: { created: true, id: "b1", name: "Overview" } };
  toolResults = {};
  extraDraft = null;
  drafts = [defaultDraft];
  readSteps = [];
  nextStep = "add two charts";
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : String(input);
    const body = typeof init?.body === "string" ? init.body : "";
    if (url.startsWith("/api/ai/chat")) {
      chatRequests.push({ body });
      return fakeResponse(chatBody(), {});
    }
    if (url === "/api/ai/tool") {
      const sent = body ? (JSON.parse(body) as { tool?: string }) : {};
      return fakeResponse("{}", toolResults[sent.tool ?? ""] ?? toolResult);
    }
    if (url.startsWith("/api/ai/sessions")) return fakeResponse("{}", { id: "s1", sessions: [] });
    return fakeResponse("{}", {});
  }) as typeof fetch;
});

afterEach(() => {
  globalThis.fetch = originalFetch;
});

type Copilot = ReturnType<typeof useCopilot>;
let copilot: Copilot | null = null;

function Harness() {
  const api = useCopilot();
  React.useEffect(() => {
    copilot = api;
  }, [api]);
  return null;
}

/**
 * Renders the shared Copilot in Build mode. The hook's value is read from
 * `copilot`, which every render refreshes: the value the provider handed
 * out is a new object each time, so a copy taken here would be stale.
 */
function renderCopilot(): void {
  render(
    <CopilotProvider>
      <Harness />
    </CopilotProvider>,
  );
  act(() => {
    copilot!.setMode("build");
  });
}

/** The messages of the chat request the hook sent. */
function sentMessages(index: number): { role: string; content: string }[] {
  return JSON.parse(chatRequests[index].body).messages;
}

describe("the Copilot goes on after the draft is confirmed (AI-15)", () => {
  it("sends the steps the request still has back as a message", async () => {
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    expect(chatRequests.length).toBe(1);

    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });

    expect(chatRequests.length).toBe(2);
    expect(sentMessages(1).at(-1)).toEqual({
      role: "user",
      content: 'Confirmed and saved: create_board b1 "Overview". Do not draft these again. Continue: add two charts',
    });
  });

  it("shows the continuation in the chat as a message of the person", async () => {
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });
    await waitFor(() => expect(copilot!.messages.length).toBe(4));

    const typed = copilot!.messages.filter((m) => m.role === "user");
    expect(typed.at(-1)?.content).toBe(
      'Confirmed and saved: create_board b1 "Overview". Do not draft these again. Continue: add two charts',
    );
  });

  it("sends nothing when the draft is cancelled", async () => {
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    await act(async () => {
      copilot!.cancelTool(1, 0);
    });

    expect(chatRequests.length).toBe(1);
  });

  it("sends nothing when the confirm fails", async () => {
    toolResult = { outcome: "failed", result: { error: "The board could not be created." } };
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });

    expect(chatRequests.length).toBe(1);
  });

  it("sends at most three continuations after one typed message", async () => {
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    // The typed message, then the answer with the draft; every continuation
    // adds the same two.
    const expected = [4, 6, 8, 8];
    for (const [round, index] of [1, 3, 5, 7].entries()) {
      await act(async () => {
        await copilot!.confirmTool(index, 0);
      });
      await waitFor(() => expect(copilot!.messages.length).toBe(expected[round]));
    }

    expect(chatRequests.length).toBe(4);
    expect(sentMessages(3).at(-1)?.content).toBe(
      'Confirmed and saved: create_board b1 "Overview". Do not draft these again. Continue: add two charts',
    );
  });

  it("names every object the person confirmed, in the order of the steps", async () => {
    extraDraft = {
      tool: "create_chart",
      args: { title: "Visitors", kind: "bar" },
      summary: "Creates the chart Visitors.",
    };
    toolResults = {
      create_chart: { outcome: "ok", result: { created: true, id: "c1", title: "Visitors" } },
    };
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    // The board is confirmed first; the chart still waits for its own
    // Confirm, so the request does not go on yet.
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });
    expect(chatRequests.length).toBe(1);

    await act(async () => {
      await copilot!.confirmTool(1, 1);
    });

    expect(chatRequests.length).toBe(2);
    expect(sentMessages(1).at(-1)?.content).toBe(
      'Confirmed and saved: create_board b1 "Overview"; create_chart c1 "Visitors". Do not draft these again. Continue: add two charts',
    );
  });

  it("names only the step the person confirmed, not the steps that only read", async () => {
    readSteps = [
      { tool: "list_boards", args: {}, result: { boards: [{ id: "b1" }] } },
      { tool: "describe_mart", args: { mart: "sales" }, result: { name: "sales" } },
    ];
    drafts = [
      { tool: "create_chart", args: { title: "Visitors", kind: "bar" }, summary: "Creates the chart Visitors." },
    ];
    toolResults = {
      create_chart: { outcome: "ok", result: { created: true, id: "c1", title: "Visitors" } },
    };
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    // The chart is the third step of the answer: the two reads came first.
    await act(async () => {
      await copilot!.confirmTool(1, 2);
    });

    expect(chatRequests.length).toBe(2);
    expect(sentMessages(1).at(-1)?.content).toBe(
      'Confirmed and saved: create_chart c1 "Visitors". Do not draft these again. Continue: add two charts',
    );
  });

  it("leaves out the id and the title the result does not carry", async () => {
    toolResult = { outcome: "ok", result: { created: true } };
    renderCopilot();

    await act(async () => {
      await copilot!.send("Build me a dashboard about events.");
    });
    await act(async () => {
      await copilot!.confirmTool(1, 0);
    });

    expect(chatRequests.length).toBe(2);
    expect(sentMessages(1).at(-1)?.content).toBe(
      "Confirmed and saved: create_board. Do not draft these again. Continue: add two charts",
    );
  });

  it("names a calculated field by the id and the name under `field`", () => {
    const messages: Msg[] = [
      {
        role: "assistant",
        content: "",
        tools: [
          {
            tool: "create_calculated_field",
            args: {},
            ok: true,
            confirmed: true,
            result: { field: { id: "f1", name: "Margin" } },
          },
        ],
      },
    ];

    expect(continuationText(messages, 0, "add the chart")).toBe(
      'Confirmed and saved: create_calculated_field f1 "Margin". Do not draft these again. Continue: add the chart',
    );
  });
});
