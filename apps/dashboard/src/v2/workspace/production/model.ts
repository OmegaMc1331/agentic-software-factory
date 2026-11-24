import type { SerializedDockview } from "dockview";
import type { ThemePreference } from "../../design-system/themes/theme";
import { validFile, type FileRef } from "../../files/types";

export const presets = ["Focus", "Code", "Agents", "Review"] as const;
export type Preset = (typeof presets)[number];
export const viewKinds = [
  "home",
  "workflows",
  "workflow",
  "progress",
  "agents",
  "memory",
  "console",
  "terminal",
  "search",
  "sourceControl",
  "review",
  "settings",
  "notifications",
  "explorer",
  "code",
  "scratch",
  "diagnostics",
  "tests",
  "editor",
  "diff",
  "extensions",
  "debug",
  "developmentTerminal",
  "extensionView",
] as const;
export type ViewKind = (typeof viewKinds)[number];
export const futureKinds: ViewKind[] = [];
export const titles: Record<ViewKind, string> = {
  home: "Orchestrator",
  workflows: "Workflows",
  workflow: "Workflow",
  progress: "Progress",
  agents: "Agents",
  memory: "Memory",
  console: "Agent output",
  terminal: "Agent terminal",
  search: "Search",
  sourceControl: "Source Control",
  review: "Review",
  settings: "Settings",
  notifications: "Notifications",
  explorer: "Explorer",
  code: "Code",
  scratch: "Workspace note",
  diagnostics: "Diagnostics",
  tests: "Tests",
  editor: "File editor",
  diff: "Diff",
  extensions: "Extensions",
  debug: "Debug",
  developmentTerminal: "Development terminal",
  extensionView: "Extension view",
};
export interface ViewState {
  settingsSection?: string;
  missionMode?: "chat" | "form";
  conversationId?: string;
  memoryId?: string;
  query?: string;
  cursor?: number;
  page?: number;
  draft?: string;
  anchor?: number;
  head?: number;
  scroll?: number;
  dirty?: boolean;
  rootId?: string;
  expanded?: string[];
  selectedPath?: string;
  includeHidden?: boolean;
  searchContent?: boolean;
  line?: number;
  column?: number;
  jump?: number;
}
export interface WorkspaceView {
  id: string;
  kind: ViewKind;
  projectId: string;
  sessionId: string;
  title: string;
  resourceId?: number;
  agent?: string;
  file?: FileRef;
  documentId?: string;
  comparison?: "base" | "disk" | "head";
  state: ViewState;
  hidden: boolean;
  closed: boolean;
}
export interface WorkspaceSession {
  lastActivated: number;
  id: string;
  projectId: string;
  title: string;
  views: WorkspaceView[];
  layout: SerializedDockview | null;
  activeViewId: string | null;
  preset: Preset;
  activity: ViewKind;
  createdAt: string;
  updatedAt: string;
  archived: boolean;
}
export interface WorkspaceDocument {
  version: 3;
  id: string;
  openProjects: string[];
  sessions: WorkspaceSession[];
  activeProjectId: string | null;
  activeSessionId: string | null;
  preferences: { theme: ThemePreference };
}
export interface WorkspaceEnvelope {
  revision: number;
  document: unknown;
}
export const projectIdValid = (id: unknown): id is string =>
  typeof id === "string" && /^p_[0-9a-f]{32}$/.test(id);
export const identity = (prefix: "w" | "s" | "v") =>
  `${prefix}_${crypto.randomUUID().replaceAll("-", "")}`;
const localId = (id: unknown, prefix: string): id is string =>
  typeof id === "string" && new RegExp(`^${prefix}_[0-9a-f]{32}$`).test(id);
const object = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);
const shortText = (value: unknown, max = 240): value is string =>
  typeof value === "string" && value.length <= max;
