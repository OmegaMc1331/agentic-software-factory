import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, cleanup, act } from "@testing-library/react";
import type { request } from "../../api";
import { ConversationApi, ConversationStore, conversationPath } from "./client";
import { validateGraph } from "./validation";
import type { ConversationEvent, Plan } from "./types";
import { MissionForm } from "./MissionForm";
import { MessageRow } from "./ConversationPanel";
import type { WorkspaceView } from "../workspace/production/model";
import type { DockSession } from "../workspace/production/controller";
import type { Message } from "./types";
vi.mock("../workspace/production/data", () => ({
  useData: (_project: string, path: string) => ({
    value:
      path === "roles"
        ? [
            {
              id: "planner",
              name: "Planner",
              assignments: [{ agent: "planner", preferred: true }],
            },
            { id: "worker", name: "Worker", assignments: [{ agent: "worker", preferred: true }] },
            {
              id: "reviewer",
              name: "Reviewer",
              assignments: [{ agent: "reviewer", preferred: true }],
            },
            {
              id: "researcher",
              name: "Researcher",
              assignments: [{ agent: "verifier", preferred: true }],
            },
            { id: "architect", name: "Architect", assignments: [] },
          ]
        : [],
    loading: false,
  }),
}));
vi.mock("@xyflow/react", () => ({
  ReactFlow: () => <div aria-label="Executable graph" />,
  Background: () => null,
  useNodesState: (initial: unknown) => [initial, vi.fn(), vi.fn()],
}));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
const project = "p_" + "a".repeat(32),
  id = "c_" + "b".repeat(32);
