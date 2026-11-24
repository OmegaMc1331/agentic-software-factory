import type { DockviewApi, SerializedDockview } from "dockview";
import type { ChatMessage } from "../design-system/patterns/domain";
import {
  agents,
  filesFor,
  messagesFor,
  missionFor,
  notifications,
  scenarios,
  type Scenario,
} from "../prototypes/fixtures/data";
import { addView, applyPreset, primaryPath, type Preset, type ViewName } from "./layouts/presets";

type BufferState = { text: string; anchor: number; head: number; scrollTop: number };
export type PrototypeState = ReturnType<typeof initialState>;
function initialState(scenario: Scenario) {
  return {
    scenario,
    preset: (scenario === "code" ? "Code" : scenario === "review" ? "Review" : "Focus") as Preset,
    activity: "explorer",
    messages: messagesFor(scenario),
    mission: missionFor(scenario),
    agents: agents.map((agent) => ({
      ...agent,
      status:
        scenario === "attention" && agent.id === "codex" ? ("unavailable" as const) : agent.status,
    })),
    notifications: [...notifications],
    selectedFile: primaryPath,
    review: "pending" as "pending" | "accepted",
    notice: "",
    issue: "",
    ready: false,
    layoutVersion: 0,
    outputChannel: scenario === "review" ? "Tests" : "Terminal",
    outputCleared: false,
  };
}

type SavedWorkspace = {
  version: 1;
  scenario: Scenario;
  project: string;
  preset: Preset;
  layout: SerializedDockview;
  buffers: [string, BufferState][];
  draft: string;
  messages: ChatMessage[];
  review: "pending" | "accepted";
  selectedFile: string;
};

export class WorkspaceController {
  readonly project: string;
  readonly storageKey: string;
  readonly files;
  state: PrototypeState;
  api?: DockviewApi;
  draft = "";
  private listeners = new Set<() => void>();
  private bufferListeners = new Map<string, Map<string, (text: string) => void>>();
  private buffers = new Map<string, BufferState>();
  private saved?: SavedWorkspace;
  private timer?: ReturnType<typeof setTimeout>;
  private serial = 0;
  private applying = false;
  private narrow = window.innerWidth < 1100;
  readonly dirty = new Set<string>();
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  readonly getSnapshot = () => this.state;

  constructor() {
    const params = new URLSearchParams(location.search);
    const scenario =
      scenarios.find((item) => item.id === params.get("scenario"))?.id ?? "conversation";
    this.project = params.get("project") === "fieldnotes" ? "fieldnotes" : "atlas";
    this.storageKey = `asf.v2.p02.workspace.${this.project}.${scenario}`;
    this.files = filesFor(scenario);
    this.state = initialState(scenario);
    try {
      const stored = localStorage.getItem(this.storageKey);
      if (stored) {
        const value: unknown = JSON.parse(stored);
        if (!this.isSaved(value)) throw new Error("Preview state has an incompatible format.");
        this.saved = value;
        this.state = {
          ...this.state,
          preset: value.preset,
          messages: value.messages,
          review: value.review,
          selectedFile: value.selectedFile,
        };
        this.draft = value.draft;
        for (const [path, buffer] of value.buffers) this.buffers.set(path, buffer);
        this.serial = Math.max(
          0,
          ...Object.keys(value.layout.panels).map((id) => Number(id.split(":").at(-1)) || 0),
          ...value.messages.map((message) => Number(message.id.split("-").at(-1)) || 0)
        );
      }
    } catch {
      this.state.issue =
        "Saved preview layout could not be restored. A fresh layout is available; fixture files are unchanged.";
    }
  }

