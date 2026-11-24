import React, { useEffect, useRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { createDockview, themeDark, type DockviewApi, type IContentRenderer, type GroupPanelPartInitParameters } from "dockview";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import "dockview/dist/styles/dockview.css";
import "./style.css";
import { adapter, scopeProbe } from "@candidate";
import { api, authenticate, connect, waitFor, type Connection, type Document, type EditorHandle, type Project, type DiffHandle } from "./bridge";
import { breakpointSession, type DebugSession } from "./debug";

class Workbench {
  project!: Project;
  projects: Project[] = [];
  dock!: DockviewApi;
  documents = new Map<string, Document>();
  handles = new Map<string, EditorHandle>();
  listeners = new Set<() => void>();
  tree: { path: string; directory: boolean; bytes: number }[] = [];
  error = "";
  status = "Opening real project";
  ready = false;
  timer: number | undefined;
  lsp?: Connection;
  lspReadyMs = 0;
  debug?: DebugSession;
  debugEvidence: unknown;
  terminal?: { id: number; socket: WebSocket };
  terminalOutput = "";
  diffViews = new Map<string, DiffHandle>();
  changed() { for (const listener of this.listeners) listener(); }
  fail(error: unknown) { this.error = error instanceof Error ? error.message : String(error); this.status = "Operation failed"; this.changed(); }
  path(suffix: string) { return `/projects/${this.project.id}${suffix}`; }
  get activePath() { return this.dock.activePanel?.params?.path as string | undefined; }
  current() { const path = this.activePath; if (!path) throw new Error("Select an editor tab"); return this.documents.get(path)!; }
  snapshot() {
    for (const [path, handle] of this.handles) { const doc = this.documents.get(path); if (doc) doc.view = handle.state(); }
    return { version: 1, candidate: __CANDIDATE__, project: this.project.id, documents: [...this.documents.values()], layout: this.dock.toJSON() };
  }
  async persist() { await api(this.path(`/state/${__CANDIDATE__}`), "PUT", this.snapshot()); }
  schedule() { if (!this.ready) return; clearTimeout(this.timer); this.timer = window.setTimeout(() => this.persist().catch(error => this.fail(error)), 180); }
  async open(path: string, split = false) {
    let doc = this.documents.get(path);
    if (!doc) { doc = await api(this.path(`/file?path=${encodeURIComponent(path)}`)); this.documents.set(path, doc!); }
    const id = `file:${path}`;
    const existing = this.dock.getPanel(id);
    if (existing) { existing.api.setActive(); return; }
    const reference = this.dock.activePanel?.id.startsWith("file:") ? this.dock.activePanel : this.dock.getPanel("conversation");
    this.dock.addPanel({ id, component: "editor", title: path.split("/").pop()!, params: { path }, position: reference ? { referencePanel: reference, direction: split ? "right" : "within" } : undefined });
    this.schedule(); this.changed();
    await waitFor(() => this.handles.get(path));
  }
  async save() {
    const doc = this.current();
    const saved = await api(this.path("/file"), "PUT", doc);
    doc.revision = saved.revision;
    this.status = `Saved ${doc.path}`;
    this.error = "";
    this.changed(); await this.persist();
    return saved;
  }
  async showDiff() {
    const doc = this.current();
    const disk = await api(this.path(`/file?path=${encodeURIComponent(doc.path)}`));
    const id = `diff:${doc.path}`;
    this.dock.getPanel(id)?.api.close();
    this.dock.addPanel({ id, component: "diff", title: `Diff ${doc.path}`, params: { original: disk.content, modified: doc.content }, position: { referencePanel: this.dock.activePanel!, direction: "below" } });
    await waitFor(() => this.diffViews.get(id));
    return id;
  }
  async startLsp() {
    const start = performance.now();
    this.lsp = await connect(this.project, "typescript");
    await adapter.lsp(this.project, this.lsp);
    this.lspReadyMs = performance.now() - start;
    this.status = `Real TypeScript LSP connected (PID ${this.lsp.pid})`; this.changed();
    return { id: this.lsp.id, pid: this.lsp.pid, readyMs: this.lspReadyMs };
  }
  async stopLsp() { if (this.lsp) { await api(this.path(`/transport/${this.lsp.id}`), "DELETE"); this.lsp.socket.close(); } }
  async startDebug() { const result = await breakpointSession(this.project); this.debug = result.session; this.debugEvidence = result.evidence; this.status = "DAP stopped at real breakpoint"; this.changed(); return result.evidence; }
  async continueDebug() {
    const stopped = this.debug?.events.find(e => e.event === "stopped");
    await this.debug?.request("continue", { threadId: stopped.body.threadId });
    await waitFor(() => this.debug?.events.find(e => e.event === "terminated"));
    const events = this.debug?.events;
    await this.debug?.stop(); this.status = "DAP terminated"; this.changed(); return events;
  }
  async startTerminal() {
    const session = await api(this.path("/terminal"), "POST", {});
    await this.connectTerminal(session.id);
    return session;
  }
  async connectTerminal(id: number) {
    this.terminal?.socket.close(); this.terminalOutput = "";
    const socket = new WebSocket(`${location.origin.replace("http", "ws")}/api${this.path(`/terminal/${id}`)}`);
    socket.onmessage = event => { const value = JSON.parse(event.data); if (value.type === "output") { this.terminalOutput += value.data; window.dispatchEvent(new CustomEvent("p01-terminal", { detail: value.data })); } };
    await new Promise<void>((resolve, reject) => { socket.onopen = () => resolve(); socket.onerror = () => reject(new Error("terminal connection failed")); });
    this.terminal = { id, socket }; this.changed();
  }
  terminalInput(value: string) { this.terminal?.socket.send(JSON.stringify({ type: "input", data: value })); }
  terminalResize(cols: number, rows: number) { this.terminal?.socket.send(JSON.stringify({ type: "resize", cols, rows })); }
  async stopTerminal() { if (this.terminal) { const result = await api(this.path(`/terminal/${this.terminal.id}`), "DELETE"); this.terminal.socket.close(); return result; } }
  async plugin(action: string) {
    const start = performance.now();
    const result = await api(this.path(`/extensions/${__CANDIDATE__}`), "POST", { id: "yaml", action });
    await adapter.plugin(result.active);
    this.status = `YAML package ${action}`; this.changed();
    return { ...result, elapsedMs: performance.now() - start, evidence: adapter.pluginEvidence() };
  }
  async initialize(element: HTMLElement) {
    await authenticate();
    this.projects = await api("/projects");
    this.project = this.projects.find(p => p.id === new URLSearchParams(location.search).get("project")) ?? this.projects.find(p => p.id === "small")!;
    this.tree = await api(this.path("/tree"));
    const stored = await api(this.path(`/state/${__CANDIDATE__}`));
    for (const doc of stored?.documents ?? []) this.documents.set(doc.path, doc);
    this.dock = createDockview(element, { theme: themeDark, createComponent: options => new Panel(this, options.id, options.name) });
    const resize = new ResizeObserver(() => this.dock.layout(element.clientWidth, element.clientHeight)); resize.observe(element);
    if (stored?.layout) this.dock.fromJSON(stored.layout);
    else {
      this.dock.addPanel({ id: "conversation", component: "conversation", title: "Orchestrator — composition probe" });
      this.dock.addPanel({ id: "explorer", component: "explorer", title: "Files", position: { referencePanel: "conversation", direction: "left" }, initialWidth: 240 });
      this.dock.addPanel({ id: "terminal", component: "terminal", title: "Factory-owned PTY probe", position: { referencePanel: "conversation", direction: "below" }, initialHeight: 170 });
      this.dock.addPanel({ id: "agents", component: "agents", title: "Process evidence", position: { referencePanel: "conversation", direction: "right" }, initialWidth: 220 });
    }
    this.dock.onDidLayoutChange(() => this.schedule());
    window.addEventListener("p01-error", event => this.fail((event as CustomEvent).detail));
    this.ready = true; this.status = `Real Git project ${this.project.id}, ${this.tree.filter(e => !e.directory).length} files`; this.changed();
    const extension = await api(this.path(`/extensions/${__CANDIDATE__}`));
    if (extension.status?.active) await adapter.plugin(true);
  }
}

function useWorkbench(workbench: Workbench) { const [, set] = useState(0); useEffect(() => { const update = () => set(value => value + 1); workbench.listeners.add(update); return () => { workbench.listeners.delete(update); }; }, [workbench]); }
function EditorPanel({ workbench, path }: { workbench: Workbench; path: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const doc = workbench.documents.get(path)!;
    const handle = adapter.mount(ref.current!, doc, value => { doc.content = value; workbench.schedule(); });
    workbench.handles.set(path, handle); if (doc.view) handle.restore(doc.view);
    return () => { doc.view = handle.state(); workbench.handles.delete(path); handle.destroy(); };
  }, [path, workbench]);
  return <div className="editor-host" ref={ref} />;
}
function TerminalPanel({ workbench }: { workbench: Workbench }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const terminal = new Terminal({ cols: 90, rows: 12, fontSize: 12, theme: { background: "#12151a" } }); terminal.open(ref.current!);
    const write = (event: Event) => terminal.write((event as CustomEvent).detail);
    window.addEventListener("p01-terminal", write); terminal.write(workbench.terminalOutput); terminal.onData(value => workbench.terminalInput(value));
    return () => { window.removeEventListener("p01-terminal", write); terminal.dispose(); };
  }, [workbench]);
  return <div className="terminal-host" ref={ref} />;
}
function Content({ workbench, name, params }: { workbench: Workbench; name: string; params: any }) {
  useWorkbench(workbench);
  if (name === "editor") return <EditorPanel workbench={workbench} path={params.path} />;
  if (name === "explorer") return <div className="file-tree" aria-label="Project file tree">{workbench.tree.map(entry => <button key={entry.path} title={entry.path} disabled={entry.directory} onClick={() => workbench.open(entry.path).catch(e => workbench.fail(e))}>{entry.directory ? "▸ " : "  "}{entry.path}</button>)}</div>;
  if (name === "terminal") return <TerminalPanel workbench={workbench} />;
  if (name === "agents") return <div className="panel-copy"><p>P01 composition test. Agent workflows are not invoked here.</p><p>{workbench.status}</p><pre>{JSON.stringify(workbench.debugEvidence, null, 2)}</pre></div>;
  return <div className="panel-copy"><h1>ASF orchestrator region</h1><p>Technical composition prototype. This panel has no AI connection.</p><textarea aria-label="Orchestrator composition input" placeholder="Conversation surface — P07 remains unimplemented" /><p>Editor, explorer, protocol evidence and terminal are real dockable views.</p></div>;
}

