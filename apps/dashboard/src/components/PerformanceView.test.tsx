import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchAgentPerformance, fetchPerformanceOverview } from "../api";
import type { AgentPerformanceDetail, PerformanceOverview } from "../types";
import { PerformanceView } from "./PerformanceView";

vi.mock("../api", () => ({
  fetchPerformanceOverview: vi.fn(),
  fetchAgentPerformance: vi.fn(),
}));

const codexSummary = {
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
      intervalLow: 0.03,
      intervalHigh: 0.14,
      reliable: true,
    },
    retryRate: {
      successes: 6,
      total: 86,
      rate: 0.07,
      intervalLow: 0.03,
      intervalHigh: 0.14,
      reliable: true,
    },
    terminalFailure: {
      successes: 0,
      total: 86,
      rate: 0,
      intervalLow: 0,
      intervalHigh: 0.04,
      reliable: true,
    },
    executionDuration: {
      samples: 86,
      medianMs: 94_000,
      p95Ms: 210_000,
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
        rate: 82 / 85,
        intervalLow: 0.9,
        intervalHigh: 0.99,
        reliable: true,
      },
      conflictRate: {
        successes: 1,
        total: 85,
        rate: 1 / 85,
        intervalLow: 0,
        intervalHigh: 0.06,
        reliable: true,
      },
    },
  },
};

const qwenSummary = {
  agent: "qwen",
  metrics: {
    ...codexSummary.metrics,
    tasksAttempted: 2,
    attempts: 2,
    attemptsPerTask: 1,
    avgAttemptsPerSuccessful: 1,
    qualifyingTasks: 2,
    firstPassApproval: {
      successes: 2,
      total: 2,
      rate: 1,
      intervalLow: 0.34,
      intervalHigh: 1,
      reliable: false,
    },
  },
};

const overview: PerformanceOverview = {
  window: "all_time",
  agents: [codexSummary, qwenSummary],
  facets: {
    roles: ["worker"],
    operations: ["implement"],
    languages: ["rust", "typescript"],
  },
};

const detail: AgentPerformanceDetail = {
  summary: codexSummary,
  byRole: [{ dimension: "role", key: "worker", metrics: codexSummary.metrics }],
  byOperation: [{ dimension: "operation", key: "implement", metrics: codexSummary.metrics }],
  byLanguage: [{ dimension: "language", key: "rust", metrics: codexSummary.metrics }],
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
    weekly: {
      current: {
        label: "Last 7 days",
        firstPass: {
          successes: 18,
          total: 20,
          rate: 0.9,
          intervalLow: 0.7,
          intervalHigh: 0.97,
          reliable: true,
        },
        medianExecutionMs: 92_000,
      },
      previous: {
        label: "Previous 7 days",
        firstPass: {
          successes: 10,
          total: 20,
          rate: 0.5,
          intervalLow: 0.3,
          intervalHigh: 0.7,
          reliable: true,
        },
        medianExecutionMs: 101_000,
      },
      firstPassDeltaPp: 40,
    },
  },
  reworkReasons: [{ reason: "missing unit tests", count: 4 }],
  failureReasons: [],
};

beforeEach(() => {
  vi.mocked(fetchPerformanceOverview).mockReset().mockResolvedValue(overview);
  vi.mocked(fetchAgentPerformance).mockReset().mockResolvedValue(detail);
});

afterEach(cleanup);

describe("PerformanceView", () => {
  it("renders the overview table with reliable rates and sample caveats", async () => {
    render(<PerformanceView />);

    expect(await screen.findByText("codex")).toBeTruthy();
    expect(screen.getByText("93%")).toBeTruthy();
    // qwen has n=2: no percentage, an explicit insufficient-data marker.
    expect(screen.getByText("Insufficient data (n=2)")).toBeTruthy();
    expect(screen.getAllByText("1m 34s").length).toBeGreaterThan(0);
  });

  it("opens an agent detail with breakdowns, trend and reasons", async () => {
    render(<PerformanceView />);

    fireEvent.click(await screen.findByText("codex"));
    expect(await screen.findByText("Back to overview")).toBeTruthy();
    expect(fetchAgentPerformance).toHaveBeenCalledWith("codex", expect.anything());

    expect(screen.getByText("By role")).toBeTruthy();
    expect(screen.getByText("By operation")).toBeTruthy();
    expect(screen.getByText("By language")).toBeTruthy();
    expect(screen.getByText("Recent 10 tasks")).toBeTruthy();
    expect(screen.getByText(/\+40 pp/)).toBeTruthy();
    expect(screen.getByText("missing unit tests")).toBeTruthy();
    // The Wilson interval is surfaced in the detail view.
    expect(screen.getByText("93% (84–97%)")).toBeTruthy();
  });

  it("marks the selected agent row and returns to the overview", async () => {
    render(<PerformanceView />);

    fireEvent.click(await screen.findByText("codex"));
    await screen.findByText("Back to overview");
    fireEvent.click(screen.getByText("Back to overview"));
    await waitFor(() => expect(screen.queryByText("Back to overview")).toBeNull());
  });

  it("loads a deep-linked agent directly", async () => {
    render(<PerformanceView initialAgent="codex" />);

    expect(await screen.findByText("Back to overview")).toBeTruthy();
  });

  it("surfaces connection errors", async () => {
    vi.mocked(fetchPerformanceOverview).mockRejectedValue(
      new Error("Factory API did not respond.")
    );

    render(<PerformanceView />);

    expect(await screen.findByText("Factory API did not respond.")).toBeTruthy();
  });
});
