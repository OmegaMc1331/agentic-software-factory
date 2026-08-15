import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  addRoleAssignment,
  createRole,
  removeRoleAssignment,
  setPreferredAssignment,
  updateRole,
} from "../api";
import type { RoleInfo } from "../types";
import { RoleInspector } from "./RoleInspector";

vi.mock("../api", () => ({
  addRoleAssignment: vi.fn(),
  createRole: vi.fn(),
  removeRoleAssignment: vi.fn(),
  setPreferredAssignment: vi.fn(),
  updateRole: vi.fn(),
}));

function customRole(overrides: Partial<RoleInfo> = {}): RoleInfo {
  return {
    id: "database_engineer",
    name: "Database Engineer",
    kind: "custom",
    description: "Designs schema migrations.",
    instructions: "Purpose: keep the schema consistent.",
    executionClass: "advisory",
    assignments: [
      { agent: "codex", preferred: true },
      { agent: "claude", preferred: false },
    ],
    available: true,
    ...overrides,
  };
}

const coreRole: RoleInfo = {
  id: "worker",
  name: "Worker",
  kind: "core",
  description: "Implements a planned task in an isolated worktree.",
  instructions: "Purpose: implement exactly one task.",
  executionClass: "execution",
  assignments: [{ agent: "opencode", preferred: true }],
  available: true,
};

const agents = ["claude", "codex", "gemini"];

beforeEach(() => {
  vi.mocked(addRoleAssignment).mockReset().mockResolvedValue(customRole());
  vi.mocked(createRole).mockReset().mockResolvedValue(customRole());
  vi.mocked(removeRoleAssignment).mockReset().mockResolvedValue(customRole());
  vi.mocked(setPreferredAssignment).mockReset().mockResolvedValue(customRole());
  vi.mocked(updateRole).mockReset().mockResolvedValue(customRole());
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Role inspector", () => {
  it("shows core role agents without definition edit or delete actions", async () => {
    const onChanged = vi.fn();
    render(
      <RoleInspector
        role={coreRole}
        agents={agents}
        onClose={vi.fn()}
        onChanged={onChanged}
        onDelete={vi.fn()}
      />
    );

    expect(screen.getByText("Core role")).toBeTruthy();
    expect(screen.getByText("Worker")).toBeTruthy();
    expect(screen.getByText("Implements a planned task in an isolated worktree.")).toBeTruthy();
    expect(screen.getByText("opencode")).toBeTruthy();
    expect(screen.getByText("Preferred")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Edit" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
  });

  it("removes an assignment through the API", async () => {
    const onChanged = vi.fn();
    render(
      <RoleInspector
        role={customRole()}
        agents={agents}
        onClose={vi.fn()}
        onChanged={onChanged}
        onDelete={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Remove claude from Database Engineer" }));

    await waitFor(() =>
      expect(removeRoleAssignment).toHaveBeenCalledWith("database_engineer", "claude")
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it("assigns a second agent and marks a preferred one", async () => {
    const onChanged = vi.fn();
    render(
      <RoleInspector
        role={customRole()}
        agents={agents}
        onClose={vi.fn()}
        onChanged={onChanged}
        onDelete={vi.fn()}
      />
    );

    fireEvent.change(screen.getByLabelText("Add agent to role"), { target: { value: "gemini" } });
    fireEvent.click(screen.getByRole("button", { name: "Assign" }));
    await waitFor(() =>
      expect(addRoleAssignment).toHaveBeenCalledWith("database_engineer", "gemini")
    );

    fireEvent.click(screen.getByRole("button", { name: "Set preferred" }));
    await waitFor(() =>
      expect(setPreferredAssignment).toHaveBeenCalledWith("database_engineer", "claude")
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it("edits a custom role definition through the update API", async () => {
    const onChanged = vi.fn();
    render(
      <RoleInspector
        role={customRole()}
        agents={agents}
        onClose={vi.fn()}
        onChanged={onChanged}
        onDelete={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
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
        instructions: "Purpose: keep the schema consistent.",
      })
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it("duplicates a custom role with a copied name and assignments", async () => {
    const onChanged = vi.fn();
    render(
      <RoleInspector
        role={customRole()}
        agents={agents}
        onClose={vi.fn()}
        onChanged={onChanged}
        onDelete={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Duplicate" }));

    await waitFor(() =>
      expect(createRole).toHaveBeenCalledWith({
        name: "Database Engineer (copy)",
        description: "Designs schema migrations.",
        executionClass: "advisory",
        instructions: "Purpose: keep the schema consistent.",
        agents: ["codex", "claude"],
        preferredAgent: "codex",
      })
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it("surfaces role API failures inline", async () => {
    vi.mocked(removeRoleAssignment).mockRejectedValue(
      new Error("Factory API request failed (HTTP 409): already assigned")
    );
    render(
      <RoleInspector
        role={customRole()}
        agents={agents}
        onClose={vi.fn()}
        onChanged={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Remove claude from Database Engineer" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByText(/already assigned/)).toBeTruthy();
  });
});
