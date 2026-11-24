import { afterEach, beforeAll, afterAll, describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryEditor } from "./MemoryEditor";
import { MemoryPanel } from "./MemoryPanel";
import { MemoryUsage } from "./MemoryUsage";
import { useMemoryData } from "./client";
import type { MemoryRecord, Settings } from "./types";
import type { DockSession } from "../workspace/production/controller";

const A = "p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B = "p_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const mocks = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("../../api", () => ({ request: mocks.request }));
vi.mock("../workspace/production/data", () => ({
  useData: () => ({
    value: { roots: [{ id: "r_primary", kind: "repository", label: "Repository" }] },
  }),
}));
const record: MemoryRecord = {
  schemaVersion: 1,
  id: "pm_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  projectId: A,
  revision: 3,
  category: "architecture",
  title: "Database adapters",
  content: "Database access uses repository adapters.",
  subject: "database",
  scope: "api",
  status: "active",
  origin: "user_authored",
  provenance: [
    {
      source: { projectId: A, reference: { kind: "manual", note: "User decision" } },
      sourceRevision: "user_authored",
      fingerprint: "digest",
      validationStatus: "user_assertion",
      checkedAt: "2026-11-08T00:00:00Z",
    },
  ],
  createdAt: "2026-11-08T00:00:00Z",
  updatedAt: "2026-11-08T00:00:00Z",
  changeReason: "Manual",
  relatedIds: [],
};
const settings: Settings = {
  revision: 2,
  enabled: true,
  extractionMode: "review",
  retrievalEnabled: true,
  categories: ["architecture"],
  maxContextRecords: 6,
  maxContextBytes: 12000,
};
const originalShow = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
const originalClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute("open");
  };
});
afterAll(() => {
  if (originalShow) Object.defineProperty(HTMLDialogElement.prototype, "showModal", originalShow);
  else Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
  if (originalClose) Object.defineProperty(HTMLDialogElement.prototype, "close", originalClose);
  else Reflect.deleteProperty(HTMLDialogElement.prototype, "close");
});
afterEach(() => {
  cleanup();
  mocks.request.mockReset();
});

