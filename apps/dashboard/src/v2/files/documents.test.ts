import { afterEach, describe, expect, it, vi } from "vitest";
import { FactoryApiError } from "../../api";
import { DocumentManager } from "./documents";
import type { FileApi } from "./fileApi";
import type { DraftJournal, LocalDraft } from "./journal";
import {
  fileKey,
  token,
  type Draft,
  type DraftEnvelope,
  type FileRef,
  type FileSnapshot,
} from "./types";
import {
  createSession,
  createView,
  decodeWorkspace,
  emptyWorkspace,
} from "../workspace/production/model";

const a = `p_${"a".repeat(32)}`,
  b = `p_${"b".repeat(32)}`,
  root = `r_${"a".repeat(32)}`,
  otherRoot = `r_${"b".repeat(32)}`;
const file: FileRef = { project_id: a, root_id: root, path: "src/main.rs" };
function snapshot(file: FileRef, content = "fn main() {}\n", revision = "v_1"): FileSnapshot {
  return {
    file,
    content,
    revision,
    size: content.length,
    modified: "1",
    encoding: "utf-8",
    bom: false,
    newline: content.includes("\r\n") ? "crlf" : "lf",
    read_only: false,
    reason: null,
    mode: "text",
    durability: null,
  };
}
const managers: DocumentManager[] = [];
afterEach(() => {
  for (const manager of managers.splice(0)) manager.dispose();
  vi.restoreAllMocks();
});
function fixture() {
  const disks = new Map<string, FileSnapshot>([[fileKey(file), snapshot(file)]]),
    drafts = new Map<string, DraftEnvelope>(),
    locals = new Map<string, LocalDraft>();
  const missing = () =>
    new FactoryApiError("missing", "http", { status: 404, code: "file_not_found" });
  const api = {
    roots: vi.fn(async () => ({
      roots: [
        { id: root, kind: "repository", label: "Repository", read_only: false },
        { id: otherRoot, kind: "task_worktree", label: "Agent worktree", read_only: true },
      ],
    })),
    read: vi.fn(async (file: FileRef) => {
      const value = disks.get(fileKey(file));
      if (!value) throw missing();
      return structuredClone(value);
    }),
    draft: vi.fn(async (_file: FileRef, id: string) => {
      const value = drafts.get(id);
      if (!value) throw missing();
      return structuredClone(value);
    }),
    saveDraft: vi.fn(async (document: Draft, revision: number) => {
      const old = drafts.get(document.id);
      if ((old?.revision ?? 0) !== revision)
        throw new FactoryApiError("Recovery changed", "http", { status: 409 });
      const next = { revision: revision + 1, document: structuredClone(document) };
      drafts.set(document.id, next);
      return next;
    }),
    discardDraft: vi.fn(async (_file: FileRef, id: string, revision: number) => {
      if (drafts.get(id)?.revision !== revision)
        throw new FactoryApiError("Recovery changed", "http", { status: 409 });
      drafts.delete(id);
    }),
    save: vi.fn(async (file: FileRef, expected: string, content: string): Promise<FileSnapshot> => {
      const old = disks.get(fileKey(file));
      if (old?.revision !== expected)
        throw new FactoryApiError("Disk changed", "http", { status: 409 });
      const next = { ...old, content, revision: `${expected}+`, size: content.length };
      disks.set(fileKey(file), next);
      return structuredClone(next);
    }),
    changes: vi.fn(async () => ({ changes: [] })),
  };
  const journal: DraftJournal = {
    read: async (id) => structuredClone(locals.get(id)),
    write: async (value) => {
      locals.set(value.document.id, structuredClone(value));
    },
    remove: async (id) => {
      locals.delete(id);
    },
  };
  const host = { changed: vi.fn(), issue: vi.fn() };
  const manager = () => {
    const value = new DocumentManager(host, api as unknown as FileApi, journal);
    managers.push(value);
    return value;
  };
  return { disks, drafts, locals, api, journal, host, manager };
}
describe("production editor document ownership", () => {
  it("can reopen a released canonical alias without following a dangling document ID", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file),
      aliasId = token();
    const alias = { ...file, path: "SRC/Main.rs" };
    f.disks.set(fileKey(alias), snapshot(file));
    expect(await m.open(alias, aliasId)).toBe(id);
    m.releaseUnused(new Set());
    expect(m.get(id)).toBeUndefined();
    expect(await m.open(alias, aliasId)).toBe(aliasId);
    expect(m.get(aliasId, alias)?.file).toEqual(file);
  });
  it.each([
    { ...file, project_id: b },
    { ...file, root_id: otherRoot },
    { ...file, path: "src/other.rs" },
  ])("rejects a restored document ID bound to another file: %j", async (foreign) => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    m.setText(id, "owned draft");
    expect(m.get(id, foreign)).toBeUndefined();
    await expect(m.open(foreign, id)).rejects.toThrow("different file");
    expect(await m.save(id, foreign)).toBe(false);
    expect(m.raw(m.get(id, file)!)).toBe("owned draft");
    expect(f.api.read).toHaveBeenCalledTimes(1);
    expect(f.api.save).not.toHaveBeenCalled();
  });
  it("forgets verified path aliases after a move instead of redirecting a recreated file", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    const alias = { ...file, path: "SRC/Main.rs" },
      to = { ...file, path: "src/moved.rs" };
    f.disks.set(fileKey(alias), snapshot(file));
    expect(await m.open(alias)).toBe(id);
    f.disks.set(fileKey(to), snapshot(to, "moved version", "v_2"));
    await m.moved({ from: file, to, kind: "file" });
    f.disks.set(fileKey(alias), snapshot(file, "recreated old path", "v_3"));
    const recreated = await m.open(alias);
    expect(recreated).not.toBe(id);
    expect(m.raw(m.get(recreated)!)).toBe("recreated old path");
    expect(m.get(id)?.file).toEqual(to);
  });
  it("discards a conflicted shared draft to the observed disk version without rewriting it", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    const first = m.pane(id, "first", () => [], {}),
      second = m.pane(id, "second", () => [], {});
    m.setText(id, "human draft");
    f.disks.set(fileKey(file), snapshot(file, "external version", "v_2"));
    await m.refresh(id);
    await m.discard(id);
    expect(first.state.doc.toString()).toBe("external version");
    expect(second.state.doc.toString()).toBe("external version");
    expect(m.get(id)?.baseRevision).toBe("v_2");
    expect(m.get(id)?.dirty).toBe(false);
    expect(f.api.save).not.toHaveBeenCalled();
    m.setText(id, "a new explicit edit");
    expect(await m.save(id)).toBe(true);
    expect(f.disks.get(fileKey(file))?.content).toBe("a new explicit edit");
  });
  it("ignores a disk read that arrives after a confirmed newer save", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    let finish!: (value: FileSnapshot) => void;
    f.api.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    const refreshing = m.refresh(id);
    await vi.waitFor(() => expect(f.api.read).toHaveBeenCalledTimes(2));
    m.setText(id, "confirmed newer save");
    expect(await m.save(id)).toBe(true);
    finish(snapshot(file));
    await refreshing;
    expect(m.raw(m.get(id)!)).toBe("confirmed newer save");
    expect(m.get(id)?.baseRevision).toBe("v_1+");
    expect(m.get(id)?.dirty).toBe(false);
  });
  it("keeps the newest refresh when responses arrive out of order", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    let first!: (value: FileSnapshot) => void, second!: (value: FileSnapshot) => void;
    f.api.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          first = resolve;
        })
    );
    f.api.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          second = resolve;
        })
    );
    const older = m.refresh(id),
      newer = m.refresh(id);
    await vi.waitFor(() => expect(f.api.read).toHaveBeenCalledTimes(3));
    second(snapshot(file, "newer", "v_3"));
    await newer;
    first(snapshot(file, "older", "v_2"));
    await older;
    expect(m.raw(m.get(id)!)).toBe("newer");
    expect(m.get(id)?.baseRevision).toBe("v_3");
  });
  it("waits for pending draft recovery before collecting Save All documents", async () => {
    const f = fixture(),
      first = f.manager(),
      id = await first.open(file);
    first.setText(id, "recovered before Save All");
    await first.persist(id);
    first.dispose();
    const second = f.manager();
    const opening = second.open(file, id);
    const results = await second.saveAll();
    await opening;
    expect(results).toHaveLength(1);
    expect(results[0].saved).toBe(true);
    expect(f.disks.get(fileKey(file))?.content).toBe("recovered before Save All");
  });
  it("does not replace edits typed while a disk reload is in flight", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    let finish!: (snapshot: FileSnapshot) => void;
    f.api.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    const reloading = m.reloadDisk(id);
    await vi.waitFor(() => expect(f.api.read).toHaveBeenCalledTimes(2));
    m.setText(id, "new edit during reload");
    finish(snapshot(file, "disk version", "v_2"));
    await reloading;
    expect(m.raw(m.get(id)!)).toBe("new edit during reload");
    expect(m.get(id)?.dirty).toBe(true);
    expect(m.get(id)?.error).toContain("No local changes were discarded");
  });
  it("drains pending local journal writes before explicit discard", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    m.setText(id, "discard me");
    await m.discard(id);
    await new Promise((resolve) => setTimeout(resolve, 350));
    expect(f.locals.has(id)).toBe(false);
    expect(f.drafts.has(id)).toBe(false);
    expect(m.get(id)?.dirty).toBe(false);
  });
  it("blocks edits to a read-only agent document", async () => {
    const f = fixture();
    f.disks.set(fileKey(file), { ...snapshot(file), read_only: true, reason: "agent_worktree" });
    const manager = f.manager(),
      id = await manager.open(file);
    manager.setText(id, "forbidden");
    expect(manager.get(id)?.dirty).toBe(false);
    expect(f.host.issue).toHaveBeenCalled();
    expect(f.api.save).not.toHaveBeenCalled();
  });
  it("preserves two recovery versions when windows collide and supports an explicit fork", async () => {
    const f = fixture(),
      first = f.manager(),
      second = f.manager(),
      id = token();
    await first.open(file, id);
    await second.open(file, id);
    first.setText(id, "window A");
    await first.persist(id);
    second.setText(id, "window B");
    await second.persist(id);
    expect(second.get(id)?.draftConflict).toBe(true);
    expect(f.drafts.get(id)?.document.content).toBe("window A");
    expect(second.raw(second.get(id)!)).toBe("window B");
    const independent = await second.fork(id);
    expect(independent).not.toBe(id);
    expect(f.drafts.get(independent)?.document.content).toBe("window B");
    expect(f.drafts.get(id)?.document.content).toBe("window A");
  });
  it("coalesces a canonical filesystem alias into the existing shared buffer", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file),
      alias = { ...file, path: "SRC/Main.rs" };
    f.disks.set(fileKey(alias), snapshot(file));
    expect(await m.open(alias)).toBe(id);
    expect(m.all()).toHaveLength(1);
  });
  it("can recover a server draft even if root discovery is temporarily unavailable", async () => {
    const f = fixture(),
      first = f.manager(),
      id = await first.open(file);
    first.setText(id, "durable recovery");
    await first.persist(id);
    f.locals.clear();
    f.api.roots.mockRejectedValue(new Error("git unavailable"));
    const second = f.manager();
    await second.open(file, id);
    expect(second.raw(second.get(id)!)).toBe("durable recovery");
    expect(second.get(id)?.recovered).toBe(true);
  });
  it("loads a real API-shaped snapshot and reuses one shared document", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    expect(m.raw(m.get(id)!)).toBe("fn main() {}\n");
    expect(await m.open({ ...file })).toBe(id);
    expect(f.api.read).toHaveBeenCalledTimes(1);
  });
  it("partitions identical paths by project and worktree", async () => {
    const f = fixture(),
      m = f.manager();
    const second = { ...file, project_id: b },
      worktree = { ...file, root_id: otherRoot };
    f.disks.set(fileKey(second), snapshot(second, "B"));
    f.disks.set(fileKey(worktree), { ...snapshot(worktree, "agent"), read_only: true });
    const ids = await Promise.all([m.open(file), m.open(second), m.open(worktree)]);
    expect(new Set(ids).size).toBe(3);
    m.setText(ids[0], "A edited");
    await m.save(ids[0]);
    expect(f.disks.get(fileKey(second))?.content).toBe("B");
    expect(f.disks.get(fileKey(worktree))?.content).toBe("agent");
  });
  it("preserves CRLF and exact final-newline state on save", async () => {
    const f = fixture();
    f.disks.set(fileKey(file), snapshot(file, "old\r\nend"));
    const m = f.manager(),
      id = await m.open(file);
    m.setText(id, "new\r\nend");
    expect(await m.save(id)).toBe(true);
    expect(f.api.save).toHaveBeenLastCalledWith(file, "v_1", "new\r\nend");
    expect(m.get(id)?.dirty).toBe(false);
  });
  it("preserves a dirty buffer on failed save", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    m.setText(id, "unsaved");
    f.api.save.mockRejectedValue(new FactoryApiError("Disk full", "http", { status: 503 }));
    expect(await m.save(id)).toBe(false);
    expect(m.get(id)?.dirty).toBe(true);
    expect(m.raw(m.get(id)!)).toBe("unsaved");
    expect(f.locals.get(id)?.document.content).toBe("unsaved");
  });
  it("does not mark edits made during a save as clean", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    let finish!: (value: FileSnapshot) => void;
    f.api.save.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        })
    );
    m.setText(id, "first");
    const saving = m.save(id);
    await vi.waitFor(() => expect(f.api.save).toHaveBeenCalled());
    m.setText(id, "second");
    finish(snapshot(file, "first", "v_2"));
    expect(await saving).toBe(true);
    expect(m.get(id)?.dirty).toBe(true);
    expect(m.raw(m.get(id)!)).toBe("second");
    expect(m.get(id)?.baseText.toString()).toBe("first");
  });
  it("updates clean documents on external change without changing pane selection", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    const pane = m.pane(id, "pane", () => [], { anchor: 3 });
    f.disks.set(fileKey(file), snapshot(file, "fn changed() {}\n", "v_2"));
    await m.refresh(id);
    expect(m.get(id)?.baseRevision).toBe("v_2");
    expect(pane.state.selection.main.anchor).toBe(3);
    expect(m.get(id)?.dirty).toBe(false);
  });
  it("keeps base, disk and dirty buffer separate on an agent change", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    m.setText(id, "human");
    f.disks.set(fileKey(file), snapshot(file, "agent", "v_2"));
    await m.refresh(id);
    expect(m.raw(m.get(id)!)).toBe("human");
    expect(m.get(id)?.baseText.toString()).toBe("fn main() {}\n");
    expect(m.get(id)?.disk?.content).toBe("agent");
    expect(m.get(id)?.conflict).toBe(true);
    expect(await m.save(id)).toBe(false);
    expect(f.api.save).not.toHaveBeenCalled();
  });
  it("revalidates the exact reviewed disk revision before saving a resolution", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    m.setText(id, "resolved");
    f.disks.set(fileKey(file), snapshot(file, "newer", "v_3"));
    expect(await m.saveResolved(id, "v_2")).toBe(false);
    expect(f.api.save).not.toHaveBeenCalled();
    expect(await m.saveResolved(id, "v_3")).toBe(true);
    expect(f.disks.get(fileKey(file))?.content).toBe("resolved");
  });
  it("recovers drafts after reload without writing disk", async () => {
    const f = fixture(),
      first = f.manager(),
      id = await first.open(file);
    first.setText(id, "recovered");
    await first.persist(id);
    first.dispose();
    const second = f.manager();
    await second.open(file, id);
    expect(second.raw(second.get(id)!)).toBe("recovered");
    expect(second.get(id)?.recovered).toBe(true);
    expect(f.api.save).not.toHaveBeenCalled();
    expect(f.disks.get(fileKey(file))?.content).toBe("fn main() {}\n");
  });
  it("retains a draft when the project or file is unavailable", async () => {
    const f = fixture(),
      first = f.manager(),
      id = await first.open(file);
    first.setText(id, "retain");
    await first.persist(id);
    first.dispose();
    f.disks.delete(fileKey(file));
    const second = f.manager();
    await second.open(file, id);
    expect(second.get(id)?.status).toBe("missing");
    expect(second.raw(second.get(id)!)).toBe("retain");
  });
  it("rejects a recovery record belonging to another file", async () => {
    const f = fixture(),
      id = token();
    f.drafts.set(id, {
      revision: 1,
      document: {
        schema: 1,
        id,
        file: { ...file, project_id: b },
        base_revision: "v_1",
        base_content: "base",
        content: "foreign",
        newline: "lf",
        bom: false,
        dirty: true,
      },
    });
    const m = f.manager();
    await m.open(file, id);
    expect(m.get(id)?.status).toBe("unavailable");
    expect(m.get(id)?.text.length).toBe(0);
    expect(f.drafts.has(id)).toBe(true);
  });
  it("coordinates multiple views through one shared text with independent selections", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file),
      first = m.pane(id, "one", () => [], { anchor: 2 }),
      second = m.pane(id, "two", () => [], { anchor: 6 });
    m.setText(id, "changed\n");
    expect(first.state.doc.toString()).toBe("changed\n");
    expect(second.state.doc.toString()).toBe("changed\n");
    expect(m.all()).toHaveLength(1);
  });
  it("renames an open dirty document without losing its buffer", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file),
      to = { ...file, path: "src/renamed.rs" };
    m.setText(id, "unsaved");
    f.disks.set(fileKey(to), snapshot(to, "fn main() {}\n", "v_renamed"));
    f.disks.delete(fileKey(file));
    await m.moved({ from: file, to, kind: "file" });
    expect(m.get(id)?.file).toEqual(to);
    expect(m.raw(m.get(id)!)).toBe("unsaved");
    expect(await m.save(id)).toBe(true);
    expect(f.disks.get(fileKey(to))?.content).toBe("unsaved");
  });
  it("reports a partial Save All without clearing failed documents", async () => {
    const f = fixture(),
      m = f.manager(),
      second = { ...file, project_id: b };
    f.disks.set(fileKey(second), snapshot(second));
    const aid = await m.open(file),
      bid = await m.open(second);
    m.setText(aid, "A");
    m.setText(bid, "B");
    f.disks.set(fileKey(second), snapshot(second, "external", "v_2"));
    const results = await m.saveAll();
    expect(results.map((result) => result.saved)).toEqual([true, false]);
    expect(m.get(bid)?.dirty).toBe(true);
  });
  it("surfaces local quota failures while retaining the live draft", async () => {
    const f = fixture();
    f.journal.write = async () => {
      throw new Error("quota full");
    };
    const m = f.manager(),
      id = await m.open(file);
    m.setText(id, "preserve");
    f.api.saveDraft.mockRejectedValue(new Error("server offline"));
    await m.persist(id);
    expect(m.get(id)?.dirty).toBe(true);
    expect(m.raw(m.get(id)!)).toBe("preserve");
    expect(m.get(id)?.recoverySaved).toBe(false);
    expect(m.needsUnloadWarning()).toBe(true);
  });
  it("does not evict dirty documents when clean views are released", async () => {
    const f = fixture(),
      m = f.manager(),
      id = await m.open(file);
    m.setText(id, "draft");
    m.releaseUnused(new Set());
    expect(m.get(id)?.dirty).toBe(true);
  });
  it("keeps file contents out of the versioned workspace layout", () => {
    const workspace = emptyWorkspace(),
      session = createSession(a);
    session.views = [createView(session, "editor", { file, documentId: token() })];
    workspace.sessions = [session];
    workspace.openProjects = [a];
    const restored = decodeWorkspace(workspace).document;
    expect(restored.version).toBe(3);
    expect(restored.sessions[0].views[0].file).toEqual(file);
    expect(JSON.stringify(restored)).not.toContain("base_content");
  });
});