  private isSaved(value: unknown): value is SavedWorkspace {
    if (!value || typeof value !== "object") return false;
    const saved = value as Partial<SavedWorkspace>;
    return (
      saved.version === 1 &&
      saved.scenario === this.state.scenario &&
      saved.project === this.project &&
      ["Focus", "Code", "Agents", "Review"].includes(saved.preset ?? "") &&
      !!saved.layout?.panels &&
      Object.values(saved.layout.panels).every(
        (panel) =>
          typeof panel.contentComponent === "string" &&
          [
            "conversation",
            "explorer",
            "mission",
            "agents",
            "output",
            "review",
            "diff",
            "editor",
          ].includes(panel.contentComponent)
      ) &&
      typeof saved.draft === "string" &&
      Array.isArray(saved.messages) &&
      saved.messages.every(
        (message) => typeof message.body === "string" && message.provenance === "fixture"
      ) &&
      Array.isArray(saved.buffers) &&
      saved.buffers.every(
        (entry) =>
          Array.isArray(entry) &&
          this.files.some((file) => file.path === entry[0]) &&
          typeof entry[1]?.text === "string" &&
          Number.isSafeInteger(entry[1].anchor) &&
          entry[1].anchor >= 0 &&
          entry[1].anchor <= entry[1].text.length &&
          Number.isSafeInteger(entry[1].head) &&
          entry[1].head >= 0 &&
          entry[1].head <= entry[1].text.length &&
          Number.isFinite(entry[1].scrollTop) &&
          entry[1].scrollTop >= 0
      ) &&
      ["pending", "accepted"].includes(saved.review ?? "") &&
      this.files.some((file) => file.path === saved.selectedFile)
    );
  }

  update(patch: Partial<PrototypeState>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
    this.schedule();
  }

  attach(api: DockviewApi) {
    this.api = api;
    this.applying = true;
    try {
      if (this.saved) api.fromJSON(this.saved.layout);
      else applyPreset(api, this.state.preset, this.narrow);
    } catch {
      this.saved = undefined;
      applyPreset(api, this.state.preset, this.narrow);
      this.state = {
        ...this.state,
        issue:
          "The saved panel layout was invalid. The preset was restored; local buffers are retained.",
      };
    }
    this.applying = false;
    this.update({ ready: true });
  }

  setPreset(preset: Preset) {
    if (!this.api) return;
    this.applying = true;
    applyPreset(this.api, preset, this.narrow);
    this.applying = false;
    this.update({
      preset,
      layoutVersion: this.state.layoutVersion + 1,
      notice: `${preset} layout selected`,
    });
  }

  adaptWidth(width: number) {
    const narrow = width < 1100;
    if (narrow === this.narrow || !this.api) return;
    this.narrow = narrow;
    if (narrow) {
      this.setPreset(this.state.preset);
      this.update({
        notice: "Secondary views are grouped for this width. All views remain available in Views.",
      });
    }
  }

  layoutChanged() {
    if (!this.applying) this.schedule();
  }
  selectionChanged() {
    if (!this.applying) this.update({ layoutVersion: this.state.layoutVersion + 1 });
  }
  hasView(name: string) {
    return !!this.api?.getPanel(name);
  }

  toggleView(name: ViewName) {
    if (!this.api) return;
    const existing = this.api.getPanel(name);
    if (existing) existing.api.close();
    else {
      const reference = this.api.getPanel("conversation")?.id ?? this.api.activePanel?.id;
      addView(this.api, name, {
        reference,
        direction:
          name === "explorer"
            ? "left"
            : name === "output"
              ? "below"
              : this.narrow
                ? "within"
                : "right",
        width: name === "explorer" ? 224 : 286,
        height: 165,
      });
    }
    this.update({ layoutVersion: this.state.layoutVersion + 1 });
  }

  showView(name: ViewName) {
    if (!this.hasView(name)) this.toggleView(name);
    this.api?.getPanel(name)?.api.setActive();
  }

  openFile(path: string, split = false) {
    if (!this.api || !this.files.some((file) => file.path === path)) return;
    const open = this.api.panels.find(
      (panel) =>
        panel.params?.path === path && (panel.id === "editor" || panel.id.startsWith("file:"))
    );
    if (open && !split) open.api.setActive();
    else {
      const active = this.api.activePanel;
      const reference =
        active && (active.id === "editor" || active.id.startsWith("file:"))
          ? active.id
          : (open?.id ?? this.api.getPanel("conversation")?.id ?? active?.id);
      this.api.addPanel({
        id: `file:${path}:${++this.serial}`,
        component: "editor",
        title: path.split("/").pop(),
        params: { path },
        position: reference
          ? { referencePanel: reference, direction: split ? "right" : "within" }
          : undefined,
      });
    }
    this.update({ selectedFile: path, layoutVersion: this.state.layoutVersion + 1 });
  }

