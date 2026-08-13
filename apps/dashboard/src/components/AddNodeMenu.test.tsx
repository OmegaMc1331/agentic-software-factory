import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AddNodeMenu } from "./AddNodeMenu";

afterEach(cleanup);

describe("Add Node menu", () => {
  it("puts Workflow first and plans with the configured Planner role", () => {
    const onCreateWorkflow = vi.fn();
    render(
      <AddNodeMenu
        open
        config={{
          agents: { codex: { command: "codex", args: ["exec"], env: {} } },
          roles: { planner: { agent: "codex" } },
        }}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={onCreateWorkflow}
        onCreateAgent={vi.fn()}
        onCreateRole={vi.fn()}
        onCreateVisual={vi.fn()}
      />
    );

    const options = screen.getAllByRole("button");
    expect(options[1].textContent).toContain("Workflow");
    fireEvent.click(screen.getByRole("button", { name: /Workflow/ }));
    expect(screen.getByText("codex")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("What should the Factory build?"), {
      target: { value: "Implement authentication" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Plan" }));
    expect(onCreateWorkflow).toHaveBeenCalledWith("Implement authentication");
  });

  it("explains how to configure a missing Planner instead of faking an override", () => {
    render(
      <AddNodeMenu
        open
        initialKind="workflow"
        config={{ agents: {}, roles: {} }}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={vi.fn()}
        onCreateAgent={vi.fn()}
        onCreateRole={vi.fn()}
        onCreateVisual={vi.fn()}
      />
    );

    expect(screen.getByText("No planner configured.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Plan" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("button", { name: "Configure agents" })).toBeTruthy();
  });

  it("rejects malformed environment lines instead of silently dropping them", () => {
    const onCreateAgent = vi.fn();
    render(
      <AddNodeMenu
        open
        config={{ agents: {}, roles: {} }}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={vi.fn()}
        onCreateAgent={onCreateAgent}
        onCreateRole={vi.fn()}
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
        config={{ agents: {}, roles: {} }}
        error={null}
        onClose={vi.fn()}
        onCreateWorkflow={vi.fn()}
        onCreateAgent={onCreateAgent}
        onCreateRole={vi.fn()}
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
});
