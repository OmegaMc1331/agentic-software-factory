import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  addRoleAssignment,
  createRole,
  removeRoleAssignment,
  setPreferredAssignment,
  setRolePolicy,
  updateRole,
} from "../api";
import type { PolicyView, RoleInfo } from "../types";
import { RoleInspector } from "./RoleInspector";

vi.mock("../api", () => ({
  addRoleAssignment: vi.fn(),
  createRole: vi.fn(),
  removeRoleAssignment: vi.fn(),
  setPreferredAssignment: vi.fn(),
  setRolePolicy: vi.fn(),
  updateRole: vi.fn(),
}));

function permissions(overrides: Partial<PolicyView> = {}): PolicyView {
  return {
    source: "role:database_engineer",
    permissive: false,
    filesystemMode: "restricted",
    readScopes: ["**"],
    writeScopes: ["migrations/**"],
    denyWriteScopes: [],
    commandsMode: "restricted",
    commandsAllow: ["git"],
    commandsDeny: ["bash"],
    network: "allow",
    networkEnforcement: "advisory",
    environmentMode: "filtered",
    environmentAllowed: ["PATH"],
    environmentDenied: [],
    gitAllowed: ["read", "commit_in_task_worktree"],
    gitDenied: ["push", "force_push", "delete_branch", "reset_branch", "modify_remotes"],
    ...overrides,
  };
}

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
  vi.mocked(setRolePolicy).mockReset().mockResolvedValue(customRole());
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

  it("shows the effective permissions with write scopes and the advisory network mode", () => {
    render(
      <RoleInspector
        role={customRole({
          permissions: permissions(),
          policyPreset: "custom",
        })}
        agents={agents}
        onClose={vi.fn()}
        onChanged={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(screen.getByText(/write: migrations\/\*\*/)).toBeTruthy();
    expect(screen.getByText("Task worktree only")).toBeTruthy();
    expect(screen.getByText(/advisory — not process-enforced/)).toBeTruthy();
    expect(
      screen.getByText(
        /Instructions say what this role should do; the policy says what Factory permits/
      )
    ).toBeTruthy();
  });

  it("edits the role's policy preset through the policy endpoint", async () => {
    const onChanged = vi.fn();
    render(
      <RoleInspector
        role={customRole({ permissions: permissions(), policyPreset: "custom" })}
        agents={agents}
        onClose={vi.fn()}
        onChanged={onChanged}
        onDelete={vi.fn()}
      />
    );

    fireEvent.change(screen.getByLabelText("Policy preset"), {
      target: { value: "read_only" },
    });

    await waitFor(() =>
      expect(setRolePolicy).toHaveBeenCalledWith("database_engineer", "read_only")
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it("clearing the preset sends null so the role returns to permissive defaults", async () => {
    const onChanged = vi.fn();
    render(
      <RoleInspector
        role={customRole({ permissions: permissions(), policyPreset: "custom" })}
        agents={agents}
        onClose={vi.fn()}
        onChanged={onChanged}
        onDelete={vi.fn()}
      />
    );

    fireEvent.change(screen.getByLabelText("Policy preset"), {
      target: { value: "" },
    });

    await waitFor(() => expect(setRolePolicy).toHaveBeenCalledWith("database_engineer", null));
  });
});
