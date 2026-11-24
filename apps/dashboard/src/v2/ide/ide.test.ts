import { describe, it, expect, vi } from "vitest";
import { EditorState, Text } from "@codemirror/state";
import { LSPPlugin } from "@codemirror/lsp-client";
import { EditorView } from "@codemirror/view";
import { forceLinting, forEachDiagnostic } from "@codemirror/lint";
import { editorContributions, formatText, formatterApplies } from "./extensions";
import { LspVersions } from "./versions";
import { languageId, LanguageConnection, LanguageManager } from "./client";
import { diagnosticFits } from "./diagnostics";
import * as ideApi from "./api";
import type { DocumentManager, EditorDocument } from "../files/documents";
import type { WorkspaceModel } from "../workspace/production/controller";
import { FactoryApiError } from "../../api";
import { DebugManager, type DebugState } from "./debug";
import { sanitizeDocumentation } from "./documentation";
import type { InstalledExtension } from "./types";
const a = {
  project_id: "p_" + "1".repeat(32),
  root_id: "r_" + "1".repeat(32),
  path: "src/main.ts",
};
const b = { ...a, project_id: "p_" + "2".repeat(32) };
describe("language documentation trust", () => {
  it("preserves readable code while removing executable markup and resource attributes", () => {
    const html = sanitizeDocumentation(
      '<p onclick="bad()">Type <code>number</code></p><img src="https://outside.invalid/x" onerror="bad()"><svg><a href="javascript:bad()">bad</a></svg><script>bad()</script><a href="javascript:bad()">definition</a>'
    );
    const host = document.createElement("div");
    host.innerHTML = html;
    expect(host.querySelector("code")?.textContent).toBe("number");
    expect(host.textContent).toBe("Type numberdefinition");
    expect(host.querySelector("[onclick],[href],[src],img,svg,script")).toBeNull();
  });
  it("bounds deeply nested or oversized server documentation", () => {
    expect(
      sanitizeDocumentation("<p>".repeat(10000) + "x" + "</p>".repeat(10000)).length
    ).toBeLessThan(20000);
    expect(sanitizeDocumentation("x".repeat(100000)).length).toBe(65536);
  });
});
describe("IDE document version ownership", () => {
  it("forgets a missing server after restart without concealing transient stop failures", async () => {
    const api = vi
      .spyOn(ideApi, "ideRequest")
      .mockRejectedValueOnce(new FactoryApiError("temporary failure", "http", { status: 503 }))
      .mockRejectedValue(new FactoryApiError("gone", "http", { status: 404 }));
    const model = { documents: { subscribe: () => () => {} } } as unknown as WorkspaceModel;
    const manager = new LanguageManager(model),
      dispose = vi.fn();
    const connection = {
      projectId: a.project_id,
      info: { id: "gone" },
      dispose,
    } as unknown as LanguageConnection;
    manager.connections.set(`${a.project_id}:gone`, connection);
    try {
      await expect(manager.stop(connection)).rejects.toThrow("temporary failure");
      expect(manager.connections.size).toBe(1);
      await manager.stop(connection);
      expect(manager.connections.size).toBe(0);
      expect(dispose).toHaveBeenCalledOnce();
    } finally {
      manager.dispose();
      api.mockRestore();
    }
  });
  it("clears official client mappings in every pane after synchronizing a shared buffer", async () => {
    const uri = "file:///workspace/main.ts";
    const api = vi
      .spyOn(ideApi, "ideRequest")
      .mockImplementation(async (_project, route) =>
        route.includes("events") ? { cursor: 0, events: [], gap: false, alive: true } : { uri }
      );
    const current = {
      id: "shared",
      file: a,
      text: Text.of(["alpha", "beta", "gamma"]),
      generation: 0,
      status: "ready",
      baseRevision: "base",
      dirty: true,
      disk: { read_only: false },
    } as EditorDocument;
    const views: EditorView[] = [];
    const documents = {
      get: () => current,
      all: () => [current],
      views: () => views,
      raw: () => current.text.toString(),
    } as unknown as DocumentManager;
    const connection = new LanguageConnection(
      {
        id: "test",
        root_id: a.root_id,
        profile: "test",
        client_id: "window",
        capabilities: { textDocumentSync: 1 },
        alive: true,
        pid: 1,
      },
      a.project_id,
      documents,
      async () => {},
      () => {}
    );
    try {
      const extension = await connection.attach(current);
      for (let i = 0; i < 2; i++)
        views.push(
          new EditorView({
            state: EditorState.create({ doc: current.text, extensions: extension }),
          })
        );
      for (const view of views) view.dispatch({ changes: { from: 1, insert: "X" } });
      current.text = views[0].state.doc;
      current.generation++;
      connection.client.sync();
      for (const view of views) {
        expect(LSPPlugin.get(view)?.syncedDoc.toString()).toBe("aXlpha\nbeta\ngamma");
        expect(LSPPlugin.get(view)?.unsyncedChanges.empty).toBe(true);
      }
      for (const view of views)
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "short" } });
      current.text = views[0].state.doc;
      current.generation++;
      connection.client.sync();
      for (const view of views) {
        expect(LSPPlugin.get(view)?.syncedDoc.toString()).toBe("short");
        expect(LSPPlugin.get(view)?.unsyncedChanges.empty).toBe(true);
      }
    } finally {
      connection.dispose();
      for (const view of views) view.destroy();
      api.mockRestore();
    }
  });
  it("rejects stale out-of-range diagnostics and split Unicode surrogate positions", () => {
    const text = Text.of(["😀x", "second"]);
    const diagnostic = (line: number, character: number) => ({
      message: "test",
      range: { start: { line, character }, end: { line, character } },
    });
    expect(diagnosticFits(text, diagnostic(0, 2))).toBe(true);
    for (const [line, character] of [
      [0, 1],
      [3, 0],
      [-1, 0],
      [0, 4],
      [0, -1],
    ])
      expect(diagnosticFits(text, diagnostic(line, character))).toBe(false);
    expect(diagnosticFits(text, {})).toBe(false);
  });
  it("restores monotonic LSP versions independently of editor generation", () => {
    const versions = new LspVersions([{ file: a, version: 91 }]);
    const doc = { id: "doc-a", file: a, generation: 0 };
    expect(versions.version(doc)).toBe(92);
    expect(versions.version(doc)).toBe(92);
    expect(versions.version({ ...doc, generation: 15 })).toBe(93);
  });
  it("does not reuse versions from another project or worktree", () => {
    const versions = new LspVersions([{ file: a, version: 91 }]);
    expect(versions.version({ id: "b", file: b, generation: 0 })).toBe(1);
    expect(
      versions.version({ id: "c", file: { ...a, root_id: "r_" + "3".repeat(32) }, generation: 0 })
    ).toBe(1);
  });
  it.each([
    ["main.rs", "rust"],
    ["main.ts", "typescript"],
    ["view.tsx", "typescriptreact"],
    ["main.py", "python"],
    ["thing.unknown", "plaintext"],
  ])("detects %s without changing file scope", (path, expected) =>
    expect(languageId(path)).toBe(expected)
  );
});
describe("declarative extension runtime", () => {
  it("formats actual buffers while preserving Unicode and CRLF", () => {
    expect(formatText("a 😀  \r\nβ\t\r\n", "trim-trailing-whitespace")).toBe("a 😀\r\nβ\r\n");
    expect(formatText("text", "final-newline")).toBe("text\n");
    expect(formatText("text\n", "final-newline")).toBe("text\n");
    expect(() => formatText("text", "eval-js")).toThrow();
  });
  it("matches configured language identities", () => {
    expect(formatterApplies(["python"], "main.py")).toBe(true);
    expect(formatterApplies(["rust"], "main.py")).toBe(false);
  });
  const extension: InstalledExtension = {
    manifest: {
      id: "asf.test",
      name: "Test",
      version: "1.0.0",
      publisher: "asf",
      description: "Test",
      entrypoint: "contributions",
      extension_api_version: 1,
      capabilities: ["editor", "commands"],
      permissions: ["editor", "commands"],
      supported_platforms: ["any"],
      contributions: {
        diagnostics: [
          { id: "fixme", needle: "FIXME", message: "Real marker", severity: "warning" },
        ],
        commands: [
          {
            id: "command",
            title: "Command",
            action: "format",
            target: "trim",
            keybinding: "Alt-Shift-w",
          },
        ],
      },
    },
    sha256: "hash",
    provenance: "test",
    installed_at: "",
    enabled: true,
    status: "active",
    error: null,
  };
  it("activates a genuine CodeMirror lint contribution and deactivates it", async () => {
    const contribution = await editorContributions([extension], "main.ts", vi.fn());
    const parent = document.createElement("div");
    document.body.append(parent);
    const view = new EditorView({
      parent,
      state: EditorState.create({ doc: "// FIXME", extensions: contribution }),
    });
    try {
      forceLinting(view);
      await new Promise((resolve) => setTimeout(resolve, 10));
      const diagnostics: string[] = [];
      forEachDiagnostic(view.state, (diagnostic) => diagnostics.push(diagnostic.message));
      expect(diagnostics).toEqual(["Real marker"]);
      expect(
        await editorContributions([{ ...extension, enabled: false }], "main.ts", vi.fn())
      ).toEqual([]);
    } finally {
      view.destroy();
      parent.remove();
    }
  });
  it("binds declared commands to real editor key handling", async () => {
    const command = vi.fn();
    const contribution = await editorContributions([extension], "main.ts", command);
    expect(contribution.length).toBeGreaterThan(0);
  });
});
describe("debugger UI identity", () => {
  it("keeps execution locations scoped and rejects late frame inspection", async () => {
    const debug = new DebugManager();
    const state = (id: string): DebugState => ({
      info: {
        id,
        root_id: a.root_id,
        profile_id: "test",
        configuration_id: "test",
        capabilities: {},
        alive: true,
        pid: 1,
      },
      cursor: 0,
      status: "Stopped",
      threads: [],
      frames: [],
      variables: [],
      output: "",
      error: "",
      stopped: false,
      inspection: 0,
    });
    const first = state("one"),
      second = state("two");
    debug.sessions.set(`${a.project_id}:one`, first);
    debug.sessions.set(`${b.project_id}:two`, second);
    let release!: (value: unknown) => void;
    const pending = new Promise((resolve) => {
      release = resolve;
    });
    const request = vi
      .spyOn(debug, "command")
      .mockImplementation(async (_project, _id, command, args) =>
        command === "scopes" && args?.frameId === 1 ? pending : { scopes: [] }
      );
    try {
      const stale = debug.selectFrame(a.project_id, first, {
        id: 1,
        name: "old",
        line: 1,
        column: 1,
        asfFile: a,
      });
      await debug.selectFrame(a.project_id, first, {
        id: 2,
        name: "new",
        line: 3,
        column: 1,
        asfFile: a,
      });
      await debug.selectFrame(b.project_id, second, {
        id: 3,
        name: "other",
        line: 2,
        column: 1,
        asfFile: b,
      });
      release({ scopes: [{ variablesReference: 999 }] });
      await stale;
      expect(first.activeFrameId).toBe(2);
      expect(debug.executionLine(a)).toBe(3);
      expect(debug.executionLine(b)).toBe(2);
      expect(request.mock.calls.some((call) => call[2] === "variables")).toBe(false);
    } finally {
      debug.dispose();
    }
  });
  it("keeps equal path and line breakpoints separate across projects", () => {
    localStorage.removeItem("asf.v2.breakpoints.1");
    const debug = new DebugManager();
    try {
      debug.toggle(a, 2);
      debug.toggle(b, 2);
      expect(debug.breakpoints.length).toBe(2);
      debug.toggle(a, 2);
      expect(debug.breakpoints[0].file.project_id).toBe(b.project_id);
    } finally {
      debug.dispose();
      localStorage.removeItem("asf.v2.breakpoints.1");
    }
  });
  it("rejects malformed persisted breakpoint state safely", () => {
    localStorage.setItem("asf.v2.breakpoints.1", "{broken");
    const debug = new DebugManager();
    expect(debug.breakpoints).toEqual([]);
    debug.dispose();
    localStorage.removeItem("asf.v2.breakpoints.1");
  });
});
