import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { SettingsView } from "./Settings";
import type { ConfigData } from "../types";

vi.mock("../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api")>();
  return {
    ...actual,
    fetchConfig: vi.fn(),
    fetchAgents: vi.fn(),
    fetchRoles: vi.fn(),
    fetchRecipes: vi.fn(),
    fetchMcpServers: vi.fn(),
    saveConfig: vi.fn(),
  };
});

import * as api from "../api";

const config: ConfigData = {
  agents: {
    codex: { command: "codex", args: ["exec"], env: {}, capabilities: [] },
    claude: { command: "claude", args: ["-p"], env: {}, capabilities: [] },
  },
  roles: {},
  role_assignments: [{ role: "planner", agent: "codex", preferred: true }],
};

function primeLoads() {
  vi.mocked(api.fetchConfig).mockResolvedValue(config);
  vi.mocked(api.fetchAgents).mockResolvedValue([]);
  vi.mocked(api.fetchRoles).mockResolvedValue([]);
  vi.mocked(api.fetchRecipes).mockResolvedValue([]);
  vi.mocked(api.fetchMcpServers).mockResolvedValue([]);
}

beforeEach(() => {
  primeLoads();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function loaded() {
  render(<SettingsView />);
  await waitFor(() => expect(screen.getAllByRole("button", { name: "Remove" }).length).toBe(2));
}

describe("Settings configuration conflicts", () => {
  it("shows a conflict alert, no success message, and never retries the stale save", async () => {
    await loaded();
    vi.mocked(api.saveConfig).mockRejectedValue(
      new api.FactoryApiError(
        "Factory API request failed (HTTP 412): the configuration was modified",
        "http",
        { status: 412 }
      )
    );

    fireEvent.click(screen.getAllByRole("button", { name: "Remove" })[0]);

    const alert = await screen.findByTestId("config-conflict");
    expect(alert.textContent).toContain("modified elsewhere");
    expect(screen.queryByText(/Removed agent/)).toBeNull();
    expect(vi.mocked(api.saveConfig)).toHaveBeenCalledTimes(1);
  });

  it("reloads the latest configuration from the conflict alert", async () => {
    await loaded();
    vi.mocked(api.saveConfig).mockRejectedValue(
      new api.FactoryApiError("Factory API request failed (HTTP 412)", "http", { status: 412 })
    );

    fireEvent.click(screen.getAllByRole("button", { name: "Remove" })[0]);
    fireEvent.click(await screen.findByText("Reload latest configuration"));

    await waitFor(() => expect(api.fetchConfig).toHaveBeenCalledTimes(2));
    expect(screen.queryByTestId("config-conflict")).toBeNull();
  });

  it("keeps the open agent editor draft when the save hits a conflict", async () => {
    await loaded();
    vi.mocked(api.saveConfig).mockRejectedValue(
      new api.FactoryApiError("Factory API request failed (HTTP 428)", "http", { status: 428 })
    );

    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);
    const nameField = await screen.findByLabelText("Name");
    fireEvent.change(nameField, { target: { value: "kept-draft" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await screen.findByTestId("config-conflict");

    // The unsaved draft survives the conflict for review or copy.
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("kept-draft");
    expect(screen.getByRole("button", { name: "Save changes" })).toBeTruthy();
  });

  it("still reports ordinary save failures as errors", async () => {
    await loaded();
    vi.mocked(api.saveConfig).mockRejectedValue(
      new api.FactoryApiError(
        "Factory API request failed (HTTP 400): unknown agent 'ghost'",
        "http",
        { status: 400 }
      )
    );

    fireEvent.click(screen.getAllByRole("button", { name: "Remove" })[0]);

    expect(await screen.findByText(/unknown agent 'ghost'/)).toBeTruthy();
    expect(screen.queryByTestId("config-conflict")).toBeNull();
  });
});
