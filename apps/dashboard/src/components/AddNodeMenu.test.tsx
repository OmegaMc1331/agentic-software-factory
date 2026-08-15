import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ConfigData, RoleInfo } from "../types";
import { AddNodeMenu } from "./AddNodeMenu";

afterEach(cleanup);

function role(
  id: string,
  agents: string[],
  options: { kind?: "core" | "custom"; preferred?: string } = {}
): RoleInfo {
  return {
    id,
    name: id === "security_auditor" ? "Security Auditor" : id[0].toUpperCase() + id.slice(1),
    kind: options.kind ?? "core",
    description: "Test role.",
    instructions: "",
    executionClass: "execution",
    assignments: agents.map((agent) => ({
      agent,
      preferred: agent === (options.preferred ?? agents[0]),
    })),
    available: agents.length > 0,
  };
}

const config: ConfigData = {
  agents: {
    codex: { command: "codex", args: ["exec"], env: {} },
    claude: { command: "claude", args: ["-p"], env: {} },
    gemini: { command: "gemini", args: ["-p"], env: {} },
  },
  roles: {},
  role_assignments: [],
};

const pipelineRoles: RoleInfo[] = [
  role("planner", ["codex"]),
  role("worker", ["claude"], { preferred: "claude" }),
  role("reviewer", ["codex"]),
];

