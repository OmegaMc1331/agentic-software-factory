import "@testing-library/jest-dom/vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ProjectsView } from "./ProjectsView";
import { ThemeProvider } from "../design-system/themes/ThemeProvider";
import {
  applicationInfo,
  listProjects,
  projectMutation,
  projectPath,
  projectSnapshot,
  type Project,
  type ProjectSnapshot,
} from "./projectApi";
import { useProjectSnapshot } from "./useProjectSnapshot";
import { FactoryApiError } from "../../api";
vi.mock("./projectApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./projectApi")>()),
  applicationInfo: vi.fn(),
  listProjects: vi.fn(),
  projectMutation: vi.fn(),
  projectSnapshot: vi.fn(),
}));
const aid = `p_${"a".repeat(32)}`,
  bid = `p_${"b".repeat(32)}`;
function project(id: string, name: string, lifecycle: Project["lifecycle"] = "open"): Project {
  return {
    project_id: id,
    display_name: name,
    canonical_root: `C:/fixtures/${name}`,
    created_at: "2026-10-27",
    last_opened_at: null,
    registry_schema_version: 1,
    lifecycle,
    availability: "available",
    problem: null,
  };
}
function snapshot(id: string, label: string): ProjectSnapshot {
  return {
    projectId: id,
    runs: [],
    config: {
      agents: {},
      roles: {},
      role_assignments: [{ role: "worker", agent: label, preferred: true }],
    },
  };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
beforeEach(() => {
  localStorage.clear();
  vi.resetAllMocks();
  vi.stubGlobal("matchMedia", () => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }));
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute("open");
  };
  vi.mocked(listProjects).mockResolvedValue([project(aid, "A"), project(bid, "B")]);
  vi.mocked(applicationInfo).mockResolvedValue({
    default_project_id: aid,
    authorized_roots: ["C:/fixtures"],
    limits: { open_projects: 8 },
  });
  vi.mocked(projectSnapshot).mockImplementation(async (id) =>
    snapshot(id, id === aid ? "A only" : "B only")
  );
  vi.mocked(projectMutation).mockResolvedValue(project(aid, "A"));
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
it("retries a confirmed transient close refusal without force or replaying an uncertain operation", async () => {
  vi.mocked(projectMutation)
    .mockRejectedValueOnce(
      new FactoryApiError("project owns active work or live requests; close those first", "http", {
        status: 409,
      })
    )
    .mockResolvedValueOnce(project(aid, "A", "closed"));
  render(
    <ThemeProvider>
      <ProjectsView />
    </ThemeProvider>
  );
  await screen.findByRole("button", { name: "A" });
  fireEvent.click(screen.getAllByRole("button", { name: "Close when idle" })[0]);
  await waitFor(() => expect(projectMutation).toHaveBeenCalledTimes(2));
  expect(
    vi
      .mocked(projectMutation)
      .mock.calls.every(
        ([path, method, body]) =>
          path === projectPath(aid, "close") && method === "POST" && JSON.stringify(body) === "{}"
      )
  ).toBe(true);
});
it("does not retry a timeout whose close outcome is uncertain", async () => {
  vi.mocked(projectMutation).mockRejectedValue(
    new FactoryApiError("The operation may have completed", "timeout", { uncertain: true })
  );
  render(
    <ThemeProvider>
      <ProjectsView />
    </ThemeProvider>
  );
  await screen.findByRole("button", { name: "A" });
  fireEvent.click(screen.getAllByRole("button", { name: "Close when idle" })[0]);
  await screen.findAllByText("The operation may have completed");
  expect(projectMutation).toHaveBeenCalledTimes(1);
});
it("stops bounded close retries while Rust continues to report active ownership", async () => {
  vi.mocked(projectMutation).mockRejectedValue(
    new FactoryApiError("project owns active work or live requests; close those first", "http", {
      status: 409,
    })
  );
  render(
    <ThemeProvider>
      <ProjectsView />
    </ThemeProvider>
  );
  await screen.findByRole("button", { name: "A" });
  fireEvent.click(screen.getAllByRole("button", { name: "Close when idle" })[0]);
  await waitFor(() => expect(projectMutation).toHaveBeenCalledTimes(6), { timeout: 3000 });
  await screen.findAllByText("project owns active work or live requests; close those first");
  expect(
    vi
      .mocked(projectMutation)
      .mock.calls.every(([path, method]) => path === projectPath(aid, "close") && method === "POST")
  ).toBe(true);
});
it("routes numeric collisions through the authoritative project identity", () => {
  expect(projectPath(aid, "runs/1")).toBe(`/projects/${aid}/runs/1`);
  expect(projectPath(bid, "runs/1")).not.toBe(projectPath(aid, "runs/1"));
});
it.each(["../escape", "1", "p_bad"])("rejects malformed project id %s before requesting", (id) =>
  expect(() => projectPath(id, "config")).toThrow("Invalid project")
);
it("discards late A responses after switching to B even if a transport ignores abort", async () => {
  const a = deferred<ProjectSnapshot>(),
    b = deferred<ProjectSnapshot>();
  vi.mocked(projectSnapshot).mockImplementation((id) => (id === aid ? a.promise : b.promise));
  const { result, rerender } = renderHook(({ id }) => useProjectSnapshot(id, 0), {
    initialProps: { id: aid },
  });
  const signal = vi.mocked(projectSnapshot).mock.calls[0][1].signal!;
  rerender({ id: bid });
  expect(signal.aborted).toBe(true);
  await act(async () => b.resolve(snapshot(bid, "B only")));
  expect(result.current.snapshot?.config.role_assignments[0].agent).toBe("B only");
  await act(async () => a.resolve(snapshot(aid, "A only")));
  expect(result.current.snapshot?.projectId).toBe(bid);
  expect(result.current.snapshot?.config.role_assignments[0].agent).toBe("B only");
});
it("does not let an old project's error replace the selected project's result", async () => {
  const old = deferred<ProjectSnapshot>();
  vi.mocked(projectSnapshot).mockImplementation((id) =>
    id === aid ? old.promise : Promise.resolve(snapshot(bid, "B"))
  );
  const { result, rerender } = renderHook(({ id }) => useProjectSnapshot(id, 0), {
    initialProps: { id: aid },
  });
  rerender({ id: bid });
  await waitFor(() => expect(result.current.snapshot?.projectId).toBe(bid));
  await act(async () => old.reject(new Error("A failure")));
  expect(result.current.error).toBe("");
});
it("clears project data immediately when the selection closes", async () => {
  const { result, rerender } = renderHook(({ id }) => useProjectSnapshot(id, 0), {
    initialProps: { id: aid },
  });
  await waitFor(() => expect(result.current.snapshot).toBeDefined());
  rerender({ id: "" });
  expect(result.current.snapshot).toBeUndefined();
});
it("shows real project names and switches scoped configuration", async () => {
  render(
    <ThemeProvider>
      <ProjectsView />
    </ThemeProvider>
  );
  await screen.findByText(/A only/);
  fireEvent.click(screen.getByRole("button", { name: "B" }));
  await screen.findByText(/B only/);
  expect(screen.queryByText(/A only/)).not.toBeInTheDocument();
  expect(localStorage.getItem("asf.v2.projects.selection.v1")).toBe(bid);
});
it("requires explicit trust and sends entered paths only to registration", async () => {
  render(
    <ThemeProvider>
      <ProjectsView />
    </ThemeProvider>
  );
  await screen.findByText(/A only/);
  fireEvent.click(screen.getByRole("button", { name: "Register" }));
  const dialog = screen.getByRole("dialog");
  const submit = within(dialog).getByRole("button", { name: "Register repository" });
  expect(submit).toBeDisabled();
  fireEvent.change(within(dialog).getByLabelText("Repository directory"), {
    target: { value: "C:/fixtures/new" },
  });
  fireEvent.click(within(dialog).getByRole("checkbox"));
  fireEvent.submit(submit.closest("form")!);
  await waitFor(() =>
    expect(projectMutation).toHaveBeenCalledWith("/projects/register", "POST", {
      path: "C:/fixtures/new",
      display_name: undefined,
      trust_project: true,
    })
  );
});
it("keeps unavailable entries visible and supports explicit relink", async () => {
  vi.mocked(listProjects).mockResolvedValue([
    {
      ...project(aid, "Missing", "unavailable"),
      availability: "unavailable",
      problem: "Volume unavailable",
    },
  ]);
  render(
    <ThemeProvider>
      <ProjectsView />
    </ThemeProvider>
  );
  await screen.findByText("Volume unavailable");
  fireEvent.click(screen.getByRole("button", { name: "Relink" }));
  expect(screen.getByLabelText("Repository directory")).toHaveValue("C:/fixtures/Missing");
  expect(screen.getByRole("checkbox", { name: /different filesystem/ })).not.toBeChecked();
});
it("reports failures without claiming a project was created", async () => {
  vi.mocked(projectMutation).mockRejectedValue(new Error("Destination exists; nothing registered"));
  render(
    <ThemeProvider>
      <ProjectsView />
    </ThemeProvider>
  );
  await screen.findByText(/A only/);
  fireEvent.click(screen.getByRole("button", { name: "Create project" }));
  const dialog = screen.getByRole("dialog");
  fireEvent.change(within(dialog).getByLabelText("New destination directory"), {
    target: { value: "C:/fixtures/exists" },
  });
  fireEvent.change(within(dialog).getByLabelText("Display name"), { target: { value: "Exists" } });
  fireEvent.click(within(dialog).getByRole("checkbox"));
  fireEvent.submit(within(dialog).getByRole("button", { name: "Create project" }).closest("form")!);
  await waitFor(() =>
    expect(within(dialog).getByRole("alert")).toHaveTextContent("Destination exists")
  );
  expect(dialog).toHaveAttribute("open");
});