class Panel implements IContentRenderer {
  element = document.createElement("div"); root?: Root; cleanup?: () => void;
  constructor(private workbench: Workbench, private id: string, private name: string) { this.element.className = "panel-content"; }
  init(parameters: GroupPanelPartInitParameters) {
    if (this.name === "diff") {
      const diff = adapter.diff(this.element, parameters.params.original, parameters.params.modified);
      this.workbench.diffViews.set(this.id, diff); this.cleanup = () => { this.workbench.diffViews.delete(this.id); diff.destroy(); };
    } else { this.root = createRoot(this.element); this.root.render(<Content workbench={this.workbench} name={this.name} params={parameters.params} />); }
  }
  dispose() { this.cleanup?.(); this.root?.unmount(); }
}

const workbench = new Workbench();
window.p01 = { workbench, adapter, scopeProbe };
function App() {
  const host = useRef<HTMLDivElement>(null); useWorkbench(workbench);
  useEffect(() => { workbench.initialize(host.current!).catch(error => workbench.fail(error)); }, []);
  const action = (run: () => Promise<unknown>) => run().catch(error => workbench.fail(error));
  return <main><header><strong>ASF P01 · {__CANDIDATE__}</strong><select aria-label="Project" value={workbench.project?.id ?? "small"} onChange={event => location.search = `?project=${event.target.value}`}>{workbench.projects.map(project => <option key={project.id}>{project.id}</option>)}</select>
    <button onClick={() => action(() => workbench.save())}>Save</button><button onClick={() => action(() => workbench.open("src/lib.ts", true))}>Split file</button><button onClick={() => action(() => workbench.showDiff())}>Diff</button>
    <button onClick={() => action(() => workbench.startLsp())}>Connect LSP</button><button onClick={() => action(() => workbench.startDebug())}>Debug breakpoint</button><button onClick={() => action(() => workbench.continueDebug())}>Continue</button>
    <button onClick={() => action(() => workbench.startTerminal())}>Start PTY</button><button onClick={() => action(() => workbench.stopTerminal())}>Stop PTY</button>
    <button onClick={() => action(() => workbench.plugin("install"))}>Install YAML</button><button onClick={() => action(() => workbench.plugin("activate"))}>Activate YAML</button><button onClick={() => action(() => workbench.plugin("uninstall"))}>Uninstall YAML</button><button onClick={() => action(() => workbench.persist())}>Persist</button></header>
    <div className="status" role="status">{workbench.status}</div>{workbench.error && <div className="error" role="alert">{workbench.error}</div>}<div className="dock" ref={host} /></main>;
}
createRoot(document.getElementById("root")!).render(<App />);
