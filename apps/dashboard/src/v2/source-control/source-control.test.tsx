import { afterEach, describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createSession, createView } from "../workspace/production/model";
import type { DockSession, WorkspaceModel } from "../workspace/production/controller";
import { SourceControlPanel, SourceDiff } from "./SourceControlPanel";
import { GitHubPanel } from "./GitHubPanel";
import { ReviewPanel } from "./ReviewPanel";
import { useSourceData } from "./client";
import type { GitDiff, GitStatus, Worktree } from "./types";

const mocks = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("../../api", () => ({ request: mocks.request }));
vi.mock("../files/ComparisonView", () => ({
  ComparisonView: ({ before, after }: { before: string; after: string }) => (
    <div aria-label="CodeMirror comparison">
      {before} → {after}
    </div>
  ),
}));
const A = `p_${"a".repeat(32)}`,
  B = `p_${"b".repeat(32)}`;
const status: GitStatus = {
  head: "a".repeat(40),
  branch: "main",
  upstream: null,
  ahead: null,
  behind: null,
  indexRevision: "index",
  generation: "observed",
  operation: null,
  observation: "not atomic",
  changes: [
    {
      path: "source.py",
      originalPath: null,
      index: ".",
      working: "M",
      conflict: false,
      supported: true,
    },
  ],
};
const root: Worktree = {
  root: { id: "r_primary", label: "Repository", kind: "repository", read_only: false },
  status,
  gitCommonIdentity: "common",
};
const diff: GitDiff = {
  path: "source.py",
  comparison: "working_index",
  base: "index:old",
  target: "working_tree",
  generation: "observed",
  version: "actual-diff-version",
  before: "old\n",
  after: "new\n",
  patch: "recorded patch",
  hunks: [{ id: "actual-hunk", header: "@@ -1 +1 @@", patch: "server-selected patch" }],
  limited: false,
  reason: null,
};
afterEach(() => {
  cleanup();
  mocks.request.mockReset();
});

function provide(worktree = root) {
  mocks.request.mockImplementation(async (path: string) => {
    if (path.endsWith("/worktrees")) return [worktree];
    if (path.endsWith("/roots")) return [worktree.root];
    if (path.endsWith("/repository") || path.endsWith(`/repository?rootId=${worktree.root.id}`))
      return worktree;
    if (path.includes("/repository?")) throw new Error("Unknown root");
    if (path.includes("/branches?"))
      return [
        { name: "main", current: true, protected: false, worktree: "/owned" },
        { name: "feature", current: false, protected: false, worktree: "" },
      ];
    if (path.includes("/history?")) return [];
    if (path.includes("/reviews?")) return { items: [] };
    if (path.endsWith("/diff")) return diff;
    return worktree.status;
  });
}
function shell(dirty = false) {
  const view = createView(createSession(A), "sourceControl");
  const model = {
    updateView: vi.fn(),
    documents: {
      all: () => (dirty ? [{ dirty: true, file: { project_id: A } }] : []),
      refresh: vi.fn(),
    },
  } as unknown as WorkspaceModel;
  const dock = {
    sessionId: view.sessionId,
    openFile: vi.fn(),
    open: vi.fn(),
  } as unknown as DockSession;
  return { view, model, dock };
}

