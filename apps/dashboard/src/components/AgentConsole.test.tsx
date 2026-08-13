import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchAgentSessions } from "../api";
import type { AgentSession } from "../types";
import { AgentConsole } from "./AgentConsole";

vi.mock("../api", () => ({
  agentSessionStreamUrl: (id: number) => `/api/sessions/${id}/stream`,
  fetchAgentSessions: vi.fn(),
}));

class FakeEventSource {
  static latest: FakeEventSource | null = null;
  onerror: ((event: Event) => void) | null = null;
  readonly url: string;
  closed = false;

  constructor(url: string | URL) {
    this.url = String(url);
    FakeEventSource.latest = this;
  }

  addEventListener() {}

  close() {
    this.closed = true;
  }
}

const meta = {
  command: "codex exec",
  available: true,
  roles: ["worker"],
};

function session(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id: 12,
    runId: 7,
    taskId: 17,
    attemptId: null,
    role: "worker",
    agent: "codex",
    command: "codex exec",
    status: "success",
    startedAt: "2026-08-13T08:00:00Z",
    finishedAt: "2026-08-13T08:00:02Z",
    exitCode: 0,
    durationMs: 2_000,
    stdout: "compiled\n",
    stderr: "warning\n",
    workingDirectory: "D:\\factory",
    interactive: false,
    ...overrides,
  };
}

beforeEach(() => {
  vi.mocked(fetchAgentSessions).mockReset();
  FakeEventSource.latest = null;
  vi.stubGlobal("EventSource", FakeEventSource);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("Agent Console", () => {
  it("shows an honest idle state when the configured agent has no sessions", async () => {
    vi.mocked(fetchAgentSessions).mockResolvedValue([]);
    render(
      <AgentConsole
        agentName="codex"
        meta={meta}
        activity={null}
        onClose={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(await screen.findByText("No active session.")).toBeTruthy();
    expect(screen.getByText("This configured agent is currently idle.")).toBeTruthy();
    expect(screen.getByText("Available")).toBeTruthy();
  });

  it("renders persisted completed stdout and stderr without fabricating output", async () => {
    vi.mocked(fetchAgentSessions).mockResolvedValue([session()]);
    render(
      <AgentConsole
        agentName="codex"
        meta={meta}
        activity={null}
        onClose={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(await screen.findByText("compiled")).toBeTruthy();
    expect(screen.getByText("warning")).toBeTruthy();
    expect(screen.getByText("Completed")).toBeTruthy();
    expect(screen.getByText("Exit code 0")).toBeTruthy();
    expect(screen.getByText("This agent session is non-interactive.")).toBeTruthy();
  });

  it("opens the scoped stream for an active session", async () => {
    vi.mocked(fetchAgentSessions).mockResolvedValue([
      session({ status: "running", finishedAt: null, exitCode: null, durationMs: null }),
    ]);
    render(
      <AgentConsole
        agentName="codex"
        meta={meta}
        activity={{ runId: 7, taskId: 17 }}
        onClose={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(await screen.findByText("Running")).toBeTruthy();
    await waitFor(() => expect(FakeEventSource.latest?.url).toBe("/api/sessions/12/stream"));
  });

  it("reports a disconnected live stream inline", async () => {
    vi.mocked(fetchAgentSessions).mockResolvedValue([
      session({ status: "running", finishedAt: null, exitCode: null, durationMs: null }),
    ]);
    render(
      <AgentConsole
        agentName="codex"
        meta={meta}
        activity={null}
        onClose={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    await waitFor(() => expect(FakeEventSource.latest).not.toBeNull());
    act(() => FakeEventSource.latest?.onerror?.(new Event("error")));
    expect(await screen.findByText("The agent session stream disconnected.")).toBeTruthy();
    expect(FakeEventSource.latest?.closed).toBe(true);
  });

  it("surfaces session lookup failures", async () => {
    vi.mocked(fetchAgentSessions).mockRejectedValue(new Error("session not found"));
    render(
      <AgentConsole
        agentName="codex"
        meta={meta}
        activity={null}
        onClose={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(await screen.findByText("session not found")).toBeTruthy();
  });

  it("provides a keyboard-accessible custom connection fallback in Overview", async () => {
    vi.mocked(fetchAgentSessions).mockResolvedValue([]);
    const onConnect = vi.fn();
    const nodesById = new Map([
      [
        "agent:codex",
        {
          id: "agent:codex",
          kind: "agent" as const,
          label: "Codex",
          meta,
        },
      ],
      [
        "agent:opencode",
        {
          id: "agent:opencode",
          kind: "agent" as const,
          label: "OpenCode",
          meta: { command: "opencode run", available: true, roles: ["worker"] },
        },
      ],
    ]);
    render(
      <AgentConsole
        agentName="codex"
        meta={meta}
        activity={null}
        nodesById={nodesById}
        onClose={vi.fn()}
        onDelete={vi.fn()}
        onConnect={onConnect}
      />
    );

    fireEvent.click(screen.getByRole("tab", { name: "Overview" }));
    fireEvent.change(screen.getByLabelText("Add supported connection"), {
      target: { value: "agent:opencode" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(onConnect).toHaveBeenCalledWith("agent:opencode");
  });
});