describe("Add Node menu", () => {
  it("puts Workflow first and plans with the default team from preferred assignments", () => {
    const onCreateWorkflow = vi.fn();
    render(
      <AddNodeMenu
        open
        config={config}
        roles={pipelineRoles}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={onCreateWorkflow}
        onCreateAgent={vi.fn()}
        onCreateRole={vi.fn()}
        onAssignCoreRole={vi.fn()}
        onCreateVisual={vi.fn()}
      />
    );

    const options = screen.getAllByRole("button");
    expect(options[1].textContent).toContain("Workflow");
    fireEvent.click(screen.getByRole("button", { name: /Workflow/ }));
    expect((screen.getByLabelText("Planner") as HTMLSelectElement).value).toBe("codex");
    fireEvent.change(screen.getByLabelText("What should the Factory build?"), {
      target: { value: "Implement authentication" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Plan" }));
    expect(onCreateWorkflow).toHaveBeenCalledWith("Implement authentication", {
      planner: "codex",
      workers: ["claude"],
      reviewers: ["codex"],
      additional: {},
    });
  });

  it("disables planning and explains when the planner role has no agent", () => {
    render(
      <AddNodeMenu
        open
        initialKind="workflow"
        config={{ agents: {}, roles: {}, role_assignments: [] }}
        roles={pipelineRoles.map((entry) => (entry.id === "planner" ? role("planner", []) : entry))}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={vi.fn()}
        onCreateAgent={vi.fn()}
        onCreateRole={vi.fn()}
        onAssignCoreRole={vi.fn()}
        onCreateVisual={vi.fn()}
      />
    );

    expect(
      screen.getByText(/No planner configured\. Assign an agent to the planner role/)
    ).toBeTruthy();
    expect((screen.getByRole("button", { name: "Plan" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("includes optional role agents picked under Advanced team in the workflow team", () => {
    const onCreateWorkflow = vi.fn();
    const roles: RoleInfo[] = [
      ...pipelineRoles,
      role("security_auditor", ["gemini"], { preferred: "gemini" }),
    ];
    render(
      <AddNodeMenu
        open
        initialKind="workflow"
        config={config}
        roles={roles}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={onCreateWorkflow}
        onCreateAgent={vi.fn()}
        onCreateRole={vi.fn()}
        onAssignCoreRole={vi.fn()}
        onCreateVisual={vi.fn()}
      />
    );

    fireEvent.change(screen.getByLabelText("What should the Factory build?"), {
      target: { value: "Harden the API" },
    });
    fireEvent.click(screen.getByText("Advanced team"));
    fireEvent.click(screen.getByLabelText("gemini"));
    fireEvent.click(screen.getByRole("button", { name: "Plan" }));

    expect(onCreateWorkflow).toHaveBeenCalledWith(
      "Harden the API",
      expect.objectContaining({
        planner: "codex",
        additional: { security_auditor: ["gemini"] },
      })
    );
  });

  it("rejects malformed environment lines instead of silently dropping them", () => {
    const onCreateAgent = vi.fn();
    render(
      <AddNodeMenu
        open
        config={{ agents: {}, roles: {}, role_assignments: [] }}
        roles={[]}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={vi.fn()}
        onCreateAgent={onCreateAgent}
        onCreateRole={vi.fn()}
        onAssignCoreRole={vi.fn()}
        onCreateVisual={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Agent/ }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "codex" } });
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "codex" } });
    fireEvent.change(screen.getByLabelText("Environment"), {
      target: { value: "VALID=1\nmissing-separator" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    expect(screen.getByText("Environment line 2 must use KEY=VALUE.")).toBeTruthy();
    expect(onCreateAgent).not.toHaveBeenCalled();
  });

  it("creates a known agent with its workflow preset", () => {
    const onCreateAgent = vi.fn();
    render(
      <AddNodeMenu
        open
        initialKind="agent"
        config={{ agents: {}, roles: {}, role_assignments: [] }}
        roles={[]}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={vi.fn()}
        onCreateAgent={onCreateAgent}
        onCreateRole={vi.fn()}
        onAssignCoreRole={vi.fn()}
        onCreateVisual={vi.fn()}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "coding" } });
    fireEvent.change(screen.getByLabelText("Agent type"), {
      target: { value: "open_code" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    expect(onCreateAgent).toHaveBeenCalledWith("coding", {
      kind: "open_code",
      command: "opencode",
      args: ["run"],
      env: {},
      capabilities: [],
    });
  });

  it("creates a custom role with a derived slug, template prefill and agent selection", () => {
    const onCreateRole = vi.fn();
    render(
      <AddNodeMenu
        open
        initialKind="role"
        config={config}
        roles={pipelineRoles}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={vi.fn()}
        onCreateAgent={vi.fn()}
        onCreateRole={onCreateRole}
        onAssignCoreRole={vi.fn()}
        onCreateVisual={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Custom role/ }));
    fireEvent.change(screen.getByLabelText("Template"), { target: { value: "research" } });
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Database Engineer" } });
    expect(screen.getByText("database_engineer")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Role id"), { target: { value: "dba" } });
    fireEvent.click(screen.getByLabelText("codex"));
    fireEvent.click(screen.getAllByLabelText("Preferred")[0]);
    fireEvent.click(screen.getByLabelText("claude"));
    fireEvent.click(screen.getByRole("button", { name: "Create role" }));

    expect(onCreateRole).toHaveBeenCalledWith({
      id: "dba",
      name: "Database Engineer",
      description: "Gathers technical context another role needs.",
      executionClass: "advisory",
      instructions: expect.stringContaining("Purpose: collect and summarize"),
      agents: ["codex", "claude"],
      preferredAgent: "codex",
    });
  });

  it("assigns an agent to a dormant optional core role", () => {
    const onAssignCoreRole = vi.fn();
    render(
      <AddNodeMenu
        open
        initialKind="role"
        config={config}
        roles={[...pipelineRoles, role("researcher", [])]}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={vi.fn()}
        onCreateAgent={vi.fn()}
        onCreateRole={vi.fn()}
        onAssignCoreRole={onAssignCoreRole}
        onCreateVisual={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Core role/ }));
    fireEvent.click(screen.getByRole("button", { name: /Researcher/ }));
    fireEvent.change(screen.getByLabelText(/Agent for Researcher/), {
      target: { value: "codex" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Assign" }));

    expect(onAssignCoreRole).toHaveBeenCalledWith("researcher", "codex");
  });
});