const natural = (value: unknown) =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
export function emptyWorkspace(): WorkspaceDocument {
  return {
    version: 3,
    id: identity("w"),
    openProjects: [],
    sessions: [],
    activeProjectId: null,
    activeSessionId: null,
    preferences: { theme: "system" },
  };
}
export function createSession(projectId: string, title = "Session 1"): WorkspaceSession {
  const id = identity("s"),
    now = new Date().toISOString();
  return {
    id,
    lastActivated: 0,
    projectId,
    title,
    views: [],
    layout: null,
    activeViewId: null,
    preset: "Focus",
    activity: "workflows",
    createdAt: now,
    updatedAt: now,
    archived: false,
  };
}
export function createView(
  session: WorkspaceSession,
  kind: ViewKind,
  options: Partial<
    Pick<
      WorkspaceView,
      "projectId" | "resourceId" | "agent" | "title" | "file" | "documentId" | "comparison"
    >
  > = {}
): WorkspaceView {
  return {
    id: identity("v"),
    kind,
    sessionId: session.id,
    projectId: options.projectId ?? session.projectId,
    title: (options.title ?? titles[kind]).slice(0, 240),
    resourceId: options.resourceId,
    agent: options.agent,
    file: options.file,
    documentId: options.documentId,
    comparison: options.comparison,
    state: {},
    hidden: false,
    closed: false,
  };
}
export function decodeWorkspace(value: unknown): { document: WorkspaceDocument; issues: string[] } {
  if (
    !object(value) ||
    ![1, 2, 3].includes(value.version as number) ||
    !localId(value.id, "w") ||
    !Array.isArray(value.openProjects) ||
    value.openProjects.length > 32 ||
    !value.openProjects.every(projectIdValid) ||
    !Array.isArray(value.sessions) ||
    value.sessions.length > 32 ||
    JSON.stringify(value).length > 1_048_576
  )
    throw new Error(
      "Saved workspace has an invalid or unsupported schema. Its recovery copy has been retained."
    );
  const issues: string[] = [],
    ids = new Set<string>(),
    viewIds = new Set<string>();
  const sessions = value.sessions.map((raw) => {
    if (
      !object(raw) ||
      !localId(raw.id, "s") ||
      ids.has(raw.id) ||
      !projectIdValid(raw.projectId) ||
      !shortText(raw.title) ||
      !Array.isArray(raw.views) ||
      raw.views.length > 128 ||
      !presets.includes(raw.preset as Preset) ||
      !shortText(raw.createdAt) ||
      !shortText(raw.updatedAt)
    )
      throw new Error("Saved workspace session is invalid.");
    ids.add(raw.id);
    const views = raw.views.map((item) => {
      if (
        !object(item) ||
        !localId(item.id, "v") ||
        viewIds.has(item.id) ||
        item.sessionId !== raw.id ||
        !projectIdValid(item.projectId) ||
        !shortText(item.title) ||
        (item.resourceId !== undefined && (!natural(item.resourceId) || item.resourceId === 0)) ||
        (item.agent !== undefined && !shortText(item.agent))
      )
        throw new Error("Saved view identity is invalid.");
      viewIds.add(item.id);
      if (
        item.file !== undefined &&
        (!validFile(item.file) || item.file.project_id !== item.projectId)
      )
        throw new Error("Saved editor file identity is invalid.");
      if (
        item.documentId !== undefined &&
        (typeof item.documentId !== "string" || !/^d_[a-f0-9]{32}$/.test(item.documentId))
      )
        throw new Error("Saved editor document identity is invalid.");
      let kind = item.kind as ViewKind;
      if (!viewKinds.includes(kind)) {
        kind = "home";
        issues.push(
          "An unavailable view was recovered as an empty conversation view. Other resources and drafts were retained."
        );
      }
      const state: ViewState = {};
      if (
        object(item.state) &&
        typeof item.state.memoryId === "string" &&
        /^pm_[a-f0-9]{32}$/.test(item.state.memoryId)
      )
        state.memoryId = item.state.memoryId;
      if (
        object(item.state) &&
        typeof item.state.conversationId === "string" &&
        /^c_[a-f0-9]{32}$/.test(item.state.conversationId)
      )
        state.conversationId = item.state.conversationId;
      if (object(item.state)) {
        if (shortText(item.state.settingsSection, 40))
          state.settingsSection = item.state.settingsSection;
        if (item.state.missionMode === "chat" || item.state.missionMode === "form")
          state.missionMode = item.state.missionMode;
        for (const key of ["query", "draft"] as const)
          if (shortText(item.state[key], key === "draft" ? 65536 : 512))
            state[key] = item.state[key];
        for (const key of [
          "cursor",
          "page",
          "anchor",
          "head",
          "scroll",
          "line",
          "column",
          "jump",
        ] as const)
          if (natural(item.state[key])) state[key] = item.state[key] as number;
        state.dirty = item.state.dirty === true;
        if (shortText(item.state.rootId, 64)) state.rootId = item.state.rootId;
        if (shortText(item.state.selectedPath, 2048)) state.selectedPath = item.state.selectedPath;
        if (Array.isArray(item.state.expanded))
          state.expanded = item.state.expanded
            .filter((path): path is string => shortText(path, 2048))
            .slice(0, 64);
        state.includeHidden = item.state.includeHidden === true;
        state.searchContent = item.state.searchContent === true;
      }
      return {
        id: item.id,
        kind,
        projectId: item.projectId,
        sessionId: raw.id as string,
        title: item.title,
        resourceId: item.resourceId as number | undefined,
        agent: item.agent as string | undefined,
        file: item.file as FileRef | undefined,
        documentId: item.documentId as string | undefined,
        comparison: ["base", "disk", "head"].includes(item.comparison as string)
          ? (item.comparison as "base" | "disk" | "head")
          : undefined,
        state,
        hidden: item.hidden === true,
        closed: item.closed === true,
      };
    });
    const session: WorkspaceSession = {
      lastActivated: natural(raw.lastActivated)
        ? Math.min(raw.lastActivated as number, 1_000_000_000_000_000)
        : Math.max(0, Date.parse(raw.updatedAt) || 0),
      id: raw.id,
      projectId: raw.projectId,
      title: raw.title,
      views,
      layout: null,
      activeViewId: views.some((v) => v.id === raw.activeViewId)
        ? (raw.activeViewId as string)
        : null,
      preset: raw.preset as Preset,
      activity: viewKinds.includes(raw.activity as ViewKind)
        ? (raw.activity as ViewKind)
        : "workflows",
      createdAt: raw.createdAt,
      updatedAt: raw.updatedAt,
      archived: raw.archived === true,
    };
    if (raw.layout) {
      if (validLayout(raw.layout, views)) session.layout = raw.layout;
      else
        issues.push(
          "The saved split tree was invalid. The preset will be restored with all views and drafts retained."
        );
    }
    return session;
  });
  const active =
    value.activeSessionId === null
      ? undefined
      : (sessions.find((s) => s.id === value.activeSessionId && !s.archived) ??
        sessions.find((s) => !s.archived && s.projectId === value.activeProjectId));
  const preference = object(value.preferences) ? value.preferences.theme : null;
  return {
    document: {
      version: 3,
      id: value.id,
      openProjects: [...new Set(value.openProjects as string[])],
      sessions,
      activeSessionId: active?.id ?? null,
      activeProjectId:
        active?.projectId ??
        (projectIdValid(value.activeProjectId) && value.openProjects.includes(value.activeProjectId)
          ? value.activeProjectId
          : null),
      preferences: {
        theme: preference === "dark" || preference === "light" ? preference : "system",
      },
    },
    issues,
  };
}
export function validLayout(value: unknown, views: WorkspaceView[]): value is SerializedDockview {
  if (
    !object(value) ||
    !object(value.grid) ||
    !object(value.panels) ||
    Object.keys(value.panels).length > 128 ||
    (value.floatingGroups &&
      (!Array.isArray(value.floatingGroups) || value.floatingGroups.length > 0)) ||
    (value.popoutGroups && (!Array.isArray(value.popoutGroups) || value.popoutGroups.length > 0)) ||
    value.edgeGroups
  )
    return false;
  if (
    !["HORIZONTAL", "VERTICAL"].includes(value.grid.orientation as string) ||
    ![value.grid.width, value.grid.height].every(
      (n) => typeof n === "number" && Number.isFinite(n) && n >= 0 && n <= 50000
    )
  )
    return false;
  const seen = new Set<string>(),
    groups = new Set<string>();
  const panels = value.panels;
  const walk = (node: unknown, depth: number): boolean => {
    if (
      depth > 16 ||
      !object(node) ||
      (node.size !== undefined &&
        (typeof node.size !== "number" || !Number.isFinite(node.size) || node.size < 0))
    )
      return false;
    if (node.type === "branch")
      return (
        Array.isArray(node.data) &&
        node.data.length <= 128 &&
        node.data.every((child) => walk(child, depth + 1))
      );
    if (
      node.type !== "leaf" ||
      !object(node.data) ||
      !shortText(node.data.id) ||
      !/^[a-zA-Z0-9_-]{1,80}$/.test(node.data.id) ||
      ["__proto__", "constructor", "prototype"].includes(node.data.id) ||
      groups.has(node.data.id) ||
      !Array.isArray(node.data.views) ||
      node.data.views.length > 128
    )
      return false;
    groups.add(node.data.id);
    for (const id of node.data.views) {
      if (
        typeof id !== "string" ||
        seen.has(id) ||
        !views.some((v) => v.id === id && !v.closed && !v.hidden) ||
        !object(panels[id]) ||
        panels[id].id !== id ||
        panels[id].contentComponent !== "workspace" ||
        !shortText(panels[id].title)
      )
        return false;
      seen.add(id);
    }
    return node.data.activeView === undefined || node.data.views.includes(node.data.activeView);
  };
  return (
    walk(value.grid.root, 0) &&
    Object.keys(value.panels).length === seen.size &&
    (!value.activeGroup || groups.has(value.activeGroup as string))
  );
}

