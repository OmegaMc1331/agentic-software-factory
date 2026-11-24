import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FactoryApiError, type request } from "../../../api";
import { WorkspaceModel, DockSession, LAST_WORKSPACE, RECOVERY_WORKSPACE } from "./controller";
import { createSession, createView, decodeWorkspace, emptyWorkspace, validLayout } from "./model";
import { ResourceCache } from "./data";
import type { EditorDocument } from "../../files/documents";
const a = `p_${"a".repeat(32)}`,
  b = `p_${"b".repeat(32)}`;
function document() {
  const doc = emptyWorkspace(),
    session = createSession(a);
  session.views = [createView(session, "scratch")];
  session.views[0].state = { draft: "retained note", dirty: true, anchor: 4, head: 6 };
  doc.sessions = [session];
  doc.openProjects = [a];
  doc.activeProjectId = a;
  doc.activeSessionId = session.id;
  return doc;
}
beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
});
it("keeps a pinned foreign project's mission ownership separate from its presentation session", () => {
  const model = new WorkspaceModel(vi.fn() as typeof request),
    doc = document();
  model.state = { ...model.state, ready: true, document: doc };
  const owner = model.missionSession(b, doc.activeSessionId!);
  expect(owner).not.toBe(doc.activeSessionId);
  expect(model.state.document.activeProjectId).toBe(a);
  expect(model.state.document.activeSessionId).toBe(doc.activeSessionId);
  expect(model.state.document.sessions.find((s) => s.id === owner)?.projectId).toBe(b);
  expect(model.missionSession(b, doc.activeSessionId!)).toBe(owner);
  model.dispose();
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});
describe("repository view keyboard focus", () => {
  it("closes an ownerless restored file view without offering another document's save actions", () => {
    const model = new WorkspaceModel(vi.fn() as typeof request),
      doc = document(),
      session = doc.sessions[0];
    const view = createView(session, "editor", { documentId: `d_${"f".repeat(32)}` });
    view.state.dirty = true;
    session.views = [view];
    model.state = { ...model.state, document: doc };
    const dock = new DockSession(model, session.id),
      close = vi.spyOn(dock, "close").mockImplementation(() => {});
    dock.requestClose(view.id);
    expect(close).toHaveBeenCalledWith(view.id);
    expect(model.state.confirmClose).toBeNull();
    model.dispose();
  });
  it.each([
    { project_id: a, root_id: `r_${"c".repeat(32)}`, path: "src/main.rs" },
    { project_id: a, root_id: `r_${"b".repeat(32)}`, path: "src/other.rs" },
  ])("does not retarget a colliding view after another file changes: %j", (foreign) => {
    const model = new WorkspaceModel(vi.fn() as typeof request),
      doc = document(),
      session = doc.sessions[0];
    const file = { project_id: a, root_id: `r_${"b".repeat(32)}`, path: "src/main.rs" },
      id = `d_${"d".repeat(32)}`;
    session.views = [
      createView(session, "editor", { file, documentId: id }),
      createView(session, "editor", { file: foreign, documentId: id }),
    ];
    model.state = { ...model.state, document: doc };
    const event = { id, file, dirty: true, rootLabel: "Repository" } as EditorDocument;
    (model as unknown as { documentChanged(document: EditorDocument): void }).documentChanged(
      event
    );
    expect(model.session!.views[0].state.dirty).toBe(true);
    expect(model.session!.views[1].file).toEqual(foreign);
    expect(model.session!.views[1].state.dirty).not.toBe(true);
    model.dispose();
  });
  it.each([
    {
      markup: "<button disabled>Save</button><button data-target>Save all</button>",
      selector: "[data-target]",
    },
    {
      markup:
        '<button>Save all</button><div role="textbox" tabindex="0" contenteditable="false">Read-only source</div>',
      selector: '[role="textbox"]',
    },
  ])("focuses an enabled control or inspectable source: $selector", ({ markup, selector }) => {
    const model = new WorkspaceModel(vi.fn() as typeof request);
    const dock = new DockSession(model, createSession(a).id);
    const root = globalThis.document.createElement("div");
    root.tabIndex = -1;
    root.innerHTML = markup;
    globalThis.document.body.append(root);
    dock.api = { activePanel: { view: { content: { element: root } } } } as never;
    dock.focus();
    expect(globalThis.document.activeElement).toBe(root.querySelector(selector));
    root.remove();
    model.dispose();
  });
});
describe("production workspace schema", () => {
  it("round trips independent resource identities and dirty draft state", () => {
    const doc = document();
    const second = createSession(b);
    second.views = [createView(second, "workflow", { resourceId: 1 })];
    doc.sessions[0].views.push(createView(doc.sessions[0], "workflow", { resourceId: 1 }));
    doc.sessions.push(second);
    doc.openProjects.push(b);
    const restored = decodeWorkspace(JSON.parse(JSON.stringify(doc))).document;
    expect(restored.sessions[0].views[0].state.draft).toBe("retained note");
    expect(restored.sessions[0].views[1].projectId).toBe(a);
    expect(restored.sessions[1].views[0].projectId).toBe(b);
  });
  it("migrates supported version one presentation state", () => {
    const value = { ...document(), version: 1 };
    expect(decodeWorkspace(value).document.version).toBe(3);
    expect(decodeWorkspace(value).document.sessions[0].views[0].state.dirty).toBe(true);
  });
  it.each([null, {}, [], { version: 99 }, "malformed"])(
    "refuses malformed or unsupported document %j",
    (value) => {
      expect(() => decodeWorkspace(value)).toThrow();
    }
  );
  it("rejects duplicate session and view identities", () => {
    const doc = document();
    doc.sessions.push(doc.sessions[0]);
    expect(() => decodeWorkspace(doc)).toThrow();
    const other = document();
    other.sessions[0].views.push(other.sessions[0].views[0]);
    expect(() => decodeWorkspace(other)).toThrow();
  });
  it("recovers unknown views without discarding other drafts", () => {
    const doc = document();
    const raw = JSON.parse(JSON.stringify(doc));
    raw.sessions[0].views.push({
      ...raw.sessions[0].views[0],
      id: `v_${"c".repeat(32)}`,
      kind: "untrusted-plugin",
    });
    const result = decodeWorkspace(raw);
    expect(result.issues).toHaveLength(1);
    expect(result.document.sessions[0].views[0].state.draft).toBe("retained note");
    expect(result.document.sessions[0].views[1].kind).toBe("home");
  });
  it("rejects arbitrary Dockview components, popouts and malformed split trees", () => {
    const doc = document(),
      view = doc.sessions[0].views[0];
    const layout = {
      grid: {
        width: 1000,
        height: 800,
        orientation: "HORIZONTAL",
        root: { type: "leaf", data: { id: "group", views: [view.id], activeView: view.id } },
      },
      panels: { [view.id]: { id: view.id, title: view.title, contentComponent: "workspace" } },
      activeGroup: "group",
    };
    expect(validLayout(layout, doc.sessions[0].views)).toBe(true);
    expect(
      validLayout(
        { ...layout, popoutGroups: [{ url: "https://example.test" }] },
        doc.sessions[0].views
      )
    ).toBe(false);
    expect(
      validLayout(
        {
          ...layout,
          panels: { [view.id]: { ...layout.panels[view.id], contentComponent: "arbitrary" } },
        },
        doc.sessions[0].views
      )
    ).toBe(false);
    doc.sessions[0].layout = layout as never;
    const raw = JSON.parse(JSON.stringify(doc));
    raw.sessions[0].layout.grid.root.data.views = ["missing"];
    const result = decodeWorkspace(raw);
    expect(result.document.sessions[0].layout).toBeNull();
    expect(result.document.sessions[0].views[0].state.draft).toBe("retained note");
    expect(result.issues).toHaveLength(1);
  });
  it("bounds document, session and note storage", () => {
    const doc = document();
    doc.sessions[0].views[0].state.draft = "x".repeat(1_048_577);
    expect(() => decodeWorkspace(doc)).toThrow();
  });
  it("allows a deliberately pinned cross-project view with full identity", () => {
    const doc = document();
    doc.sessions[0].views.push(createView(doc.sessions[0], "workflows", { projectId: b }));
    const restored = decodeWorkspace(doc).document;
    expect(restored.sessions[0].views[1].projectId).toBe(b);
    expect(restored.sessions[0].views[1].sessionId).toBe(doc.sessions[0].id);
  });
});
describe("workspace persistence and navigation", () => {
  it("clears the local-layout recovery notice once Rust acknowledges its save", async () => {
    const doc = document();
    localStorage.setItem(LAST_WORKSPACE, doc.id);
    sessionStorage.setItem(RECOVERY_WORKSPACE, JSON.stringify({ revision: 3, document: doc }));
    const api = vi
      .fn()
      .mockResolvedValueOnce({ revision: 3, document: doc })
      .mockResolvedValueOnce({ revision: 4, document: doc });
    const model = new WorkspaceModel(api as typeof request);
    await model.load();
    expect(model.state.issue).toContain("Recovered local presentation");
    await model.save();
    expect(model.state.pending).toBe(false);
    expect(model.state.issue).toBe("");
    model.dispose();
  });
  it("restores durable state through reads without starting project or workflow operations", async () => {
    const doc = document();
    localStorage.setItem(LAST_WORKSPACE, doc.id);
    const api = vi.fn().mockResolvedValue({ revision: 3, document: doc });
    const model = new WorkspaceModel(api as typeof request);
    await model.load();
    expect(model.state.revision).toBe(3);
    expect(model.session?.views[0].state.dirty).toBe(true);
    expect(api.mock.calls.every((call) => call[1] === undefined)).toBe(true);
    model.dispose();
  });
  it("does not deserialize P02 fixture state into production", async () => {
    localStorage.setItem(
      "asf.v2.p02.workspace.atlas.conversation",
      JSON.stringify({ messages: [{ body: "fixture" }] })
    );
    const api = vi.fn().mockResolvedValue([]);
    const model = new WorkspaceModel(api as typeof request);
    await model.load();
    expect(model.state.document.sessions).toEqual([]);
    expect(model.state.ready).toBe(true);
    model.dispose();
  });
  it("preserves malformed backend documents and offers a new independent identity", async () => {
    const id = `w_${"a".repeat(32)}`;
    localStorage.setItem(LAST_WORKSPACE, id);
    const api = vi.fn().mockResolvedValue({ revision: 5, document: { version: 99 } });
    const model = new WorkspaceModel(api as typeof request);
    await model.load();
    expect(model.state.conflict).toBe(true);
    expect(model.state.document.id).not.toBe(id);
    expect(api).toHaveBeenCalledTimes(1);
    model.dispose();
  });
  it("creates independent workspace sessions without numeric AgentSession IDs", () => {
    const model = new WorkspaceModel(vi.fn() as typeof request);
    model.activateProject(a);
    const first = model.session!.id;
    model.newSession();
    const second = model.session!.id;
    model.activateProject(b);
    expect(model.session!.projectId).toBe(b);
    model.activateSession(first);
    expect(model.session!.id).toBe(first);
    expect(second).not.toBe(first);
    expect(first).toMatch(/^s_[0-9a-f]{32}$/);
    model.archiveSession();
    expect(model.state.document.sessions.find((s) => s.id === first)?.archived).toBe(true);
    model.dispose();
  });
  it("detects concurrent-window saves and never overwrites the stale revision", async () => {
    const api = vi
      .fn()
      .mockRejectedValue(new FactoryApiError("revision conflict", "http", { status: 409 }));
    const model = new WorkspaceModel(api as typeof request);
    model.activateProject(a);
    await model.save();
    expect(model.state.conflict).toBe(true);
    expect(model.state.pending).toBe(true);
    const recovery = JSON.parse(sessionStorage.getItem(RECOVERY_WORKSPACE)!);
    expect(recovery.document.activeProjectId).toBe(a);
    await model.save();
    expect(api).toHaveBeenCalledTimes(1);
    model.dispose();
  });
  it("refuses an additional project's first session when session capacity is exhausted", () => {
    const model = new WorkspaceModel(vi.fn() as typeof request),
      doc = document();
    doc.sessions = Array.from({ length: 32 }, () => createSession(a));
    doc.activeSessionId = doc.sessions[0].id;
    model.state = { ...model.state, document: doc };
    model.activateProject(b);
    expect(model.state.document.activeProjectId).toBe(a);
    expect(model.state.document.sessions).toHaveLength(32);
    expect(model.state.issue).toMatch(/session limit/);
    expect(() => decodeWorkspace(model.state.document)).not.toThrow();
    model.dispose();
  });
  it("refuses a preset that would exceed view capacity without clearing the existing layout", () => {
    const model = new WorkspaceModel(vi.fn() as typeof request),
      doc = document(),
      session = doc.sessions[0];
    session.views = Array.from({ length: 128 }, () => createView(session, "scratch"));
    model.state = { ...model.state, document: doc };
    const dock = new DockSession(model, session.id),
      clear = vi.fn();
    dock.api = { clear } as never;
    dock.preset("Review");
    expect(clear).not.toHaveBeenCalled();
    expect(model.session!.views).toHaveLength(128);
    expect(model.state.issue).toMatch(/128-view limit/);
    expect(() => decodeWorkspace(model.state.document)).not.toThrow();
    model.dispose();
  });
  it("restores the last selected navigation session even when timestamps are identical", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-27T12:00:00Z"));
    const model = new WorkspaceModel(vi.fn() as typeof request);
    model.activateProject(a);
    const first = model.session!.id;
    model.newSession();
    const second = model.session!.id;
    model.activateSession(first);
    model.activateProject(b);
    model.activateProject(a);
    expect(model.session!.id).toBe(first);
    model.activateSession(second);
    model.hideProject(a);
    model.activateProject(a);
    expect(model.session!.id).toBe(second);
    expect(decodeWorkspace(model.state.document).document.activeSessionId).toBe(second);
    model.dispose();
  });
  it("serializes saves and keeps edits made during an in-flight save", async () => {
    let finish!: (value: unknown) => void;
    const api = vi.fn().mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    const model = new WorkspaceModel(api as typeof request);
    model.activateProject(a);
    const saving = model.save();
    model.newSession();
    finish({ revision: 1, document: model.state.document });
    await saving;
    expect(model.state.pending).toBe(true);
    expect(model.state.revision).toBe(1);
    expect(model.state.document.sessions).toHaveLength(2);
    model.dispose();
  });
  it("reports quota failure without discarding live state", () => {
    const model = new WorkspaceModel(vi.fn() as typeof request);
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota");
    });
    model.activateProject(a);
    expect(model.state.document.activeProjectId).toBe(a);
    expect(model.state.issue).toMatch(/storage is full/);
    model.dispose();
  });
  it("closing a dirty view asks confirmation and retains its draft without API mutation", () => {
    const api = vi.fn(),
      model = new WorkspaceModel(api as typeof request);
    model.state = { ...model.state, document: document() };
    const dock = new DockSession(model, model.session!.id),
      id = model.session!.views[0].id;
    dock.requestClose(id);
    expect(model.state.confirmClose).toBe(id);
    dock.close(id);
    expect(model.session!.views[0].closed).toBe(true);
    expect(model.session!.views[0].state.draft).toBe("retained note");
    expect(api).not.toHaveBeenCalled();
    model.dispose();
  });
});
describe("project-keyed request ownership", () => {
  it("late A results cannot replace B", async () => {
    let finishA!: (value: unknown) => void;
    const read = vi.fn((key: string) =>
      key.includes(a)
        ? new Promise((resolve) => {
            finishA = resolve;
          })
        : Promise.resolve({ owner: b })
    );
    const cache = new ResourceCache(read);
    const keyA = `/projects/${a}/runs?limit=100`,
      keyB = `/projects/${b}/runs?limit=100`;
    const stopA = cache.subscribe(keyA, () => {}),
      stopB = cache.subscribe(keyB, () => {});
    await Promise.resolve();
    finishA({ owner: a });
    await Promise.resolve();
    await Promise.resolve();
    expect(cache.snapshot<{ owner: string }>(keyB).value?.owner).toBe(b);
    expect(cache.snapshot<{ owner: string }>(keyA).value?.owner).toBe(a);
    stopA();
    stopB();
    cache.dispose();
  });
  it("deduplicates multiple readers and aborts after the last subscriber leaves", () => {
    let signal!: AbortSignal;
    const read = vi.fn((_: string, s: AbortSignal) => {
      signal = s;
      return new Promise(() => {});
    });
    const cache = new ResourceCache(read);
    const one = cache.subscribe(`/projects/${a}/runs`, () => {}),
      two = cache.subscribe(`/projects/${a}/runs`, () => {});
    expect(read).toHaveBeenCalledTimes(1);
    one();
    expect(signal.aborted).toBe(false);
    two();
    expect(signal.aborted).toBe(true);
    cache.dispose();
  });
});
