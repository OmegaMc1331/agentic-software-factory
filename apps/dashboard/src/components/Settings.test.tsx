import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  addRoleAssignment,
  createRecipe,
  deleteRecipe,
  deleteRole,
  fetchAgents,
  fetchConfig,
  fetchRecipes,
  fetchRoles,
  removeRoleAssignment,
  saveConfig,
  setPreferredAssignment,
  updateRecipe,
  updateRole,
} from "../api";
import type { ConfigData, Recipe, RoleInfo } from "../types";
import { SettingsView } from "./Settings";

vi.mock("../api", () => ({
  addRoleAssignment: vi.fn(),
  deleteRole: vi.fn(),
  fetchAgents: vi.fn(),
  fetchConfig: vi.fn(),
  fetchRoles: vi.fn(),
  fetchRecipes: vi.fn(),
  createRecipe: vi.fn(),
  updateRecipe: vi.fn(),
  deleteRecipe: vi.fn(),
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
  vi.mocked(fetchRecipes).mockReset().mockResolvedValue([]);
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

const builtinSecurity: Recipe = {
  id: "security",
  name: "Security Sensitive",
  description: "Security-relevant work.",
  roles: ["planner", "worker", "reviewer"],
  routing: "project_default",
  maxParallelTasks: 2,
  policyPresets: {},
  plannerGuidance: "",
  builtin: true,
};

const customRecipe: Recipe = {
  id: "backend_feature",
  name: "Backend Feature",
  description: "Backend implementation.",
  roles: ["planner", "worker", "reviewer"],
  routing: "performance",
  maxParallelTasks: null,
  policyPresets: {},
  plannerGuidance: "Prefer a compact plan.",
  builtin: false,
};

describe("Settings recipes", () => {
  beforeEach(() => {
    vi.mocked(fetchConfig).mockReset().mockResolvedValue(config);
    vi.mocked(fetchAgents).mockReset().mockResolvedValue([]);
    vi.mocked(fetchRoles)
      .mockReset()
      .mockResolvedValue([
        workerRole,
        reviewerRole,
        role("planner", "Planner", [{ agent: "codex", preferred: true }]),
      ]);
    vi.mocked(fetchRecipes).mockReset().mockResolvedValue([builtinSecurity, customRecipe]);
    vi.mocked(saveConfig).mockReset().mockResolvedValue();
    vi.mocked(createRecipe).mockReset().mockResolvedValue(customRecipe);
    vi.mocked(updateRecipe).mockReset().mockResolvedValue(customRecipe);
    vi.mocked(deleteRecipe).mockReset().mockResolvedValue();
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("lists built-in read-only and custom editable recipes", async () => {
    render(<SettingsView />);

    expect(await screen.findByText("Security Sensitive")).toBeTruthy();
    expect(screen.getByText("Backend Feature")).toBeTruthy();
    expect(screen.getAllByText(/Built-in · read-only/).length).toBe(1);
    expect(screen.getByText(/Custom · Performance-aware/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Duplicate Security Sensitive" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Duplicate Backend Feature" })).toBeTruthy();
    expect(screen.getAllByRole("button", { name: /^Delete$/ }).length).toBe(1);
    expect(screen.getByRole("button", { name: "Edit Backend Feature recipe" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Edit Security Sensitive recipe" })).toBeNull();
  });

  it("creates a custom recipe from the compact editor", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "New recipe" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Hotfix" } });
    fireEvent.change(screen.getByLabelText("Routing"), { target: { value: "round_robin" } });
    fireEvent.change(screen.getByLabelText("Parallelism (1–32)"), {
      target: { value: "2" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create recipe" }));

    await waitFor(() =>
      expect(createRecipe).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Hotfix",
          roles: ["planner"],
          routing: "round_robin",
          maxParallelTasks: 2,
        })
      )
    );
  });

  it("duplicates a built-in recipe into a custom one", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "Duplicate Security Sensitive" }));
    const name = await screen.findByLabelText("Name");
    expect((name as HTMLInputElement).value).toBe("Security Sensitive (copy)");
    fireEvent.click(screen.getByRole("button", { name: "Create recipe" }));

    await waitFor(() =>
      expect(createRecipe).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Security Sensitive (copy)",
          roles: builtinSecurity.roles,
          maxParallelTasks: 2,
        })
      )
    );
  });

  it("edits and deletes a custom recipe", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "Edit Backend Feature recipe" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Renamed" } });
    fireEvent.click(screen.getByRole("button", { name: "Save recipe" }));

    await waitFor(() =>
      expect(updateRecipe).toHaveBeenCalledWith(
        "backend_feature",
        expect.objectContaining({ name: "Renamed" })
      )
    );

    fireEvent.click(screen.getByRole("button", { name: /^Delete$/ }));
    await waitFor(() => expect(deleteRecipe).toHaveBeenCalledWith("backend_feature"));
  });
});
