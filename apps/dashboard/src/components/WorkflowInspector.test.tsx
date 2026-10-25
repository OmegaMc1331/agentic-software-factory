import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createPullRequest,
  fetchDelivery,
  fetchPrPreview,
  reconcileDelivery,
  fetchRoles,
  fetchRun,
  isUncertainApiError,
  saveRunAsRecipe,
  updateWorkflowTeam,
} from "../api";
import type {
  DeliveryReport,
  GitHubDeliveryRecord,
  GraphNode,
  RoleInfo,
  RunDetail,
  RunMeta,
} from "../types";
import { WorkflowInspector } from "./WorkflowInspector";

vi.mock("../api", () => ({
  fetchRoles: vi.fn(),
  saveRunAsRecipe: vi.fn(),
  fetchRun: vi.fn(),
  updateWorkflowTeam: vi.fn(),
  fetchDelivery: vi.fn(),
  fetchPrPreview: vi.fn(),
  createPullRequest: vi.fn(),
  reconcileDelivery: vi.fn(),
  isUncertainApiError: vi.fn(),
}));

const team = {
  planner: "codex",
  workers: ["opencode", "qwen"],
  reviewers: ["claude"],
  additional: { security_auditor: ["claude"] },
};

const meta: RunMeta = {
  runId: 12,
  objective: "Implement email authentication",
  status: "planned",
  plannerAgent: "codex",
  team,
  createdAt: "2026-08-13T18:00:00Z",
  counts: { pending: 0, ready: 1, running: 0, blocked: 0, failed: 0, completed: 0, total: 1 },
};

const node: GraphNode = {
  id: "run:12",
  kind: "run",
  label: "Authentication workflow",
  meta,
};

const detail: RunDetail = {
  run: {
    id: 12,
    objective: "Implement email authentication",
    status: "planned",
    plannerAgent: "codex",
    team,
    createdAt: "2026-08-13T18:00:00Z",
    updatedAt: "2026-08-13T18:01:00Z",
  },
  tasks: [
    {
      id: 41,
      runId: 12,
      title: "Login API",
      objective: "Add login endpoint",
      acceptanceCriteria: ["Invalid credentials are rejected"],
      state: "ready",
      position: 0,
      dependencies: [],
      worktreePath: null,
      role: null,
      operation: "implement",
      createdAt: "2026-08-13T18:01:00Z",
      updatedAt: "2026-08-13T18:01:00Z",
    },
  ],
  attempts: [],
  sessions: [],
  stages: [
    {
      key: "implementation",
      label: "Implementation",
      total: 1,
      completed: 0,
      state: "active",
    },
  ],
  artifacts: [],
  integration: {
    branch: "factory/run-12",
    head: null,
    integratedTasks: [],
  },
};

const roles: RoleInfo[] = [
  {
    id: "planner",
    name: "Planner",
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "planning",
    assignments: [
      { agent: "codex", preferred: true },
      { agent: "gemini", preferred: false },
    ],
    available: true,
  },
  {
    id: "worker",
    name: "Worker",
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "execution",
    assignments: [
      { agent: "opencode", preferred: true },
      { agent: "qwen", preferred: false },
    ],
    available: true,
  },
  {
    id: "reviewer",
    name: "Reviewer",
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "review",
    assignments: [{ agent: "claude", preferred: true }],
    available: true,
  },
];

const deliveryNotReady: DeliveryReport = {
  runId: 12,
  state: "not_ready",
  persistedState: "not_ready",
  link: null,
  repository: {
    repository: "OmegaMc1331/example",
    remote: "origin",
    url: "https://github.com/OmegaMc1331/example",
    defaultBranch: "main",
  },
  baseBranch: "main",
  headBranch: "factory/run-12",
  integrationHead: null,
  localHead: null,
  pushedHead: null,
  pullRequest: null,
  error: null,
  eligible: false,
  blockers: ["the workflow is planned (delivery requires completed)"],
  operationId: null,
  operationActive: false,
  recoveryRequired: false,
  retrySafe: false,
  updatedAt: "2026-08-20T10:00:00Z",
};

