import type { DockviewApi, IDockviewPanel } from "dockview";
import { request, FactoryApiError } from "../../../api";
import { DocumentManager, type EditorDocument } from "../../files/documents";
import { LanguageManager } from "../../ide/client";
import { DebugManager } from "../../ide/debug";
import { FileApi } from "../../files/fileApi";
import { fileKey, fileTitle, type FileRef } from "../../files/types";
import {
  createSession,
  createView,
  decodeWorkspace,
  emptyWorkspace,
  identity,
  validLayout,
  sanitizedLayout,
  type Preset,
  type ViewKind,
  type ViewState,
  type WorkspaceDocument,
  type WorkspaceEnvelope,
  type WorkspaceSession,
  type WorkspaceView,
} from "./model";

export const LAST_WORKSPACE = "asf.v2.workspace.last";
export const RECOVERY_WORKSPACE = "asf.v2.workspace.recovery";
export interface ShellState {
  document: WorkspaceDocument;
  revision: number;
  ready: boolean;
  issue: string;
  notice: string;
  saving: boolean;
  pending: boolean;
  conflict: boolean;
  confirmClose: string | null;
}
export class WorkspaceModel {
  defaultPreset: Preset = "Focus";
  navigateResource?: (target: {
    projectId: string;
    kind: "workflow" | "review" | "agents" | "settings" | "sourceControl";
    resourceId?: number;
  }) => Promise<void>;
  state: ShellState = {
    document: emptyWorkspace(),
    revision: 0,
    ready: false,
    issue: "",
    notice: "",
    saving: false,
    pending: false,
    conflict: false,
    confirmClose: null,
  };
  private listeners = new Set<() => void>();
  private timer?: ReturnType<typeof setTimeout>;
  private generation = 0;
  private stopped = false;
  private saving?: Promise<void>;
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  readonly snapshot = () => this.state;
  readonly documents: DocumentManager;
  readonly language: LanguageManager;
  readonly debug = new DebugManager();
  navigateFile?: (file: FileRef, line?: number, column?: number) => Promise<void>;
  constructor(private api: typeof request = request) {
    this.documents = new DocumentManager(
      {
        changed: (document, previousId, previousFile) =>
          this.documentChanged(document, previousId, previousFile),
        issue: (message) => this.setIssue(message),
      },
      new FileApi(api)
    );
    this.language = new LanguageManager(this);
  }
  private documentChanged(document: EditorDocument, previousId?: string, previousFile?: FileRef) {
    if (
      !this.state.document.sessions.some((session) =>
        session.views.some(
          (view) =>
            view.documentId === document.id || (previousId && view.documentId === previousId)
        )
      )
    )
      return;
    this.updateDocument((workspace) => ({
      ...workspace,
      sessions: workspace.sessions.map((session) => ({
        ...session,
        views: session.views.map((view) =>
          (view.documentId === document.id || (previousId && view.documentId === previousId)) &&
          view.projectId === document.file.project_id &&
          view.file &&
          (fileKey(view.file) === fileKey(document.file) ||
            (previousFile && fileKey(view.file) === fileKey(previousFile)) ||
            this.documents.get(view.documentId!, view.file)?.id === document.id)
            ? {
                ...view,
                file: document.file,
                documentId: document.id,
                title:
                  (view.kind === "diff" ? "Diff · " : "") +
                  fileTitle(document.file.path, document.rootLabel),
                state: { ...view.state, dirty: document.dirty },
              }
            : view
        ),
      })),
    }));
  }
  get session() {
    return this.state.document.sessions.find((s) => s.id === this.state.document.activeSessionId);
  }
  private publish(patch: Partial<ShellState>) {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }
  async load() {
    let id: string | null = null;
    try {
      id = localStorage.getItem(LAST_WORKSPACE);
      if (!id) {
        const items = await this.api<Array<{ id: string }>>("/workspaces");
        id = items[0]?.id ?? null;
      }
      const envelope = id
        ? await this.api<WorkspaceEnvelope>(`/workspaces/${encodeURIComponent(id)}`)
        : null;
      if (envelope) {
        const restored = decodeWorkspace(envelope.document);
        this.publish({
          document: restored.document,
          revision: envelope.revision,
          issue: restored.issues.join(" "),
        });
      }
      let recovery: WorkspaceEnvelope | null = null;
      try {
        const text = sessionStorage.getItem(RECOVERY_WORKSPACE);
        if (text) recovery = JSON.parse(text) as WorkspaceEnvelope;
      } catch {
        this.publish({
          issue: "The local recovery copy is invalid. The server workspace remains available.",
        });
      }
      if (recovery && (!id || (recovery.document as { id?: unknown })?.id === id)) {
        const restored = decodeWorkspace(recovery.document);
        this.publish({
          document: restored.document,
          pending: true,
          conflict: recovery.revision !== this.state.revision,
          issue:
            recovery.revision !== this.state.revision
              ? "A recovered local layout differs from the server revision. Save an independent copy or reload the server layout."
              : "Recovered local presentation changes. Save them when the API is available.",
        });
      }
      this.publish({ ready: true });
    } catch (error) {
      let recovered = false;
      try {
        const raw = sessionStorage.getItem(RECOVERY_WORKSPACE);
        if (raw) {
          const value = JSON.parse(raw) as WorkspaceEnvelope;
          const decoded = decodeWorkspace(value.document);
          this.publish({ document: decoded.document, revision: value.revision, pending: true });
          recovered = true;
        }
      } catch {
        recovered = false;
      }
      this.publish({
        ready: true,
        issue: `${error instanceof Error ? error.message : "Workspace restoration failed."} ${recovered ? "Local recovery data is retained." : "A safe empty workspace is available; the stored document has not been overwritten."}`,
        conflict: true,
      });
    }
  }
  updateDocument(update: (document: WorkspaceDocument) => WorkspaceDocument) {
    this.generation++;
    this.publish({ document: update(this.state.document), pending: true });
    this.recovery();
    clearTimeout(this.timer);
    if (!this.state.conflict) this.timer = setTimeout(() => void this.save(), 350);
  }
  updateSession(id: string, update: (session: WorkspaceSession) => WorkspaceSession) {
    this.updateDocument((doc) => ({
      ...doc,
      sessions: doc.sessions.map((s) =>
        s.id === id ? { ...update(s), updatedAt: new Date().toISOString() } : s
      ),
    }));
  }
  updateView(sessionId: string, id: string, patch: Partial<ViewState>) {
    this.updateSession(sessionId, (s) => ({
      ...s,
      views: s.views.map((v) => (v.id === id ? { ...v, state: { ...v.state, ...patch } } : v)),
    }));
  }
  setIssue(issue: string) {
    this.publish({ issue });
  }
  announce(notice: string) {
    this.publish({ notice });
  }
  private recovery() {
    try {
      sessionStorage.setItem(
        RECOVERY_WORKSPACE,
        JSON.stringify({ revision: this.state.revision, document: this.state.document })
      );
      localStorage.setItem(LAST_WORKSPACE, this.state.document.id);
    } catch {
      this.publish({
        issue:
          "Browser recovery storage is full or unavailable. Keep this window open until the workspace is saved to Rust.",
      });
    }
  }
  async save() {
    if (this.saving) return this.saving;
    if (this.state.conflict || !this.state.pending || this.stopped) return;
    const document = this.state.document,
      generation = this.generation;
    this.publish({ saving: true });
    this.saving = (async () => {
      try {
        const result = await this.api<WorkspaceEnvelope>(`/workspaces/${document.id}`, {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ base_revision: this.state.revision, document }),
        });
        const pending = this.generation !== generation;
        this.publish({
          revision: result.revision,
          pending,
          saving: false,
          ...(this.state.issue ===
          "Recovered local presentation changes. Save them when the API is available."
            ? { issue: "" }
            : {}),
        });
        if (pending) this.recovery();
        else sessionStorage.removeItem(RECOVERY_WORKSPACE);
        localStorage.setItem(LAST_WORKSPACE, document.id);
      } catch (error) {
        const conflict = error instanceof FactoryApiError && error.status === 409;
        this.publish({
          saving: false,
          conflict,
          pending: true,
          issue: error instanceof Error ? error.message : "Workspace save failed.",
        });
        this.recovery();
      } finally {
        this.saving = undefined;
        if (this.state.pending && !this.state.conflict && !this.state.issue && !this.stopped)
          this.timer = setTimeout(() => void this.save(), 350);
      }
    })();
    return this.saving;
  }
  async independentCopy() {
    await this.saving;
    this.publish({
      document: { ...this.state.document, id: identity("w") },
      revision: 0,
      conflict: false,
      issue: "",
      pending: true,
    });
    this.generation++;
    this.recovery();
    void this.save();
  }
  async reloadSaved() {
    clearTimeout(this.timer);
    await this.saving;
    sessionStorage.removeItem(RECOVERY_WORKSPACE);
    this.publish({ ready: false, conflict: false, issue: "", pending: false });
    await this.load();
  }
  recoverEmpty() {
    if (this.saving) return;
    clearTimeout(this.timer);
    this.publish({
      document: emptyWorkspace(),
      revision: 0,
      conflict: false,
      issue: "",
      ready: true,
      pending: false,
    });
    sessionStorage.removeItem(RECOVERY_WORKSPACE);
    localStorage.setItem(LAST_WORKSPACE, this.state.document.id);
  }
  retrySave() {
    this.publish({ issue: "" });
    void this.save();
  }
  activateProject(projectId: string) {
    if (
      !this.state.document.openProjects.includes(projectId) &&
      this.state.document.openProjects.length >= 32
    ) {
      this.setIssue("Workspace open-project limit reached (32). Use an independent workspace.");
      return;
    }
    const candidates = this.state.document.sessions
      .filter((s) => s.projectId === projectId)
      .sort((a, b) => b.lastActivated - a.lastActivated || a.id.localeCompare(b.id));
    let session = candidates.find((s) => !s.archived) ?? candidates[0];
    if (!session && this.state.document.sessions.length >= 32) {
      this.setIssue("Workspace session limit reached (32). Use an independent workspace.");
      return;
    }
    if (!session) session = { ...createSession(projectId), preset: this.defaultPreset };
    const selected = session;
    this.updateDocument((doc) => ({
      ...doc,
      openProjects: doc.openProjects.includes(projectId)
        ? doc.openProjects
        : [...doc.openProjects, projectId],
      sessions: doc.sessions.some((s) => s.id === selected.id)
        ? doc.sessions
        : [...doc.sessions, selected],
      activeProjectId: projectId,
      activeSessionId: selected.id,
    }));
    this.updateSession(selected.id, (s) => ({
      ...s,
      archived: false,
      lastActivated:
        Math.max(0, ...this.state.document.sessions.map((item) => item.lastActivated)) + 1,
    }));
  }
  hideProject(projectId: string) {
    this.updateDocument((doc) => {
      const openProjects = doc.openProjects.filter((id) => id !== projectId),
        activeProjectId =
          doc.activeProjectId === projectId ? (openProjects[0] ?? null) : doc.activeProjectId;
      const sessions = doc.sessions.map((s) =>
        s.projectId === projectId ? { ...s, archived: true } : s
      );
      return {
        ...doc,
        openProjects,
        sessions,
        activeProjectId,
        activeSessionId:
          sessions.find((s) => s.projectId === activeProjectId && !s.archived)?.id ?? null,
      };
    });
    this.announce(
      "Project tab hidden. Its runtime and workflows continue; reopening restores its navigation session."
    );
  }
  newSession() {
    const project = this.state.document.activeProjectId;
    if (!project) return;
    if (this.state.document.sessions.length >= 32) {
      this.setIssue("Workspace session limit reached (32). Use an independent workspace.");
      return;
    }
    const count = this.state.document.sessions.filter((s) => s.projectId === project).length;
    const session = {
      ...createSession(project, `Session ${count + 1}`),
      preset: this.defaultPreset,
    };
    session.lastActivated =
      Math.max(0, ...this.state.document.sessions.map((item) => item.lastActivated)) + 1;
    this.updateDocument((doc) => ({
      ...doc,
      sessions: [...doc.sessions, session],
      activeSessionId: session.id,
    }));
  }
  missionSession(projectId: string, presentationSessionId: string): string {
    const direct = this.state.document.sessions.find(
      (s) => s.id === presentationSessionId && s.projectId === projectId
    );
    if (direct) return direct.id;
    const existing = this.state.document.sessions
      .filter((s) => s.projectId === projectId)
      .sort((a, b) => b.lastActivated - a.lastActivated)[0];
    if (existing) return existing.id;
    if (this.state.document.sessions.length >= 32) {
      this.setIssue(
        "Workspace session limit reached (32). Open an owning session before submitting work."
      );
      throw new Error("An owning workspace session is required for this project's mission.");
    }
    const owner = createSession(projectId);
    this.updateDocument((doc) => ({ ...doc, sessions: [...doc.sessions, owner] }));
    return owner.id;
  }
  activateSession(id: string) {
    const session = this.state.document.sessions.find((s) => s.id === id);
    if (session)
      this.updateDocument((doc) => ({
        ...doc,
        activeSessionId: id,
        activeProjectId: session.projectId,
        sessions: doc.sessions.map((s) =>
          s.id === id
            ? {
                ...s,
                archived: false,
                updatedAt: new Date().toISOString(),
                lastActivated: Math.max(0, ...doc.sessions.map((item) => item.lastActivated)) + 1,
              }
            : s
        ),
      }));
  }
  archiveSession(id = this.session?.id) {
    const session = this.state.document.sessions.find((s) => s.id === id);
    if (!session) return;
    this.updateDocument((doc) => {
      const sessions = doc.sessions.map((s) =>
        s.id === session.id ? { ...s, archived: true } : s
      );
      const next = sessions.find((s) => s.projectId === session.projectId && !s.archived);
      return {
        ...doc,
        sessions,
        activeSessionId:
          doc.activeSessionId === session.id ? (next?.id ?? null) : doc.activeSessionId,
      };
    });
    this.announce(
      "Session hidden from navigation. Backend workflows and terminals continue; no suspension was requested."
    );
  }
  reorderSessions(id: string, direction: number) {
    this.updateDocument((doc) => {
      const moving = doc.sessions.find((s) => s.id === id);
      if (!moving) return doc;
      const peers = doc.sessions.filter((s) => s.projectId === moving.projectId),
        index = peers.findIndex((s) => s.id === id),
        target = peers[Math.max(0, Math.min(peers.length - 1, index + direction))];
      if (!target) return doc;
      const order = [...doc.sessions],
        from = order.findIndex((s) => s.id === id),
        to = order.findIndex((s) => s.id === target.id);
      order.splice(from, 1);
      order.splice(to, 0, moving);
      return { ...doc, sessions: order };
    });
  }
  reorderProjects(id: string, direction: number) {
    this.updateDocument((doc) => {
      const tabs = [...doc.openProjects],
        index = tabs.indexOf(id),
        to = Math.max(0, Math.min(tabs.length - 1, index + direction));
      tabs.splice(index, 1);
      tabs.splice(to, 0, id);
      return { ...doc, openProjects: tabs };
    });
  }
  dispose() {
    this.debug.dispose();
    this.language.dispose();
    this.documents.dispose();
    this.stopped = true;
    clearTimeout(this.timer);
    this.listeners.clear();
  }
}

