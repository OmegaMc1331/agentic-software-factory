import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchRun, fetchRunsPage } from "./api";
import App from "./App";
import type { RunDetail, RunPage, RunSummary } from "./types";

vi.mock("./api", () => ({
  fetchRun: vi.fn(),
  fetchRunsPage: vi.fn(),
  RUN_PAGE_SIZE: 25,
  progress: ({ completed, total }: { completed: number; total: number }) =>
    total === 0 ? 0 : completed / total,
}));

const run: RunSummary = {
  id: 1,
  objective: "Ship the runtime fix",
  status: "planned",
  plannerAgent: "codex",
  createdAt: "2026-08-13T00:00:00Z",
  counts: {
    pending: 0,
    ready: 0,
    running: 0,
    blocked: 0,
    failed: 0,
    completed: 0,
    total: 0,
  },
};

function page(items: RunSummary[], nextCursor: number | null = null, hasMore = false): RunPage {
  return { items, nextCursor, hasMore };
}

function runDetail(id: number, objective: string): RunDetail {
  return {
    run: {
      id,
      objective,
      status: "planned",
      plannerAgent: "codex",
      team: null,
      createdAt: "2026-08-13T00:00:00Z",
      updatedAt: "2026-08-13T00:00:00Z",
    },
    tasks: [],
    attempts: [],
    sessions: [],
    stages: [],
    artifacts: [],
    integration: { branch: `factory/run-${id}`, head: null, integratedTasks: [] },
  };
}

beforeEach(() => {
  window.location.hash = "#/";
  vi.mocked(fetchRunsPage).mockReset();
  vi.mocked(fetchRun).mockReset();
});

afterEach(cleanup);

describe("Runs loading state", () => {
  it("renders an empty ready state when there are no runs", async () => {
    vi.mocked(fetchRunsPage).mockResolvedValue(page([]));
    render(<App />);

    expect(await screen.findByText("No runs yet")).toBeTruthy();
  });

  it("renders existing runs", async () => {
    vi.mocked(fetchRunsPage).mockResolvedValue(page([run]));
    render(<App />);

    expect(await screen.findByText("Ship the runtime fix")).toBeTruthy();
  });

  it("stops loading after failure and retries through loadRuns", async () => {
    vi.mocked(fetchRunsPage)
      .mockRejectedValueOnce(new Error("Factory API did not respond."))
      .mockResolvedValueOnce(page([]));
    render(<App />);

    expect(await screen.findByText("Could not connect to the Factory API.")).toBeTruthy();
    expect(screen.queryByText("Loading…")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(fetchRunsPage).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("No runs yet")).toBeTruthy();
  });
});

describe("Runs pagination", () => {
  it("loads the next page with the cursor and returns through the cursor stack", async () => {
    const newest = { ...run, id: 3, objective: "Newest run" };
    const older = { ...run, id: 2, objective: "Older run" };
    vi.mocked(fetchRunsPage)
      .mockResolvedValueOnce(page([newest], 3, true))
      .mockResolvedValueOnce(page([older]))
      .mockResolvedValueOnce(page([newest], 3, true));
    render(<App />);

    expect(await screen.findByText("Newest run")).toBeTruthy();
    expect(fetchRunsPage).toHaveBeenNthCalledWith(
      1,
      { limit: 25, before: null },
      expect.objectContaining({ signal: expect.anything() })
    );

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByText("Older run")).toBeTruthy();
    expect(fetchRunsPage).toHaveBeenNthCalledWith(
      2,
      { limit: 25, before: 3 },
      expect.objectContaining({ signal: expect.anything() })
    );
    expect(screen.getByText("Page 2")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Next" })).toHaveProperty("disabled", true);

    fireEvent.click(screen.getByRole("button", { name: "Previous" }));
    expect(await screen.findByText("Newest run")).toBeTruthy();
    expect(fetchRunsPage).toHaveBeenNthCalledWith(
      3,
      { limit: 25, before: null },
      expect.objectContaining({ signal: expect.anything() })
    );
    expect(screen.getByText("Page 1")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Previous" })).toHaveProperty("disabled", true);
  });

  it("keeps the current page when opening a run and returning", async () => {
    const newest = { ...run, id: 3, objective: "Newest run" };
    const older = { ...run, id: 2, objective: "Older run" };
    vi.mocked(fetchRunsPage)
      .mockResolvedValueOnce(page([newest], 3, true))
      .mockResolvedValueOnce(page([older]));
    vi.mocked(fetchRun).mockResolvedValue(runDetail(2, "Older run"));
    render(<App />);

    expect(await screen.findByText("Newest run")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByText("Older run")).toBeTruthy();

    fireEvent.click(screen.getByText("Older run"));
    expect(await screen.findByText("Run #2")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /All runs/ }));
    expect(await screen.findByText("Older run")).toBeTruthy();
    expect(screen.getByText("Page 2")).toBeTruthy();
    expect(fetchRunsPage).toHaveBeenCalledTimes(2);
  });
});