export function sanitizedLayout(
  layout: SerializedDockview,
  views: WorkspaceView[]
): SerializedDockview {
  const sanitizeNode = (
    node: SerializedDockview["grid"]["root"]
  ): SerializedDockview["grid"]["root"] => {
    if (node.type === "branch")
      return {
        type: "branch",
        size: node.size,
        data: (node.data as SerializedDockview["grid"]["root"][]).map(sanitizeNode),
      };
    const data = node.data as { id: string; views: string[]; activeView?: string };
    return {
      type: "leaf",
      size: node.size,
      data: { id: data.id, views: [...data.views], activeView: data.activeView },
    };
  };
  return {
    grid: {
      width: layout.grid.width,
      height: layout.grid.height,
      orientation: layout.grid.orientation,
      root: sanitizeNode(layout.grid.root),
    },
    activeGroup: layout.activeGroup,
    panels: Object.fromEntries(
      Object.keys(layout.panels).map((id) => [
        id,
        {
          id,
          contentComponent: "workspace",
          tabComponent: "asf",
          minimumWidth: views.find((view) => view.id === id)?.kind === "explorer" ? 160 : 210,
          minimumHeight: 100,
          title: views.find((view) => view.id === id)?.title ?? "View",
          params: {},
          renderer: "always",
        },
      ])
    ),
  };
}

export const legacyUrl = (projectId: string) => `/?project=${encodeURIComponent(projectId)}`;
