import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchAgents, fetchConfig, fetchGraph } from "../api";
import { NetworkView } from "./NetworkView";
import { SettingsView } from "./Settings";

vi.mock("../api", () => ({
  fetchAgents: vi.fn(),
  fetchConfig: vi.fn(),
  fetchGraph: vi.fn(),
  saveConfig: vi.fn(),
}));

beforeEach(() => {
  vi.mocked(fetchAgents).mockReset();
  vi.mocked(fetchConfig).mockReset();
  vi.mocked(fetchGraph).mockReset();
});

afterEach(cleanup);

describe("secondary view loading failures", () => {
  it("renders the Agent Graph ready state", async () => {
    vi.mocked(fetchGraph).mockResolvedValue({
      nodes: [],
      edges: [],
      metadata: { runs: 0, tasks: 0, agents: 0, missingAgents: 0, roles: 0 },
    });
    render(<NetworkView />);

    expect(await screen.findByText("No agents configured.")).toBeTruthy();
  });

  it("renders Settings after config and agent availability load", async () => {
    vi.mocked(fetchConfig).mockResolvedValue({
      agents: { codex: { command: "codex", args: ["exec"], env: {} } },
      roles: { planner: { agent: "codex" } },
    });
    vi.mocked(fetchAgents).mockResolvedValue([
      { name: "codex", command: "codex", args: ["exec"], available: true },
    ]);
    render(<SettingsView />);

    expect(await screen.findByRole("heading", { name: "Settings" })).toBeTruthy();
    expect(screen.getByText("available")).toBeTruthy();
  });

  it("stops loading the Agent Graph after an API failure", async () => {
    vi.mocked(fetchGraph).mockRejectedValue(new Error("Factory API did not respond."));
    render(<NetworkView />);

    expect(await screen.findByText("Could not load the Agent Graph.")).toBeTruthy();
    expect(screen.queryByText("Loading…")).toBeNull();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
  });

  it("stops loading Settings when config or agent availability fails", async () => {
    vi.mocked(fetchConfig).mockRejectedValue(new Error("Could not connect to the Factory API."));
    vi.mocked(fetchAgents).mockResolvedValue([]);
    render(<SettingsView />);

    expect(await screen.findByText("Could not load Settings.")).toBeTruthy();
    expect(screen.queryByText("Loading…")).toBeNull();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
  });
});
