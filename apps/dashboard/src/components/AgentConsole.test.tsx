import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchAgentPerformance, fetchAgentSessions, startInteractiveAgentSession } from "../api";
import type { AgentPerformanceDetail, AgentSession } from "../types";
import { AgentConsole } from "./AgentConsole";

vi.mock("../api", () => ({
  agentSessionStreamUrl: (id: number) => `/api/sessions/${id}/stream`,
  agentTerminalSocketUrl: (id: number) => `/api/sessions/${id}/terminal`,
  fetchAgentSessions: vi.fn(),
  fetchAgentPerformance: vi.fn(),
  startInteractiveAgentSession: vi.fn(),
  stopInteractiveAgentSession: vi.fn(),
}));

vi.mock("./InteractiveTerminal", () => ({
  InteractiveTerminal: ({ sessionId }: { sessionId: number }) => (
    <div aria-label="Interactive terminal">session {sessionId}</div>
  ),
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
  kind: "codex" as const,
  workflowAvailable: true,
  interactiveAvailable: true,
  resolvedExecutable: "C:\\tools\\codex.cmd",
  resolutionError: null,
  pathEntriesChecked: 14,
  roles: ["worker"],
};

function session(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id: 12,
    runId: 7,
    taskId: 17,
    attemptId: null,
    role: "worker",
    operation: null,
    agent: "codex",
    mode: "automated",
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
  vi.mocked(fetchAgentPerformance).mockReset().mockRejectedValue(new Error("no history"));
  vi.mocked(startInteractiveAgentSession).mockReset();
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

    expect(await screen.findByText("No active interactive session.")).toBeTruthy();
    expect(screen.getByText("This configured agent is currently idle.")).toBeTruthy();
    expect(screen.getByText("Available")).toBeTruthy();
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Start session" }).disabled).toBe(
      false
    );
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
    expect(
      screen.getByText("This workflow session is non-interactive and Factory-controlled.")
    ).toBeTruthy();
  });

  it("starts a scoped interactive session on explicit user action", async () => {
    vi.mocked(fetchAgentSessions).mockResolvedValue([]);
    vi.mocked(startInteractiveAgentSession).mockResolvedValue(
      session({
        id: 44,
        mode: "interactive",
        interactive: true,
        role: "console",
        status: "running",
        runId: null,
        taskId: null,
        exitCode: null,
        finishedAt: null,
      })
    );
    render(
      <AgentConsole
        agentName="codex"
        meta={meta}
        activity={null}
        onClose={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "Start session" }));
    expect(await screen.findByLabelText("Interactive terminal")).toBeTruthy();
    expect(startInteractiveAgentSession).toHaveBeenCalledWith("codex");
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
          meta: {
            command: "opencode run",
            available: true,
            workflowAvailable: true,
            interactiveAvailable: true,
            roles: ["worker"],
          },
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
    expect(screen.getByText("C:\\tools\\codex.cmd")).toBeTruthy();
    expect(screen.getByText("Factory process PATH (14 entries checked)")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Add supported connection"), {
      target: { value: "agent:opencode" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(onConnect).toHaveBeenCalledWith("agent:opencode");
  });

  it("shows a compact performance summary on the overview tab when history exists", async () => {
    vi.mocked(fetchAgentSessions).mockResolvedValue([]);
    const detail = {
      summary: {
        agent: "codex",
        metrics: {
          tasksAttempted: 86,
          attempts: 95,
          attemptsPerTask: 1.1,
          avgAttemptsPerSuccessful: 1.12,
          qualifyingTasks: 86,
          outcomeCounts: {
            approved: 84,
            firstPassApproved: 80,
            changesRequested: 2,
            agentFailed: 0,
            integrationConflict: 0,
            cancelled: 2,
            interrupted: 0,
            policyBlocked: 0,
            configurationError: 0,
            inProgress: 0,
          },
          firstPassApproval: {
            successes: 80,
            total: 86,
            rate: 80 / 86,
            intervalLow: 0.84,
            intervalHigh: 0.97,
            reliable: true,
          },
          eventualApproval: {
            successes: 84,
            total: 86,
            rate: 84 / 86,
            intervalLow: 0.9,
            intervalHigh: 0.99,
            reliable: true,
          },
          requestChanges: {
            successes: 6,
            total: 86,
            rate: 0.07,
            intervalLow: 0,
            intervalHigh: 1,
            reliable: true,
          },
          retryRate: {
            successes: 6,
            total: 86,
            rate: 0.07,
            intervalLow: 0,
            intervalHigh: 1,
            reliable: true,
          },
          terminalFailure: {
            successes: 0,
            total: 86,
            rate: 0,
            intervalLow: 0,
            intervalHigh: 0,
            reliable: true,
          },
          executionDuration: {
            samples: 86,
            medianMs: 94_000,
            p95Ms: 200_000,
            approximateSamples: 0,
            reliable: true,
          },
          reviewDuration: {
            samples: 84,
            medianMs: 18_000,
            p95Ms: 60_000,
            approximateSamples: 0,
            reliable: true,
          },
          totalDuration: {
            samples: 86,
            medianMs: 130_000,
            p95Ms: 300_000,
            approximateSamples: 0,
            reliable: true,
          },
          integration: {
            clean: 82,
            rebased: 2,
            conflict: 1,
            cleanRate: {
              successes: 82,
              total: 85,
              rate: 0.96,
              intervalLow: 0.9,
              intervalHigh: 0.99,
              reliable: true,
            },
            conflictRate: {
              successes: 1,
              total: 85,
              rate: 0.01,
              intervalLow: 0,
              intervalHigh: 0.06,
              reliable: true,
            },
          },
        },
      },
      byRole: [],
      byOperation: [],
      byLanguage: [],
      trend: {
        recent10: {
          label: "Recent 10 tasks",
          firstPass: {
            successes: 9,
            total: 10,
            rate: 0.9,
            intervalLow: 0.6,
            intervalHigh: 0.98,
            reliable: true,
          },
          medianExecutionMs: 90_000,
        },
        recent25: {
          label: "Recent 25 tasks",
          firstPass: {
            successes: 22,
            total: 25,
            rate: 0.88,
            intervalLow: 0.7,
            intervalHigh: 0.96,
            reliable: true,
          },
          medianExecutionMs: 95_000,
        },
        weekly: null,
      },
      reworkReasons: [],
      failureReasons: [],
    } satisfies AgentPerformanceDetail;
    vi.mocked(fetchAgentPerformance).mockResolvedValue(detail);

    render(
      <AgentConsole
        agentName="codex"
        meta={meta}
        activity={null}
        onClose={vi.fn()}
        onDelete={vi.fn()}
      />
    );
    fireEvent.click(await screen.findByRole("tab", { name: "Overview" }));

    expect(await screen.findByText("Performance")).toBeTruthy();
    expect(screen.getByText("86")).toBeTruthy();
    expect(screen.getByText("93%")).toBeTruthy();
    expect(screen.getByText("1m 34s")).toBeTruthy();
    expect(screen.getByText("1.10")).toBeTruthy();
    expect(screen.getByRole("button", { name: "View details" })).toBeTruthy();
  });

  it("omits the performance section when the agent has no history", async () => {
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
    fireEvent.click(await screen.findByRole("tab", { name: "Overview" }));

    await waitFor(() => expect(fetchAgentPerformance).toHaveBeenCalled());
    expect(screen.queryByText("View details")).toBeNull();
  });
});