describe("Source Control boundaries", () => {
  it("distinguishes a recovered historical conflict from an active recovery requirement", async () => {
    let state = "completed";
    const { view, model, dock } = shell();
    view.state.cursor = 7;
    mocks.request.mockImplementation(async (path: string) => {
      if (path.endsWith("/worktrees")) return [root];
      if (path.includes("/reviews?")) return { items: [], nextCursor: null };
      if (path.endsWith("/reviews/7"))
        return {
          run: { id: 1, objective: "Controlled integration", status: state },
          task: { id: 1, title: "Repair", state: "completed" },
          attempt: {
            id: 7,
            status: "approved",
            evidence: null,
            review: { decision: "approve", reason: "Controlled fixture" },
          },
          actualPaths: [],
          sessions: [],
          humanComments: [],
          handoffs: [],
          authority: "Controlled recorded evidence",
          patchHashVerified: false,
          integration: [
            {
              id: 1,
              task_id: 1,
              status: "conflict",
              before_sha: "a",
              source_sha: "b",
              candidate_sha: null,
              details: {},
            },
            {
              id: 2,
              task_id: 2,
              status: "published",
              before_sha: "a",
              source_sha: "c",
              candidate_sha: "c",
              details: {},
            },
          ],
        };
      throw Error(path);
    });
    render(<ReviewPanel view={view} model={model} dock={dock} />);
    expect(await screen.findByText("Historical conflict retained")).toBeInTheDocument();
    expect(screen.queryByText("Coordinator recovery required")).not.toBeInTheDocument();
    state = "active";
    fireEvent.click(screen.getByRole("button", { name: "Refresh reviews" }));
    expect(await screen.findByText("Coordinator recovery required")).toBeInTheDocument();
  });
  it("fences another index action until the post-mutation status is observed", async () => {
    let reads = 0,
      finish: ((value: Worktree) => void) | undefined;
    mocks.request.mockImplementation(async (path: string) => {
      if (path.endsWith("/roots")) return [root.root];
      if (path.endsWith("/repository")) {
        if (++reads === 1) return root;
        return new Promise((resolve) => {
          finish = resolve;
        });
      }
      return status;
    });
    render(<SourceControlPanel {...shell()} />);
    const stage = await screen.findByRole("button", { name: "Stage source.py" });
    await waitFor(() => expect(stage).toBeEnabled());
    fireEvent.click(stage);
    await waitFor(() => expect(stage).toBeDisabled());
    await waitFor(() => expect(finish).toBeDefined());
    await act(async () => finish?.({ ...root, status: { ...status, generation: "fresh" } }));
    await waitFor(() => expect(stage).toBeEnabled());
  });
  it("does not substitute a writable primary root when a selected managed root disappears", async () => {
    provide();
    const props = shell();
    props.view.state.rootId = "r_removed";
    render(<SourceControlPanel {...props} />);
    expect(await screen.findByText("Selected worktree unavailable")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Commit staged changes" })).not.toBeInTheDocument();
    expect(await screen.findByLabelText("Repository / worktree")).toHaveValue("");
  });
  it("renders authorized status while the worktree catalog is still pending", async () => {
    mocks.request.mockImplementation((path: string) => {
      if (path.endsWith("/repository")) return Promise.resolve(root);
      if (path.endsWith("/roots")) return new Promise(() => {});
      throw new Error(`Unexpected request ${path}`);
    });
    render(<SourceControlPanel {...shell()} />);
    expect(await screen.findByText("main")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stage source.py" })).toBeEnabled();
    expect(screen.getByLabelText("Repository / worktree")).toHaveValue(root.root.id);
    expect(mocks.request.mock.calls.map(([path]) => path)).toEqual([
      `/projects/${A}/source-control/repository`,
      `/projects/${A}/source-control/roots`,
    ]);
  });
  it("retains the scoped observed snapshot when a refresh is temporarily busy", async () => {
    mocks.request
      .mockResolvedValueOnce({ label: "Observed Git state" })
      .mockRejectedValueOnce(new Error("git_busy"));
    function View({ version }: { version: number }) {
      const data = useSourceData<{ label: string }>(A, "status", version);
      return (
        <output>
          {data.value?.label} {data.error}
        </output>
      );
    }
    const ui = render(<View version={0} />);
    await screen.findByText("Observed Git state");
    ui.rerender(<View version={1} />);
    expect(await screen.findByText(/Observed Git state.*git_busy/)).toBeInTheDocument();
  });
  it("keeps exact server diff and hunk identities in a staging action", async () => {
    provide();
    render(<SourceControlPanel {...shell()} />);
    fireEvent.click(await screen.findByRole("button", { name: /Msource.py|M source.py/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Stage hunk 1" }));
    await waitFor(() =>
      expect(mocks.request.mock.calls.some(([path]) => path.endsWith("/index"))).toBe(true)
    );
    const [path, init] = mocks.request.mock.calls.find(([path]) => path.endsWith("/index"))!;
    expect(path).toBe(`/projects/${A}/source-control/index`);
    expect(JSON.parse(init.body).request).toMatchObject({
      expectedGeneration: "observed",
      hunk: "actual-hunk",
      diffVersion: "actual-diff-version",
      paths: ["source.py"],
      unstage: false,
    });
  });
  it("disables stale hunk staging and displays its observed revisions", async () => {
    provide();
    render(
      <SourceDiff
        projectId={A}
        rootId="r_primary"
        path="source.py"
        comparison="working_index"
        generation="newer"
        readOnly={false}
        stageHunk={vi.fn()}
      />
    );
    expect(await screen.findByText(/This diff is outdated/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stage hunk 1" })).toBeDisabled();
    expect(screen.getByText("Base: index:old")).toBeInTheDocument();
  });
  it("never enables manual commits or staging for managed worktrees", async () => {
    provide({ ...root, root: { ...root.root, read_only: true, label: "Agent worktree" } });
    render(<SourceControlPanel {...shell()} />);
    expect(await screen.findByText("Managed worktree · inspection only")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stage source.py" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Commit staged changes" })).toBeDisabled();
  });
  it("blocks branch switching with a live unsaved buffer before any mutation request", async () => {
    provide();
    render(<SourceControlPanel {...shell(true)} />);
    fireEvent.click(await screen.findByRole("button", { name: "Stage source.py" }));
    expect(await screen.findByText("Git operation confirmed")).toBeInTheDocument();
    fireEvent.click(await screen.findByText("Branches", { exact: true }));
    fireEvent.click(await screen.findByRole("button", { name: "feature" }));
    expect(await screen.findByText(/Unsaved editor buffers are preserved/)).toBeInTheDocument();
    expect(screen.queryByText("Git operation confirmed")).not.toBeInTheDocument();
    expect(
      mocks.request.mock.calls.filter(
        ([path, init]) => path.endsWith("/branches") && init?.method === "POST"
      )
    ).toHaveLength(0);
  });
  it("drops delayed context from a previously selected project", async () => {
    const finish: Record<string, (value: { label: string }) => void> = {};
    mocks.request.mockImplementation(
      (path: string) =>
        new Promise((resolve) => {
          finish[path] = resolve;
        })
    );
    function View({ project }: { project: string }) {
      const data = useSourceData<{ label: string }>(project, "status");
      return <output>{data.value?.label ?? "Loading"}</output>;
    }
    const ui = render(<View project={A} />);
    ui.rerender(<View project={B} />);
    await act(async () =>
      finish[`/projects/${A}/source-control/status`]({ label: "A private Git state" })
    );
    expect(screen.queryByText("A private Git state")).not.toBeInTheDocument();
    await act(async () => finish[`/projects/${B}/source-control/status`]({ label: "B state" }));
    expect(screen.getByText("B state")).toBeInTheDocument();
  });
  it("represents binary comparisons without pretending bytes are text", async () => {
    mocks.request.mockResolvedValue({
      ...diff,
      before: null,
      after: null,
      limited: true,
      reason: "Binary content",
      hunks: [],
    });
    render(
      <SourceDiff projectId={A} rootId="r_primary" path="binary.dat" comparison="working_index" />
    );
    expect(await screen.findByText("Binary content")).toBeInTheDocument();
    expect(screen.queryByLabelText("CodeMirror comparison")).not.toBeInTheDocument();
  });
});

describe("GitHub publication UI", () => {
  it("does not contact or mutate GitHub merely by mounting", () => {
    render(<GitHubPanel projectId={A} />);
    expect(mocks.request).not.toHaveBeenCalled();
  });
  it("requires explicit scoped authorization and preserves accurate unknown check state", async () => {
    mocks.request.mockResolvedValue({
      repository: {
        id: 7,
        fullName: "owner/disposable",
        url: "https://github.com/owner/disposable",
        permissions: { push: true },
      },
      preview: {
        repository: "owner/disposable",
        base: "main",
        head: "factory/run-1",
        title: "Real title",
        body: "Actual evidence",
        draft: true,
        eligible: true,
        blockers: [],
      },
      delivery: { integrationHead: "f".repeat(40), state: "ready" },
      changedFiles: ["source.py"],
      authorizationBlocker: null,
      externalActions: ["Push integration branch"],
      publishedByPreview: false,
    });
    render(<GitHubPanel projectId={A} runId={1} />);
    fireEvent.click(screen.getByRole("button", { name: /Preview delivery/ }));
    const publish = await screen.findByRole("button", { name: "Publish authorized delivery" });
    expect(publish).toBeDisabled();
    expect(mocks.request.mock.calls.filter(([, init]) => init?.method === "POST")).toHaveLength(0);
    fireEvent.click(screen.getByLabelText(/I authorize these external actions/));
    expect(publish).toBeEnabled();
    fireEvent.click(publish);
    await waitFor(() =>
      expect(mocks.request.mock.calls.some(([, init]) => init?.method === "POST")).toBe(true)
    );
    const [, init] = mocks.request.mock.calls.find(([, init]) => init?.method === "POST")!;
    expect(JSON.parse(init.body)).toMatchObject({
      authorize: true,
      repository: "owner/disposable",
      repositoryId: 7,
      expectedIntegration: "f".repeat(40),
      base: "main",
      head: "factory/run-1",
    });
  });
});