  splitActive() {
    this.openFile(this.api?.activePanel?.params?.path ?? this.state.selectedFile, true);
  }
  moveActive(direction: "left" | "right") {
    const panel = this.api?.activePanel;
    if (!panel) return;
    const group = panel.group;
    const index = group.panels.indexOf(panel);
    panel.api.moveTo({
      group,
      index: Math.max(
        0,
        Math.min(group.panels.length - 1, index + (direction === "right" ? 1 : -1))
      ),
      position: "center",
    });
    this.selectionChanged();
  }

  focusNext() {
    const groups = this.api?.groups;
    if (!groups?.length) return;
    const index = groups.findIndex((group) => group === this.api?.activeGroup);
    const next = groups[(index + 1) % groups.length];
    next.activePanel?.api.setActive();
    const target =
      next.element.querySelector<HTMLElement>('[contenteditable="true"],textarea,input') ??
      next.element.querySelector<HTMLElement>('button,[tabindex="0"]');
    target?.focus();
  }

  buffer(path: string): BufferState {
    if (!this.buffers.has(path))
      this.buffers.set(path, {
        text: this.files.find((file) => file.path === path)?.text ?? "",
        anchor: 0,
        head: 0,
        scrollTop: 0,
      });
    return this.buffers.get(path)!;
  }
  updateBuffer(path: string, text: string, origin: string) {
    const buffer = this.buffer(path);
    if (buffer.text === text) return;
    buffer.text = text;
    for (const [id, listener] of this.bufferListeners.get(path) ?? [])
      if (id !== origin) listener(text);
    if (!this.dirty.has(path)) {
      this.dirty.add(path);
      this.update({ notice: "Fixture buffer changed locally" });
    }
    this.schedule();
  }
  subscribeBuffer(path: string, id: string, listener: (text: string) => void) {
    if (!this.bufferListeners.has(path)) this.bufferListeners.set(path, new Map());
    this.bufferListeners.get(path)!.set(id, listener);
    return () => this.bufferListeners.get(path)?.delete(id);
  }

  addMessage(body: string) {
    if (!body.trim()) return;
    const id = ++this.serial;
    this.draft = "";
    this.update({
      messages: [
        ...this.state.messages,
        {
          id: `local-user-${id}`,
          role: "user",
          author: "You",
          time: "Now",
          body: body.trim(),
          provenance: "fixture",
        },
        {
          id: `local-response-${id}`,
          role: "orchestrator",
          author: "Orchestrator",
          time: "Now",
          body: "Your request is recorded in this local preview. No agent or runtime is connected.",
          provenance: "fixture",
        },
      ],
    });
  }
  notify(title: string, detail: string) {
    this.update({
      notifications: [
        {
          id: `local-${++this.serial}`,
          title,
          detail,
          severity: "info",
          createdAt: "Local preview · now",
        },
        ...this.state.notifications,
      ],
      notice: title,
    });
  }
  dismissNotification(id: string) {
    this.update({
      notifications: this.state.notifications.filter((item) => item.id !== id),
      notice: "Notification dismissed",
    });
  }

  schedule() {
    if (!this.state.ready) return;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.persist(), 200);
  }
  persist() {
    if (!this.api || !this.state.ready) return;
    try {
      this.saved = {
        version: 1,
        scenario: this.state.scenario,
        project: this.project,
        preset: this.state.preset,
        layout: this.api.toJSON(),
        buffers: [...this.buffers],
        draft: this.draft,
        messages: this.state.messages,
        review: this.state.review,
        selectedFile: this.state.selectedFile,
      };
      localStorage.setItem(this.storageKey, JSON.stringify(this.saved));
    } catch {
      if (!this.state.issue)
        this.update({
          issue:
            "Preview state could not be saved. Keep this tab open or export your local buffer before closing.",
        });
    }
  }
  resetLayout() {
    this.saved = undefined;
    this.setPreset(this.state.preset);
    this.update({ issue: "", notice: "Preset layout restored; buffers retained" });
  }
  detach() {
    clearTimeout(this.timer);
    this.persist();
    this.api = undefined;
  }
}

declare global {
  interface Window {
    __ASF_P02__?: { controller: WorkspaceController };
  }
}