export class DockSession {
  api?: DockviewApi;
  applying = false;
  constructor(
    readonly model: WorkspaceModel,
    readonly sessionId: string
  ) {}
  get session() {
    return this.model.state.document.sessions.find((s) => s.id === this.sessionId)!;
  }
  attach(api: DockviewApi) {
    this.api = api;
    this.applying = true;
    try {
      const layout = this.session.layout;
      if (layout && validLayout(layout, this.session.views)) {
        const safe = sanitizedLayout(layout, this.session.views);
        api.fromJSON(safe);
      } else this.preset(this.session.preset);
      for (const view of this.session.views)
        if (!view.closed && !view.hidden && !api.getPanel(view.id)) this.add(view);
    } catch {
      api.clear();
      this.preset(this.session.preset);
      this.model.setIssue(
        "The saved Dockview tree could not load. Views and drafts were preserved in the restored preset."
      );
    } finally {
      this.applying = false;
    }
    if (this.session.activeViewId) this.api.getPanel(this.session.activeViewId)?.api.setActive();
    this.capture();
  }
  private add(
    view: WorkspaceView,
    reference?: string,
    direction: "left" | "right" | "above" | "below" | "within" = "within",
    size?: number
  ) {
    return this.api!.addPanel({
      id: view.id,
      component: "workspace",
      title: view.title,
      renderer: "always",
      params: {},
      position: reference ? { referencePanel: reference, direction } : undefined,
      initialWidth: direction === "left" || direction === "right" ? size : undefined,
      initialHeight: direction === "below" || direction === "above" ? size : undefined,
      minimumWidth: view.kind === "explorer" ? 160 : 210,
      minimumHeight: 100,
    });
  }
  open(
    kind: ViewKind,
    options: Partial<
      Pick<
        WorkspaceView,
        "projectId" | "resourceId" | "agent" | "title" | "file" | "documentId" | "comparison"
      >
    > = {},
    direction: "within" | "right" | "below" = "within",
    duplicate = false
  ) {
    if (!this.api) return;
    const projectId = options.projectId ?? this.session.projectId;
    let view = !duplicate
      ? this.session.views.find(
          (v) =>
            v.kind === kind &&
            v.projectId === projectId &&
            v.resourceId === options.resourceId &&
            v.agent === options.agent &&
            (!options.file ? !v.file : !!v.file && fileKey(v.file) === fileKey(options.file)) &&
            v.comparison === options.comparison
        )
      : undefined;
    if (!view) {
      if (this.session.views.length >= 128) {
        this.model.setIssue(
          "This session has 128 view records. Use a new session; existing drafts are preserved."
        );
        return;
      }
      view = createView(this.session, kind, options);
      const created = view;
      this.model.updateSession(this.sessionId, (s) => ({ ...s, views: [...s.views, created] }));
    }
    this.model.updateSession(this.sessionId, (s) => ({
      ...s,
      activeViewId: view!.id,
      views: s.views.map((v) => (v.id === view!.id ? { ...v, closed: false, hidden: false } : v)),
    }));
    const existing = this.api.getPanel(view.id);
    if (existing) existing.api.setActive();
    else
      this.add(view, this.api.activePanel?.id, direction, direction === "below" ? 180 : undefined);
    this.capture();
    return view.id;
  }
  async openFile(
    file: FileRef,
    direction: "within" | "right" | "below" = "within",
    location?: { line: number; column?: number },
    duplicate = false
  ) {
    const previous = this.model.state.document.sessions
      .flatMap((session) => session.views)
      .find((view) => view.file && fileKey(view.file) === fileKey(file) && view.documentId);
    const documentId = await this.model.documents.open(file, previous?.documentId);
    const document = this.model.documents.get(documentId)!;
    const id = this.open(
      "editor",
      {
        projectId: file.project_id,
        file: document.file,
        documentId,
        title: fileTitle(document.file.path, document.rootLabel),
      },
      direction,
      duplicate
    );
    if (id && location)
      this.model.updateView(this.sessionId, id, {
        line: location.line,
        column: location.column ?? 1,
        jump: Date.now(),
      });
    this.focus();
    return id;
  }
  preset(preset: Preset) {
    if (!this.api) return;
    const required: ViewKind[] =
      preset === "Code"
        ? ["code", "explorer", "progress", "console", "home"]
        : preset === "Review"
          ? ["review", "workflows", "progress", "console", "home"]
          : ["home", "workflows", preset === "Agents" ? "agents" : "progress"];
    const missing = required.filter(
      (kind) =>
        !this.session.views.some(
          (view) => view.kind === kind && view.projectId === this.session.projectId && !view.closed
        )
    );
    if (this.session.views.length + missing.length > 128) {
      this.model.setIssue(
        "This preset exceeds the 128-view limit. Reopen its existing views or use a new session; the current layout and drafts are retained."
      );
      return;
    }
    const wasApplying = this.applying;
    this.applying = true;
    const ensure = (kind: ViewKind) => {
      let view = this.session.views.find(
        (v) => v.kind === kind && v.projectId === this.session.projectId && !v.closed
      );
      if (!view) {
        view = createView(this.session, kind);
        const next = view;
        this.model.updateSession(this.sessionId, (s) => ({ ...s, views: [...s.views, next] }));
      }
      return view;
    };
    const primary = ensure(preset === "Code" ? "code" : preset === "Review" ? "review" : "home");
    const left = ensure(preset === "Code" ? "explorer" : "workflows");
    const side = ensure(preset === "Agents" ? "agents" : "progress");
    const bottom = preset === "Code" || preset === "Review" ? ensure("console") : null;
    this.api.clear();
    this.add(primary);
    const narrow = window.innerWidth < 1100;
    this.add(left, primary.id, "left", narrow ? 190 : 232);
    this.add(side, primary.id, narrow ? "within" : "right", preset === "Agents" ? 310 : 270);
    if (bottom) this.add(bottom, primary.id, "below", 180);
    if (preset !== "Focus") {
      const home = ensure("home");
      if (!this.api.getPanel(home.id)) this.add(home, primary.id);
    }
    for (const view of this.session.views)
      if (!view.closed && !view.hidden && !this.api.getPanel(view.id)) this.add(view, primary.id);
    this.api.getPanel(primary.id)?.api.setActive();
    const visible = new Set(this.api.panels.map((p) => p.id));
    this.model.updateSession(this.sessionId, (s) => ({
      ...s,
      preset,
      views: s.views.map((v) => (visible.has(v.id) ? { ...v, hidden: false } : v)),
    }));
    this.applying = wasApplying;
    if (!this.applying) this.capture();
    this.model.announce(`${preset} layout applied. Open resources and drafts are retained.`);
  }
  capture() {
    if (this.applying || !this.api) return;
    const layout = this.api.toJSON(),
      activeViewId = this.api.activePanel?.id ?? null;
    for (const panel of Object.values(layout.panels))
      panel.title = this.session.views.find((view) => view.id === panel.id)?.title ?? "View";
    if (
      JSON.stringify(layout) === JSON.stringify(this.session.layout) &&
      activeViewId === this.session.activeViewId
    )
      return;
    this.model.updateSession(this.sessionId, (s) => ({ ...s, layout, activeViewId }));
  }
  requestClose(id = this.api?.activePanel?.id) {
    if (!id) return;
    const view = this.session.views.find((v) => v.id === id);
    if (view?.documentId && (!view.file || !this.model.documents.get(view.documentId, view.file))) {
      this.close(id);
      return;
    }
    if (view?.state.dirty) {
      this.model.state = { ...this.model.state, confirmClose: id };
      this.model.announce("This view has a retained draft. Confirm navigation-only closure.");
      return;
    }
    this.close(id);
  }
  close(id: string, hidden = false) {
    this.applying = true;
    this.model.updateSession(this.sessionId, (s) => ({
      ...s,
      views: s.views.map((v) => (v.id === id ? { ...v, hidden, closed: !hidden } : v)),
    }));
    const panel = this.api?.getPanel(id);
    if (panel) this.api?.removePanel(panel);
    this.applying = false;
    this.capture();
    this.model.state = { ...this.model.state, confirmClose: null };
    this.model.documents.releaseUnused(
      new Set(
        this.model.state.document.sessions.flatMap((session) =>
          session.views
            .filter((view) => !view.closed && view.documentId)
            .map((view) => view.documentId!)
        )
      )
    );
    this.model.announce(
      hidden
        ? "View hidden; its subscriptions and resources are retained."
        : "View closed. Backend execution was not changed; reopen from Views."
    );
  }
  move(direction: "left" | "right") {
    const panel = this.api?.activePanel;
    if (!panel) return;
    const group = panel.group,
      index = group.panels.indexOf(panel);
    panel.api.moveTo({
      group,
      index: Math.max(
        0,
        Math.min(group.panels.length - 1, index + (direction === "left" ? -1 : 1))
      ),
      position: "center",
    });
    this.capture();
  }
  moveGroup() {
    const panel = this.api?.activePanel,
      groups = this.api?.groups;
    if (!panel || !groups || groups.length < 2) return;
    const index = groups.findIndex((g) => g.id === panel.group.id);
    panel.api.moveTo({ group: groups[(index + 1) % groups.length], position: "center" });
    this.capture();
  }
  split(direction: "right" | "below") {
    const panel = this.api?.activePanel;
    const view = this.session.views.find((view) => view.id === panel?.id);
    if (view?.kind === "editor" && view.file) {
      void this.openFile(view.file, direction, undefined, true).catch((error) =>
        this.model.setIssue(String(error))
      );
      return;
    }
    if (panel)
      panel.api.moveTo({
        group: panel.group,
        position: direction === "below" ? "bottom" : "right",
      });
    this.capture();
  }
  restore(id: string) {
    const view = this.session.views.find((v) => v.id === id);
    if (!view || !this.api) return;
    this.model.updateSession(this.sessionId, (s) => ({
      ...s,
      views: s.views.map((v) => (v.id === id ? { ...v, hidden: false, closed: false } : v)),
    }));
    const panel = this.api.getPanel(id) ?? this.add(view, this.api.activePanel?.id);
    panel.api.setActive();
    this.capture();
  }
  nextTab(direction = 1) {
    const panel = this.api?.activePanel;
    if (!panel) return;
    const tabs = panel.group.panels,
      index = tabs.indexOf(panel);
    tabs[(index + direction + tabs.length) % tabs.length]?.api.setActive();
    this.capture();
    this.focus();
  }
  focusNext(direction = 1) {
    const groups = this.api?.groups ?? [],
      index = groups.findIndex((g) => g.id === this.api?.activeGroup?.id);
    const group = groups[(index + direction + groups.length) % groups.length];
    group?.activePanel?.api.setActive();
    this.focus();
  }
  focus() {
    const panel = this.api?.activePanel;
    if (panel) {
      const root = panel.view.content.element;
      const previous = document.activeElement;
      const apply = () => {
        if (this.api?.activePanel !== panel || !root.isConnected) return;
        const target =
          root.querySelector<HTMLElement>(
            '[contenteditable="true"],[role="textbox"][tabindex="0"]'
          ) ??
          root.querySelector<HTMLElement>(
            'button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),[tabindex="0"]:not(:disabled)'
          );
        (target ?? root).focus();
      };
      apply();
      requestAnimationFrame(() => {
        const active = document.activeElement;
        if (active !== previous && active !== document.body && !root.contains(active)) return;
        apply();
      });
    }
  }
  resize(width: number, height: number) {
    this.api?.activePanel?.api.setSize({
      width: Math.max(160, Math.min(window.innerWidth, width)),
      height: Math.max(100, Math.min(window.innerHeight - 160, height)),
    });
    this.capture();
  }
  removed(panel: IDockviewPanel) {
    if (this.applying) return;
    queueMicrotask(() => {
      if (!this.applying && this.api && !this.api.getPanel(panel.id)) this.close(panel.id);
    });
  }
}
