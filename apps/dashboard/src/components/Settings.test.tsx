import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  addRoleAssignment,
  deleteRole,
  fetchAgents,
  fetchConfig,
  fetchRoles,
  removeRoleAssignment,
  saveConfig,
  setPreferredAssignment,
  updateRole,
} from "../api";
import type { ConfigData, RoleInfo } from "../types";
import { SettingsView } from "./Settings";

vi.mock("../api", () => ({
  addRoleAssignment: vi.fn(),
  deleteRole: vi.fn(),
  fetchAgents: vi.fn(),
  fetchConfig: vi.fn(),
  fetchRoles: vi.fn(),
  removeRoleAssignment: vi.fn(),
  saveConfig: vi.fn(),
  setPreferredAssignment: vi.fn(),
  updateRole: vi.fn(),
}));

const config: ConfigData = {
  agents: {
    codex: { command: "codex", args: ["exec"], env: {} },
    claude: { command: "claude", args: ["-p"], env: {} },
    gemini: { command: "gemini", args: ["-p"], env: {} },
  },
  roles: {},
  role_assignments: [],
};

function role(id: string, name: string, assignments: RoleInfo["assignments"]): RoleInfo {
  return {
    id,
    name,
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "execution",
    assignments,
    available: assignments.length > 0,
  };
}

const workerRole = role("worker", "Worker", [
  { agent: "codex", preferred: true },
  { agent: "gemini", preferred: false },
]);
const reviewerRole = role("reviewer", "Reviewer", [{ agent: "codex", preferred: true }]);
const customRole: RoleInfo = {
  id: "database_engineer",
  name: "Database Engineer",
  kind: "custom",
  description: "Designs schema migrations.",
  instructions: "",
  executionClass: "advisory",
  assignments: [],
  available: false,
};

beforeEach(() => {
  vi.mocked(fetchConfig).mockReset().mockResolvedValue(config);
  vi.mocked(fetchAgents).mockReset().mockResolvedValue([]);
  vi.mocked(fetchRoles).mockReset().mockResolvedValue([workerRole, reviewerRole, customRole]);
  vi.mocked(saveConfig).mockReset().mockResolvedValue();
  vi.mocked(addRoleAssignment).mockReset().mockResolvedValue(workerRole);
  vi.mocked(removeRoleAssignment).mockReset().mockResolvedValue(workerRole);
  vi.mocked(setPreferredAssignment).mockReset().mockResolvedValue(workerRole);
  vi.mocked(updateRole).mockReset().mockResolvedValue(customRole);
  vi.mocked(deleteRole).mockReset().mockResolvedValue();
  vi.spyOn(window, "confirm").mockReturnValue(true);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Settings roles", () => {
  it("renders one card per role with agent chips and preferred markers", async () => {
    render(<SettingsView />);

    expect(await screen.findByText("Worker")).toBeTruthy();
    expect(screen.getByText("Reviewer")).toBeTruthy();
    expect(screen.getByText("Database Engineer")).toBeTruthy();
    expect(screen.getByText(/Custom role/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove codex from Worker" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove codex from Reviewer" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove gemini from Worker" })).toBeTruthy();
    expect(screen.getAllByLabelText("preferred").length).toBe(2);
    expect(screen.queryByRole("button", { name: "Edit Worker role" })).toBeNull();
    expect(screen.getAllByRole("button", { name: "Delete" }).length).toBe(1);
    expect(screen.getByRole("button", { name: "Edit Database Engineer role" })).toBeTruthy();
  });

  it("assigns and removes agents through the role assignment API", async () => {
    render(<SettingsView />);

    fireEvent.change(await screen.findByLabelText("Add agent to Worker"), {
      target: { value: "claude" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Assign agent to Worker" }));

    await waitFor(() => expect(addRoleAssignment).toHaveBeenCalledWith("worker", "claude"));

    fireEvent.click(screen.getByRole("button", { name: "Remove gemini from Worker" }));
    await waitFor(() => expect(removeRoleAssignment).toHaveBeenCalledWith("worker", "gemini"));
  });

  it("marks the preferred assignment from the radio group", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByLabelText("gemini"));

    await waitFor(() => expect(setPreferredAssignment).toHaveBeenCalledWith("worker", "gemini"));
  });

  it("edits a custom role definition", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "Edit Database Engineer role" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Data Engineer" } });
    fireEvent.change(screen.getByLabelText("Description"), {
      target: { value: "Owns data pipelines." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save role" }));

    await waitFor(() =>
      expect(updateRole).toHaveBeenCalledWith("database_engineer", {
        name: "Data Engineer",
        description: "Owns data pipelines.",
        executionClass: "advisory",
        instructions: "",
      })
    );
  });

  it("deletes a custom role and surfaces in-use failures", async () => {
    vi.mocked(deleteRole).mockRejectedValue(
      new Error("Factory API request failed (HTTP 409): used by an active workflow")
    );
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));

    await waitFor(() => expect(deleteRole).toHaveBeenCalledWith("database_engineer"));
    expect(await screen.findByText(/used by an active workflow/)).toBeTruthy();
  });
});