function event(n: number, conversation_id = id): ConversationEvent {
  return {
    schema_version: 1,
    id: n,
    conversation_id,
    event_type: "conversation.response.completed",
    payload: {},
  };
}
const plan: Plan = {
  objective: "test",
  tasks: [
    {
      id: "T1",
      title: "One",
      objective: "implement",
      dependencies: [],
      acceptanceCriteria: ["works"],
      role: "worker",
    },
    {
      id: "T2",
      title: "Two",
      objective: "verify",
      dependencies: ["T1"],
      acceptanceCriteria: ["tested"],
      role: "worker",
    },
  ],
};
describe("conversation project and event ownership", () => {
  it("uses both project and conversation identity in every endpoint", async () => {
    const api = vi.fn().mockResolvedValue([]);
    const client = new ConversationApi(project, id, api as typeof request);
    await client.messages();
    await client.missions();
    await client.cancel(1);
    expect(
      api.mock.calls.every((call) =>
        String(call[0]).startsWith(`/projects/${project}/conversations/${id}/`)
      )
    ).toBe(true);
    expect(conversationPath(project, id)).toContain(project);
  });
  it("deduplicates old/overlapping events and refuses foreign conversation events", async () => {
    const api = vi.fn().mockResolvedValue([]);
    const store = new ConversationStore(project, id, api as typeof request);
    store.accept(event(2));
    store.accept(event(2));
    store.accept(event(1));
    store.accept(event(500, "c_other"));
    await store.refresh();
    expect(store.snapshot.cursor).toBe(2);
    expect(store.snapshot.messages).toEqual([]);
    store.stop();
  });
  it("keeps late Project A responses in Project A's own store", async () => {
    let resolve!: (value: unknown) => void;
    const delayed = new Promise((r) => {
      resolve = r;
    });
    const a = new ConversationStore(
      project,
      id,
      vi
        .fn()
        .mockImplementation((path) =>
          String(path).includes("messages") ? delayed : Promise.resolve([])
        ) as typeof request
    );
    const b = new ConversationStore(
      "p_" + "c".repeat(32),
      id,
      vi.fn().mockResolvedValue([]) as typeof request
    );
    const pending = a.refresh();
    await b.refresh();
    resolve([{ id: 1, conversation_id: id, content: "A only" }]);
    await pending;
    expect(a.snapshot.messages[0].content).toBe("A only");
    expect(b.snapshot.messages).toEqual([]);
  });
  it("retains an update arriving while a history request is pending", async () => {
    let resolve!: (value: unknown) => void;
    let reads = 0;
    const first = new Promise((r) => {
      resolve = r;
    });
    const api = vi.fn().mockImplementation((path) => {
      if (!String(path).includes("messages")) return Promise.resolve([]);
      reads++;
      return reads === 1
        ? first
        : Promise.resolve([{ id: 1, conversation_id: id, content: "new" }]);
    });
    const store = new ConversationStore(project, id, api as typeof request);
    const pending = store.refresh();
    store.accept(event(1));
    resolve([]);
    await pending;
    await vi.waitFor(() => expect(store.snapshot.messages[0]?.content).toBe("new"));
    expect(reads).toBe(2);
  });
  it("surfaces identity mismatch without replacing retained history", async () => {
    const store = new ConversationStore(
      project,
      id,
      vi
        .fn()
        .mockImplementation((path) =>
          Promise.resolve(
            String(path).includes("messages") ? [{ id: 1, conversation_id: "foreign" }] : []
          )
        ) as typeof request
    );
    await store.refresh();
    expect(store.snapshot.error).toContain("identity mismatch");
    expect(store.snapshot.messages).toEqual([]);
  });
  it("preserves command keys in typed retriable submissions", async () => {
    const api = vi.fn().mockResolvedValue({ id: 1 });
    const client = new ConversationApi(project, id, api as typeof request);
    const body = {
      key: "durable-key",
      content: "objective",
      workspace_session_id: "s_" + "d".repeat(32),
      context: [],
    };
    await client.send(body);
    await client.send(body);
    expect(api.mock.calls[0][1].body).toBe(api.mock.calls[1][1].body);
  });
});
describe("executable mission plan", () => {
  it("accepts a real dependency DAG", () => {
    expect(validateGraph(plan)).toBe("");
  });
  it("refuses cycles and missing dependencies", () => {
    expect(
      validateGraph({
        ...plan,
        tasks: plan.tasks.map((t) => ({
          ...t,
          dependencies: t.id === "T1" ? ["T2"] : t.dependencies,
        })),
      })
    ).toContain("cycle");
    expect(
      validateGraph({ ...plan, tasks: [{ ...plan.tasks[0], dependencies: ["missing"] }] })
    ).toContain("unknown");
  });
  it("requires distinct IDs and acceptance criteria", () => {
    expect(validateGraph({ ...plan, tasks: [plan.tasks[0], plan.tasks[0]] })).toContain("unique");
    expect(
      validateGraph({ ...plan, tasks: [{ ...plan.tasks[0], acceptanceCriteria: [] }] })
    ).toContain("criterion");
  });
  it("submits the authored plan without asking the planner to replace it", async () => {
    const submit = vi.fn().mockResolvedValue(undefined);
    render(<MissionForm project={project} mode="visual" onSubmit={submit} onCancel={() => {}} />);
    fireEvent.change(screen.getByLabelText("Mission objective"), {
      target: { value: "Authored objective" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add task" }));
    for (const [label, value] of [
      ["Title T1", "Exact title"],
      ["Objective T1", "Exact task"],
      ["Acceptance T1", "Verified"],
    ])
      fireEvent.change(screen.getByLabelText(label), { target: { value } });
    fireEvent.click(screen.getByRole("button", { name: "Validate and launch plan" }));
    await vi.waitFor(() => expect(submit).toHaveBeenCalledOnce());
    expect(submit.mock.calls[0][0].plan.tasks[0].title).toBe("Exact title");
  });
  it("keeps the mission form intact after a failed submission", async () => {
    render(
      <MissionForm
        project={project}
        mode="form"
        onSubmit={() => Promise.reject(new Error("policy denied"))}
        onCancel={() => {}}
      />
    );
    fireEvent.change(screen.getByLabelText("Mission objective"), {
      target: { value: "Retain this objective" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Plan and start mission" }));
    expect(await screen.findByText("Error: policy denied")).toBeTruthy();
    expect((screen.getByLabelText("Mission objective") as HTMLTextAreaElement).value).toBe(
      "Retain this objective"
    );
  });
  it("keeps assigned optional roles and omits empty roles when choosing a preferred agent", async () => {
    const submit = vi.fn().mockResolvedValue(undefined);
    render(<MissionForm project={project} mode="form" onSubmit={submit} onCancel={() => {}} />);
    fireEvent.change(screen.getByLabelText("Mission objective"), {
      target: { value: "Repair interval behavior" },
    });
    fireEvent.change(screen.getByLabelText("Preferred worker"), {
      target: { value: "worker" },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Plan and start mission" }));
    });
    await vi.waitFor(() => expect(submit).toHaveBeenCalledOnce());
    expect(submit.mock.calls[0][0].team).toEqual({
      planner: "planner",
      workers: ["worker"],
      reviewers: ["reviewer"],
      additional: { researcher: ["verifier"] },
    });
  });
});

describe("untrusted output rendering", () => {
  it("opens a read-only existing mission reference in its owning project", () => {
    const open = vi.fn();
    const message: Message = {
      id: 1,
      conversation_id: id,
      role: "orchestrator",
      content: "Workflow #1 is completed.",
      status: "finished",
      run_id: null,
      parent_message_id: null,
      created_at: "now",
      provenance: {
        source: "runtime",
        snapshot: {
          run_id: 1,
          objective: "Existing mission",
          status: "completed",
          completed: 1,
          total: 1,
          terminal: true,
          tasks: [],
          sessions: [],
          attempts: [],
          changed_files: [],
          integration_sha: null,
          artifacts: [],
        },
      },
    };
    render(
      <MessageRow
        message={message}
        view={{ projectId: project } as WorkspaceView}
        dock={{ open } as unknown as DockSession}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Workflow #1" }));
    expect(open).toHaveBeenCalledWith("progress", { projectId: project, resourceId: 1 }, "right");
    fireEvent.click(screen.getByRole("button", { name: "Evidence and review" }));
    expect(open).toHaveBeenCalledWith("workflow", { projectId: project, resourceId: 1 });
    expect(screen.getByText("completed", { exact: true }).textContent).toBe("completed");
  });
  it("renders HTML, scripts and unsafe links as text and keeps code readable", () => {
    const open = vi.fn();
    const view = { projectId: project } as WorkspaceView;
    const dock = { open } as unknown as DockSession;
    const message = {
      id: 1,
      conversation_id: id,
      role: "orchestrator",
      content:
        '<img src=x onerror="alert(1)"><script>alert(2)</script> [unsafe](javascript:alert(3))\n```ts\nconst value = 42;\n```',
      status: "finished",
      run_id: null,
      parent_message_id: 0,
      provenance: { source: "agent", agent_session_id: 7 },
      created_at: "now",
    } as Message;
    const { container } = render(<MessageRow message={message} view={view} dock={dock} />);
    expect(container.querySelector("script,img,a")).toBeNull();
    expect(container.querySelector("code")?.textContent).toContain("const value = 42;");
    fireEvent.click(screen.getByRole("button", { name: "Agent session" }));
    expect(open).toHaveBeenCalledWith("console", { projectId: project, resourceId: 7 });
  });
  it("copies a failed objective only on explicit user action", () => {
    const retry = vi.fn();
    const message = {
      id: 1,
      role: "orchestrator",
      content: "Policy refused",
      status: "failed",
      provenance: { error: true },
      run_id: null,
    } as Message;
    render(
      <MessageRow
        message={message}
        view={{ projectId: project } as WorkspaceView}
        dock={{} as DockSession}
        onRetry={retry}
      />
    );
    expect(retry).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("Policy refused");
    fireEvent.click(screen.getByRole("button", { name: "Copy objective to composer" }));
    expect(retry).toHaveBeenCalledOnce();
  });
});