const deliveryReady: DeliveryReport = {
  ...deliveryNotReady,
  state: "ready",
  eligible: true,
  blockers: [],
  link: {
    provider: "github",
    repository: "OmegaMc1331/example",
    issueNumber: 42,
    issueUrl: "https://github.com/OmegaMc1331/example/issues/42",
    issueTitle: "Fix refresh token race",
    issueBody: "Tokens rotate concurrently.",
    issueState: "open",
    issueAuthor: "octocat",
    issueLabels: ["bug"],
    issueComments: [],
    importedAt: "2026-08-20T10:00:00Z",
  },
};

const readyPreview = {
  runId: 12,
  repository: "OmegaMc1331/example",
  base: "main",
  head: "factory/run-12",
  title: "Fix refresh token race",
  body: "## Summary\n\nResolve GitHub Issue #42: Fix refresh token race\n\nCloses #42",
  draft: false,
  issueNumber: 42,
  issueUrl: "https://github.com/OmegaMc1331/example/issues/42",
  existing: null,
  eligible: true,
  blockers: [],
};

const publishedDelivery: DeliveryReport = {
  ...deliveryReady,
  state: "published",
  persistedState: "published",
  pullRequest: {
    number: 58,
    url: "https://github.com/OmegaMc1331/example/pull/58",
    state: "OPEN",
    isDraft: false,
  },
};

const publishedRecord: GitHubDeliveryRecord = {
  runId: 12,
  state: "published",
  repository: "OmegaMc1331/example",
  remote: "origin",
  baseBranch: "main",
  headBranch: "factory/run-12",
  pushedHead: "abc123",
  pullRequest: {
    number: 58,
    url: "https://github.com/OmegaMc1331/example/pull/58",
    state: "OPEN",
    isDraft: false,
  },
  error: null,
  operationId: "delivery-12-1-1",
  recoveryRequired: false,
  createdAt: "2026-08-21T10:00:00Z",
  updatedAt: "2026-08-21T10:00:00Z",
};

const activeDelivery: DeliveryReport = {
  ...deliveryReady,
  state: "pushing",
  persistedState: "pushing",
  operationId: "delivery-12-2-1",
  operationActive: true,
};

const interruptedDelivery: DeliveryReport = {
  ...deliveryReady,
  state: "interrupted",
  persistedState: "interrupted",
  error: "Factory stopped before the delivery outcome was confirmed",
  operationId: "delivery-12-3-1",
  recoveryRequired: true,
};

