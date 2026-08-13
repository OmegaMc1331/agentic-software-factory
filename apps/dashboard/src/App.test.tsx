import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchRuns } from "./api";
import App from "./App";
import type { RunSummary } from "./types";

vi.mock("./api", () => ({
  fetchRun: vi.fn(),
  fetchRuns: vi.fn(),
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

beforeEach(() => {
  window.location.hash = "#/";
  vi.mocked(fetchRuns).mockReset();
});

afterEach(cleanup);

describe("Runs loading state", () => {
  it("renders an empty ready state when there are no runs", async () => {
    vi.mocked(fetchRuns).mockResolvedValue([]);
    render(<App />);

    expect(await screen.findByText("No runs yet")).toBeTruthy();
  });

  it("renders existing runs", async () => {
    vi.mocked(fetchRuns).mockResolvedValue([run]);
    render(<App />);

    expect(await screen.findByText("Ship the runtime fix")).toBeTruthy();
  });

  it("stops loading after failure and retries through loadRuns", async () => {
    vi.mocked(fetchRuns)
      .mockRejectedValueOnce(new Error("Factory API did not respond."))
      .mockResolvedValueOnce([]);
    render(<App />);

    expect(await screen.findByText("Could not connect to the Factory API.")).toBeTruthy();
    expect(screen.queryByText("Loading…")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(fetchRuns).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("No runs yet")).toBeTruthy();
  });
});
