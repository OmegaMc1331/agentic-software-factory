import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { PolicyView } from "../types";
import { PolicySummary } from "./PolicySummary";

function view(overrides: Partial<PolicyView> = {}): PolicyView {
  return {
    source: "role:worker",
    permissive: false,
    filesystemMode: "restricted",
    readScopes: ["**"],
    writeScopes: ["src/**", "tests/**"],
    denyWriteScopes: [".factory/**"],
    commandsMode: "restricted",
    commandsAllow: ["cargo", "npm", "git"],
    commandsDeny: ["powershell", "bash"],
    network: "allow",
    networkEnforcement: "advisory",
    environmentMode: "filtered",
    environmentAllowed: ["PATH", "HOME"],
    environmentDenied: ["GITHUB_TOKEN"],
    gitAllowed: ["read", "commit_in_task_worktree"],
    gitDenied: ["push", "force_push", "delete_branch", "reset_branch", "modify_remotes"],
    ...overrides,
  };
}

afterEach(cleanup);

describe("PolicySummary", () => {
  it("renders the effective write scopes and deny rules", () => {
    render(<PolicySummary permissions={view()} />);
    expect(screen.getByText(/write: src\/\*\*, tests\/\*\*/)).toBeTruthy();
    expect(screen.getByText(/deny: \.factory\/\*\*/)).toBeTruthy();
    expect(screen.getByText("Policy source: role:worker")).toBeTruthy();
  });

  it("marks the network mode as advisory, never a sandbox claim", () => {
    render(<PolicySummary permissions={view()} />);
    expect(screen.getByText("Allowed")).toBeTruthy();
    expect(screen.getByText(/advisory — not process-enforced/)).toBeTruthy();
  });

  it("summarizes git as task-worktree only and lists always-denied operations", () => {
    render(<PolicySummary permissions={view()} />);
    expect(screen.getByText("Task worktree only")).toBeTruthy();
    expect(
      screen.getByText(/Dangerous Git operations \(push, force push, branch deletion/)
    ).toBeTruthy();
  });

  it("shows read-only policies without write scopes", () => {
    render(
      <PolicySummary
        permissions={view({
          filesystemMode: "read_only",
          writeScopes: [],
          denyWriteScopes: [],
        })}
      />
    );
    expect(screen.getByText("Read-only")).toBeTruthy();
    expect(screen.queryByText(/write:/)).toBeNull();
  });

  it("flags permissive legacy policies visibly", () => {
    render(
      <PolicySummary
        permissions={view({
          source: "default",
          permissive: true,
          filesystemMode: "open",
          writeScopes: ["**"],
        })}
      />
    );
    expect(screen.getByText(/No policy configured — permissive defaults apply/)).toBeTruthy();
  });

  it("renders a hint when no policy information is available", () => {
    render(<PolicySummary permissions={undefined} />);
    expect(screen.getByText("No policy information available.")).toBeTruthy();
  });
});