beforeEach(() => {
  vi.mocked(fetchRun).mockReset().mockResolvedValue(detail);
  vi.mocked(saveRunAsRecipe)
    .mockReset()
    .mockResolvedValue({
      id: "saved_recipe",
      name: "Saved Recipe",
      description: "",
      roles: ["planner", "worker", "reviewer"],
      routing: "project_default",
      maxParallelTasks: null,
      policyPresets: {},
      plannerGuidance: "",
      builtin: false,
    });
  vi.mocked(fetchRoles).mockReset().mockResolvedValue(roles);
  vi.mocked(updateWorkflowTeam).mockReset().mockResolvedValue(team);
  vi.mocked(fetchDelivery).mockReset().mockResolvedValue(deliveryNotReady);
  vi.mocked(fetchPrPreview).mockReset();
  vi.mocked(createPullRequest).mockReset();
  vi.mocked(reconcileDelivery).mockReset();
  vi.mocked(isUncertainApiError).mockReset().mockReturnValue(false);
  vi.spyOn(window, "confirm").mockReturnValue(true);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Workflow inspector", () => {
  it("shows derived stages and an artifact inspector from the run detail", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      stages: [
        { key: "analysis", label: "Analysis", total: 1, completed: 0, state: "active" },
        {
          key: "implementation",
          label: "Implementation",
          total: 1,
          completed: 1,
          state: "completed",
        },
      ],
      artifacts: [
        {
          id: 3,
          runId: 12,
          taskId: 40,
          attemptId: 9,
          role: "researcher",
          operation: "advisory",
          kind: "research",
          content: '{"summary":"tokens in httpOnly cookies","findings":[]}',
          createdAt: "2026-08-13T18:00:00Z",
        },
      ],
    });
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText("Analysis")).toBeTruthy();
    expect(screen.getByText("Implementation")).toBeTruthy();
    expect(screen.getByText("✓")).toBeTruthy();
    expect(screen.getByText("1/1")).toBeTruthy();

    fireEvent.click(screen.getByText("Role artifacts (1)"));
    expect(screen.getByText(/Research findings/)).toBeTruthy();
    fireEvent.click(screen.getByText("Inspect content"));
    expect(screen.getByText(/tokens in httpOnly cookies/)).toBeTruthy();
  });

  it("stays simple: workflows without stages show no empty stage list", async () => {
    vi.mocked(fetchRun).mockResolvedValue({ ...detail, stages: [], artifacts: [] });
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );
    await screen.findByText("0 / 1 tasks");
    expect(screen.queryByText("Stages")).toBeNull();
  });

  it("labels every task with its role operation in the tasks tab", async () => {
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );
    fireEvent.click(await screen.findByRole("tab", { name: "tasks" }));
    expect(screen.getByText("implement")).toBeTruthy();
    expect(screen.getByText("worker")).toBeTruthy();
  });

  it("shows the persisted plan with its team and starts it after confirmation", async () => {
    const onStart = vi.fn().mockResolvedValue(undefined);
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={onStart}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText("0 / 1 tasks")).toBeTruthy();
    expect(screen.getByText("codex")).toBeTruthy();
    expect(screen.getByText("opencode, qwen")).toBeTruthy();
    expect(screen.getByText("security_auditor")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "tasks" }));
    expect(screen.getByText("#41 Login API")).toBeTruthy();
    expect(screen.getByText("Invalid credentials are rejected")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "overview" }));
    fireEvent.click(screen.getByRole("button", { name: "Start" }));

    await waitFor(() => expect(onStart).toHaveBeenCalledWith(12));
    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining("Workers: opencode, qwen"));
    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining("isolated worktrees"));
  });

  it("edits the team while the workflow is still planned", async () => {
    const onTeamUpdated = vi.fn();
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
        onTeamUpdated={onTeamUpdated}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "Edit team" }));
    await waitFor(() => expect(fetchRoles).toHaveBeenCalled());
    fireEvent.click(screen.getByLabelText("qwen"));
    fireEvent.click(screen.getByRole("button", { name: "Save team" }));

    await waitFor(() =>
      expect(updateWorkflowTeam).toHaveBeenCalledWith(12, {
        planner: "codex",
        workers: ["opencode"],
        reviewers: ["claude"],
        additional: { security_auditor: ["claude"] },
      })
    );
    await waitFor(() => expect(onTeamUpdated).toHaveBeenCalled());
  });

  it("locks the team once the workflow is active", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      run: { ...detail.run, status: "active" },
    });
    const activeNode: GraphNode = {
      ...node,
      meta: { ...meta, status: "active" },
    };
    render(
      <WorkflowInspector
        node={activeNode}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText("0 / 1 tasks")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Edit team" })).toBeNull();
    expect(screen.getByText("The team is locked while the workflow is active.")).toBeTruthy();
  });

  it("shows operation errors inline", async () => {
    const onStart = vi.fn().mockRejectedValue(new Error("No Worker agent configured."));
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={onStart}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    await screen.findByText("0 / 1 tasks");
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    expect((await screen.findByRole("alert")).textContent).toContain("No Worker agent configured.");
  });

  it("shows the GitHub issue source and blocks delivery until the workflow completes", async () => {
    vi.mocked(fetchDelivery).mockResolvedValue({
      ...deliveryReady,
      eligible: false,
      state: "not_ready",
      blockers: ["the workflow is planned (delivery requires completed)"],
    });
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText(/GitHub Issue #42 — Fix refresh token race/)).toBeTruthy();
    expect(screen.getByText("Open on GitHub").getAttribute("href")).toBe(
      "https://github.com/OmegaMc1331/example/issues/42"
    );
    expect(screen.queryByRole("button", { name: "Create Pull Request" })).toBeNull();
  });

  it("previews and creates a pull request from real delivery evidence", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      run: { ...detail.run, status: "completed" },
    });
    vi.mocked(fetchDelivery).mockResolvedValue(deliveryReady);
    vi.mocked(fetchPrPreview).mockResolvedValue({
      runId: 12,
      repository: "OmegaMc1331/example",
      base: "main",
      head: "factory/run-12",
      title: "Fix refresh token race",
      body: "## Summary\n\nResolve GitHub Issue #42: Fix refresh token race\n\nCloses #42",
      draft: false,
      issueNumber: 42,
      issueUrl: "https://github.com/OmegaMc1331/example/issues/42",
      existing: null,
      eligible: true,
      blockers: [],
    });
    vi.mocked(createPullRequest).mockResolvedValue(publishedRecord);
    const completedMeta: RunMeta = { ...meta, status: "completed" };
    const completedNode: GraphNode = { ...node, meta: completedMeta };
    const onTeamUpdated = vi.fn();
    render(
      <WorkflowInspector
        node={completedNode}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
        onTeamUpdated={onTeamUpdated}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "Create Pull Request" }));
    const titleInput = await screen.findByLabelText("Title");
    expect((titleInput as HTMLInputElement).value).toBe("Fix refresh token race");
    expect((screen.getByLabelText("Body") as HTMLTextAreaElement).value).toContain("Closes #42");
    expect(screen.getAllByText("OmegaMc1331/example").length).toBeGreaterThan(0);
    expect(screen.getAllByText("factory/run-12").length).toBeGreaterThan(0);

    fireEvent.change(titleInput, { target: { value: "Fix refresh token race (edited)" } });
    fireEvent.click(screen.getByLabelText("Create as draft"));
    fireEvent.click(screen.getByRole("button", { name: "Create Pull Request" }));

    await waitFor(() =>
      expect(createPullRequest).toHaveBeenCalledWith(12, {
        title: "Fix refresh token race (edited)",
        body: expect.stringContaining("Closes #42"),
        draft: true,
      })
    );
    await waitFor(() => expect(onTeamUpdated).toHaveBeenCalled());
  });

  it("offers the existing pull request instead of creating a duplicate", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      run: { ...detail.run, status: "completed" },
    });
    vi.mocked(fetchDelivery).mockResolvedValue({
      ...deliveryReady,
      state: "published",
      eligible: true,
      pullRequest: {
        number: 58,
        url: "https://github.com/OmegaMc1331/example/pull/58",
        state: "OPEN",
        isDraft: false,
      },
    });
    const completedNode: GraphNode = { ...node, meta: { ...meta, status: "completed" } };
    render(
      <WorkflowInspector
        node={completedNode}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText("#58 OPEN")).toBeTruthy();
    const githubLinks = screen
      .getAllByText("Open on GitHub")
      .map((link) => (link as HTMLAnchorElement).getAttribute("href"));
    expect(githubLinks).toContain("https://github.com/OmegaMc1331/example/pull/58");
    expect(screen.queryByRole("button", { name: "Create Pull Request" })).toBeNull();
    expect(fetchPrPreview).not.toHaveBeenCalled();
    expect(createPullRequest).not.toHaveBeenCalled();
  });

  it("reconciles an uncertain pull request before allowing another submission", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      run: { ...detail.run, status: "completed" },
    });
    vi.mocked(fetchDelivery).mockResolvedValue(deliveryReady);
    vi.mocked(fetchPrPreview).mockResolvedValue(readyPreview);
    const timeout = new Error(
      "The request timed out. The operation may still be running or may have completed. Check its status before retrying."
    );
    vi.mocked(createPullRequest).mockRejectedValue(timeout);
    vi.mocked(isUncertainApiError).mockReturnValue(true);
    const onTeamUpdated = vi.fn();
    render(
      <WorkflowInspector
        node={{ ...node, meta: { ...meta, status: "completed" } }}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
        onTeamUpdated={onTeamUpdated}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "Create Pull Request" }));
    await screen.findByLabelText("Title");
    vi.mocked(fetchDelivery).mockResolvedValueOnce(publishedDelivery);
    fireEvent.click(screen.getByRole("button", { name: "Create Pull Request" }));

    await waitFor(() => expect(onTeamUpdated).toHaveBeenCalled());
    expect(createPullRequest).toHaveBeenCalledTimes(1);
    expect(fetchDelivery).toHaveBeenCalledTimes(2);
    expect(screen.queryByText(timeout.message)).toBeNull();
    expect(screen.queryByRole("button", { name: "Retry delivery" })).toBeNull();
  });

  it("does not offer retry while an uncertain delivery is still in progress", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      run: { ...detail.run, status: "completed" },
    });
    vi.mocked(fetchDelivery).mockResolvedValue(deliveryReady);
    vi.mocked(fetchPrPreview).mockResolvedValue(readyPreview);
    vi.mocked(createPullRequest).mockRejectedValue(new Error("Request timed out."));
    vi.mocked(isUncertainApiError).mockReturnValue(true);
    render(
      <WorkflowInspector
        node={{ ...node, meta: { ...meta, status: "completed" } }}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "Create Pull Request" }));
    await screen.findByLabelText("Title");
    vi.mocked(fetchDelivery).mockResolvedValueOnce({
      ...deliveryReady,
      state: "pushing",
      persistedState: "pushing",
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Pull Request" }));

    expect(await screen.findByText(/Delivery is still in progress/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry delivery" })).toBeNull();
    expect(screen.getByRole("button", { name: "Check delivery status" })).toBeTruthy();
    expect(createPullRequest).toHaveBeenCalledTimes(1);
  });

  it("blocks retry when delivery status cannot be confirmed", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      run: { ...detail.run, status: "completed" },
    });
    vi.mocked(fetchDelivery).mockResolvedValue(deliveryReady);
    vi.mocked(fetchPrPreview).mockResolvedValue(readyPreview);
    vi.mocked(createPullRequest).mockRejectedValue(new Error("Request timed out."));
    vi.mocked(isUncertainApiError).mockReturnValue(true);
    render(
      <WorkflowInspector
        node={{ ...node, meta: { ...meta, status: "completed" } }}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "Create Pull Request" }));
    await screen.findByLabelText("Title");
    vi.mocked(fetchDelivery).mockRejectedValueOnce(new Error("Factory API is unavailable."));
    fireEvent.click(screen.getByRole("button", { name: "Create Pull Request" }));

    expect(await screen.findByText(/Delivery status could not be confirmed/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry delivery" })).toBeNull();
    expect(screen.getByRole("button", { name: "Check delivery status" })).toBeTruthy();
    expect(createPullRequest).toHaveBeenCalledTimes(1);
  });

  it("prevents repeated clicks from creating duplicate pull requests", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      run: { ...detail.run, status: "completed" },
    });
    vi.mocked(fetchDelivery).mockResolvedValue(deliveryReady);
    vi.mocked(fetchPrPreview).mockResolvedValue(readyPreview);
    let resolveCreate!: (record: GitHubDeliveryRecord) => void;
    vi.mocked(createPullRequest).mockReturnValue(
      new Promise<GitHubDeliveryRecord>((resolve) => {
        resolveCreate = resolve;
      })
    );
    render(
      <WorkflowInspector
        node={{ ...node, meta: { ...meta, status: "completed" } }}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "Create Pull Request" }));
    await screen.findByLabelText("Title");
    const createButton = screen.getByRole("button", { name: "Create Pull Request" });
    fireEvent.click(createButton);
    fireEvent.click(createButton);

    expect(await screen.findByRole("button", { name: "Creating pull request…" })).toBeTruthy();
    expect(createPullRequest).toHaveBeenCalledTimes(1);

    resolveCreate(publishedRecord);
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Creating pull request…" })).toBeNull()
    );
  });

  it("shows No Recipe for runs created before recipes existed", async () => {
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText("No Recipe")).toBeTruthy();
  });

  it("displays the immutable recipe snapshot with its settings", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      run: {
        ...detail.run,
        recipe: {
          id: "security",
          name: "Security Sensitive",
          roles: ["planner", "worker", "security_auditor", "reviewer"],
          routing: "round_robin",
          maxParallelTasks: 2,
          policyPresets: {},
          plannerGuidance: "",
        },
      },
    });
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText("Security Sensitive")).toBeTruthy();
    expect(screen.getByText(/Routing: Round-robin/)).toBeTruthy();
    expect(screen.getByText(/parallelism 2/)).toBeTruthy();
    expect(screen.getByText(/4 selected roles/)).toBeTruthy();
  });

  it("saves a planned workflow's configuration as a recipe", async () => {
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "Save as Recipe" }));
    fireEvent.change(screen.getByLabelText("Recipe name"), {
      target: { value: "Auth Workflow Recipe" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save recipe" }));

    await waitFor(() =>
      expect(saveRunAsRecipe).toHaveBeenCalledWith(12, {
        name: "Auth Workflow Recipe",
        description: "",
      })
    );
    expect(await screen.findByText(/Saved "Saved Recipe" as a custom recipe/)).toBeTruthy();
    // The save captured configuration only — no DAG or history involved.
    expect(saveRunAsRecipe).toHaveBeenCalledTimes(1);
  });

  it("shows an active Factory-owned operation and blocks duplicate submission", async () => {
    vi.mocked(fetchDelivery).mockReset().mockResolvedValue(activeDelivery);
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );
    await screen.findByText(/Publishing pull request… the operation is owned by Factory/);
    expect(screen.queryByText("Create Pull Request")).toBeNull();
    expect(fetchDelivery).toHaveBeenCalledWith(12);
    expect(createPullRequest).not.toHaveBeenCalled();
  });

  it("recovers an interrupted delivery through explicit reconciliation", async () => {
    vi.mocked(fetchDelivery)
      .mockReset()
      .mockResolvedValueOnce(interruptedDelivery)
      .mockResolvedValueOnce(interruptedDelivery)
      .mockResolvedValue(publishedDelivery);
    vi.mocked(reconcileDelivery).mockReset().mockResolvedValue({ linkedExisting: publishedRecord });
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );
    const reconcile = await screen.findByText("Check GitHub for the previous attempt");
    expect(screen.queryByText("Create Pull Request")).toBeNull();
    fireEvent.click(reconcile);
    expect(
      await screen.findByText("Reconciliation linked the existing pull request #58.")
    ).toBeTruthy();
    expect(await screen.findByRole("link", { name: "Open on GitHub" })).toBeTruthy();
    expect(reconcileDelivery).toHaveBeenCalledWith(12);
  });

  it("keeps retry blocked when reconciliation cannot reach GitHub", async () => {
    vi.mocked(fetchDelivery).mockReset().mockResolvedValue(interruptedDelivery);
    vi.mocked(reconcileDelivery)
      .mockReset()
      .mockResolvedValue({
        unconfirmed: { reason: "network unreachable", pushConfirmed: null },
      });
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );
    fireEvent.click(await screen.findByText("Check GitHub for the previous attempt"));
    await screen.findByText(
      /GitHub could not be reached, so the outcome stays unconfirmed: network unreachable/
    );
    expect(screen.queryByText("Create Pull Request")).toBeNull();
  });

  it("polls a running delivery to publication and then stops", async () => {
    vi.useFakeTimers();
    try {
      vi.mocked(fetchDelivery)
        .mockReset()
        .mockResolvedValueOnce(activeDelivery)
        .mockResolvedValueOnce(activeDelivery)
        .mockResolvedValue(publishedDelivery);
      render(
        <WorkflowInspector
          node={node}
          onClose={vi.fn()}
          onStart={vi.fn()}
          onCancel={vi.fn()}
          onRetry={vi.fn()}
        />
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      const pollsBefore = vi.mocked(fetchDelivery).mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      const pollsAfter = vi.mocked(fetchDelivery).mock.calls.length;
      expect(pollsAfter).toBeGreaterThan(pollsBefore);
      expect(screen.getAllByRole("link", { name: "Open on GitHub" }).length).toBeGreaterThan(0);
      const settled = vi.mocked(fetchDelivery).mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(6000);
      });
      expect(vi.mocked(fetchDelivery).mock.calls.length).toBe(settled);
    } finally {
      vi.useRealTimers();
    }
  });

  it("unmounting stops delivery polling", async () => {
    vi.useFakeTimers();
    try {
      vi.mocked(fetchDelivery).mockReset().mockResolvedValue(activeDelivery);
      const view = render(
        <WorkflowInspector
          node={node}
          onClose={vi.fn()}
          onStart={vi.fn()}
          onCancel={vi.fn()}
          onRetry={vi.fn()}
        />
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      view.unmount();
      const afterUnmount = vi.mocked(fetchDelivery).mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(6000);
      });
      expect(vi.mocked(fetchDelivery).mock.calls.length).toBe(afterUnmount);
    } finally {
      vi.useRealTimers();
    }
  });
});