describe("memory governance", () => {
  it("retains the edited draft when the backend rejects a stale revision", async () => {
    mocks.request.mockRejectedValue(new Error("stale_memory_revision"));
    render(<MemoryEditor projectId={A} previous={record} done={vi.fn()} cancel={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Project fact"), {
      target: { value: "A corrected database convention" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save memory" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("stale_memory_revision");
    expect(screen.getByLabelText("Project fact")).toHaveValue("A corrected database convention");
    const [path, init] = mocks.request.mock.calls[0];
    expect(path).toBe(`/projects/${A}/memory/${record.id}`);
    expect(JSON.parse(init.body).expectedRevision).toBe(3);
  });
  it("creates a source-labelled memory only after SQLite acknowledgement", async () => {
    let finish!: (value: MemoryRecord) => void;
    mocks.request.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      })
    );
    const done = vi.fn();
    render(<MemoryEditor projectId={A} done={done} cancel={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Title"), { target: { value: "Project decision" } });
    fireEvent.change(screen.getByLabelText("Project fact"), {
      target: { value: "Use repository adapters" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save memory" }));
    expect(done).not.toHaveBeenCalled();
    expect(JSON.parse(mocks.request.mock.calls[0][1].body).memory.sources[0]).toEqual({
      projectId: A,
      reference: { kind: "manual", note: "Explicit user-authored project fact" },
    });
    await act(async () => finish(record));
    expect(done).toHaveBeenCalledWith(record);
  });
  it("does not display a delayed response from another project", async () => {
    const finish: Record<string, (value: { name: string }) => void> = {};
    mocks.request.mockImplementation(
      (path: string) =>
        new Promise((resolve) => {
          finish[path] = resolve;
        })
    );
    function View({ project }: { project: string }) {
      const value = useMemoryData<{ name: string }>(project, "/settings");
      return <output>{value.value?.name ?? "Loading"}</output>;
    }
    const ui = render(<View project={A} />);
    ui.rerender(<View project={B} />);
    await act(async () =>
      finish[`/projects/${A}/memory/settings`]({ name: "A private convention" })
    );
    expect(screen.queryByText("A private convention")).not.toBeInTheDocument();
    await act(async () => finish[`/projects/${B}/memory/settings`]({ name: "B convention" }));
    expect(screen.getByText("B convention")).toBeInTheDocument();
  });
  it("requires deletion confirmation and removes the acknowledged record from the view", async () => {
    let deleted = false;
    mocks.request.mockImplementation(async (path: string, init?: RequestInit) => {
      if (init?.method === "DELETE") {
        deleted = true;
        return { deleted: true };
      }
      if (path.endsWith("/settings")) return settings;
      if (path.includes("memory?"))
        return { items: deleted ? [] : [record], nextCursor: null, searchMode: "sqlite_fts5" };
      if (path.endsWith(record.id)) return record;
      return [];
    });
    render(<MemoryPanel projectId={A} dock={{} as DockSession} initialId={record.id} />);
    fireEvent.click(await screen.findByRole("button", { name: "Delete memory" }));
    expect(deleted).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Delete permanently" }));
    await waitFor(() => expect(deleted).toBe(true));
    expect(
      await screen.findByText("Memory deleted from active storage and retrieval.")
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("article", { name: `Memory: ${record.title}` })
    ).not.toBeInTheDocument();
    const call = mocks.request.mock.calls.find(([, init]) => init?.method === "DELETE")!;
    expect(call[0]).toBe(`/projects/${A}/memory/${record.id}`);
    expect(JSON.parse(call[1].body)).toEqual({ expectedRevision: 3 });
  });
  it("renders model-like HTML and links as inert text", async () => {
    const unsafe = {
      ...record,
      content: '<img src=x onerror="alert(1)"> [execute](javascript:alert(1))',
    };
    mocks.request.mockImplementation(async (path: string) =>
      path.endsWith("/settings")
        ? settings
        : path.includes("memory?")
          ? { items: [unsafe], nextCursor: null }
          : unsafe
    );
    const { container } = render(
      <MemoryPanel projectId={A} dock={{} as DockSession} initialId={record.id} />
    );
    expect(await screen.findByRole("article")).toHaveTextContent("<img src=x");
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector('a[href^="javascript:"]')).toBeNull();
  });
  it("keeps a settings draft after a concurrent policy edit conflict", async () => {
    mocks.request.mockImplementation(async (path: string, init?: RequestInit) => {
      if (init?.method === "PUT") throw new Error("stale_memory_revision");
      return path.endsWith("/settings") ? settings : { items: [], nextCursor: null };
    });
    render(<MemoryPanel projectId={A} dock={{} as DockSession} />);
    await screen.findByText("No matching project memories");
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.change(screen.getByLabelText("Automatic extraction"), {
      target: { value: "automatic" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save memory policy" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("stale_memory_revision");
    expect(screen.getByLabelText("Automatic extraction")).toHaveValue("automatic");
    const call = mocks.request.mock.calls.find(([, init]) => init?.method === "PUT")!;
    expect(JSON.parse(call[1].body).revision).toBe(2);
  });
  it("loads usage only when expanded and displays historical revisions without claiming reliance", async () => {
    mocks.request.mockResolvedValue([
      {
        id: 1,
        role: "worker",
        usedAt: "then",
        contextHash: "snapshot-hash",
        memories: [{ memoryId: record.id, revision: 1, contentHash: "historical-hash" }],
      },
    ]);
    render(<MemoryUsage projectId={A} sessionId={42} />);
    expect(mocks.request).not.toHaveBeenCalled();
    const detail = screen
      .getByText("Project memory supplied to this invocation")
      .closest("details")!;
    detail.open = true;
    fireEvent(detail, new Event("toggle"));
    await waitFor(() => expect(mocks.request).toHaveBeenCalled());
    expect(await screen.findByText(/revision 1/)).toBeInTheDocument();
    expect(screen.getByText(/model reliance is not inferred/)).toBeInTheDocument();
    expect(screen.queryByText(record.content)).not.toBeInTheDocument();
  });
});
